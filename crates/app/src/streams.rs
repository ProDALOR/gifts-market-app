//! Local snapshot/replay and coalesced books shared by WebSocket and gRPC.
use crate::{
    error::{ApiError, ErrorCode, Result},
    query::*,
    replication::{ReplayBatch, ReplicaSnapshot},
    status::{ApiCursor, ResponseMeta},
};
use futures_util::{Stream, StreamExt};
use market_view::protocol::wire as w;
use market_view::{self as m, MarketEvent, Record, RecordId};
use prost::Message;
use serde::{Serialize, Serializer};
use std::{pin::Pin, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

pub type Output<T> = Pin<Box<dyn Stream<Item = Result<T>> + Send>>;
const CHUNK_BYTES: usize = 256 * 1024;
#[derive(Serialize)]
pub struct SnapshotFrame {
    pub meta: ResponseMeta,
    #[serde(serialize_with = "market_json::serialize_catalog")]
    pub catalog: Vec<m::CatalogEntry>,
    #[serde(serialize_with = "market_json::serialize_records")]
    pub records: Vec<Record>,
    pub complete: bool,
}
#[derive(Serialize)]
pub struct ChangeFrame {
    #[serde(skip)]
    pub kind: &'static str,
    pub previous: ApiCursor,
    #[serde(
        serialize_with = "market_json::serialize_catalog",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub catalog: Vec<m::CatalogEntry>,
    pub catalog_revision: String,
    pub cursor: ApiCursor,
    #[serde(serialize_with = "events")]
    pub events: Vec<MarketEvent>,
    /// Apply all chunks of a revision atomically when complete is true.
    pub complete: bool,
    pub meta: ResponseMeta,
}
fn events<S: Serializer>(
    values: &[MarketEvent],
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    let values: std::result::Result<Vec<_>, _> =
        values.iter().map(market_json::event_value).collect();
    values
        .map_err(serde::ser::Error::custom)?
        .serialize(serializer)
}
pub struct BookUpdate {
    pub kind: &'static str,
    pub response: Arc<BookResponse>,
    pub json: Arc<serde_json::value::RawValue>,
}

fn validate_scope(snapshot: &ReplicaSnapshot, scope: &Scope) -> Result<()> {
    let catalog = &snapshot
        .view
        .as_ref()
        .expect("usable replica")
        .catalog
        .catalog;
    if scope
        .feed_id
        .as_ref()
        .is_some_and(|id| catalog.feed(id).is_none())
    {
        return Err(ApiError::field("feed_id", "unknown feed"));
    }
    if scope
        .item_type_id
        .as_ref()
        .is_some_and(|id| catalog.item_type(id).is_none())
    {
        return Err(ApiError::field("item_type_id", "unknown item type"));
    }
    Ok(())
}
fn feed_matches(scope: &Scope, feed: &m::FeedId) -> bool {
    scope.feed_id.as_ref().is_none_or(|id| id == feed)
}
fn type_matches(scope: &Scope, kind: Option<&m::ItemTypeId>) -> bool {
    scope
        .item_type_id
        .as_ref()
        .is_none_or(|id| Some(id) == kind)
}
fn record_matches(scope: &Scope, record: &Record) -> bool {
    feed_matches(scope, &record.key().feed_id)
        && (matches!(record, Record::Feed(_)) || type_matches(scope, record.item_type_id()))
}
pub(crate) fn project<'a>(
    scope: &'a Scope,
    batch: &'a ReplayBatch,
) -> impl Iterator<Item = MarketEvent> + 'a {
    batch
        .events
        .iter()
        .filter_map(move |entry| match &entry.event {
            MarketEvent::Upsert(record) if record_matches(scope, record) => {
                Some(entry.event.clone())
            }
            MarketEvent::Upsert(record)
                if feed_matches(scope, &record.key().feed_id)
                    && type_matches(scope, entry.previous_item_type.as_ref()) =>
            {
                Some(MarketEvent::Delete(record.key()))
            }
            MarketEvent::Delete(key)
                if feed_matches(scope, &key.feed_id)
                    && (matches!(key.id, RecordId::Feed(_))
                        || type_matches(scope, entry.previous_item_type.as_ref())) =>
            {
                Some(entry.event.clone())
            }
            MarketEvent::OrderRemoved(v)
                if feed_matches(scope, &v.feed_id)
                    && type_matches(scope, Some(&v.item_type_id)) =>
            {
                Some(MarketEvent::Delete(v.data.key(v.feed_id.clone())))
            }
            _ => None,
        })
}
impl ApiState {
    pub fn snapshot_stream(&self, scope: Scope) -> Result<(m::Cursor, Output<SnapshotFrame>)> {
        let permit = self.stream_slot()?;
        let snapshot = self.current()?;
        validate_scope(&snapshot, &scope)?;
        let view = snapshot.view.as_ref().expect("usable replica").clone();
        let slot = self
            .snapshot_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| {
                ApiError::new(ErrorCode::CapacityExceeded, "snapshot capacity exhausted")
            })?;
        let weight = view.record_bytes().max(1).try_into().map_err(|_| {
            ApiError::new(ErrorCode::CapacityExceeded, "snapshot exceeds byte budget")
        })?;
        let bytes = self
            .snapshot_bytes
            .clone()
            .try_acquire_many_owned(weight)
            .map_err(|_| {
                ApiError::new(
                    ErrorCode::CapacityExceeded,
                    "snapshot byte budget exhausted",
                )
            })?;
        let cursor = view.cursor.clone();
        let output = async_stream::try_stream! {
            let (_slot,_bytes) = (slot,bytes);
            let mut catalog = Vec::new(); let mut records = Vec::new(); let mut size = 0;
            let as_of = m::MarketTimestamp::now();
            for entry in view.catalog.entries.iter() {
                let len = w::CatalogEntry::from(entry.clone()).encoded_len();
                if size + len > CHUNK_BYTES && !catalog.is_empty() {
                    yield SnapshotFrame {meta:ResponseMeta::new(&snapshot,as_of),catalog:std::mem::take(&mut catalog),records:Vec::new(),complete:false}; size=0;
                }
                size += len; catalog.push(entry.clone());
            }
            if !catalog.is_empty() {yield SnapshotFrame {meta:ResponseMeta::new(&snapshot,as_of),catalog,records:Vec::new(),complete:false};}
            size=0;
            for (_,record) in view.state.after(None) {
                if !record_matches(&scope,record) {continue;}
                let len = w::Record::from(record.clone()).encoded_len();
                if size + len > CHUNK_BYTES && !records.is_empty() {
                    yield SnapshotFrame {meta:ResponseMeta::new(&snapshot,as_of),catalog:Vec::new(),records:std::mem::take(&mut records),complete:false}; size=0;
                }
                size+=len; records.push(record.clone());
            }
            yield SnapshotFrame {meta:ResponseMeta::new(&snapshot,as_of),catalog:Vec::new(),records,complete:true};
        };
        // A disconnected slow reader cannot pin a view indefinitely.
        let mut output = deadline(Box::pin(output), Duration::from_secs(60));
        let guarded = async_stream::try_stream! {
            let _permit=permit;
            while let Some(value)=output.next().await {yield value?;}
        };
        Ok((cursor, Box::pin(guarded)))
    }
    pub fn changes_stream(
        &self,
        after: m::Cursor,
        scope: Scope,
        stop: CancellationToken,
    ) -> Result<Output<ChangeFrame>> {
        let permit = self.stream_slot()?;
        let mut changed = self.replica.updates(); // Subscribe before validating/capturing head.
        let snapshot = self.current()?;
        validate_scope(&snapshot, &scope)?;
        self.replica.next_batch(&after).map_err(ApiError::from)?;
        let head = snapshot
            .view
            .as_ref()
            .expect("usable replica")
            .cursor
            .clone();
        let state = self.clone();
        Ok(Box::pin(async_stream::try_stream! {
            let _permit=permit; let mut cursor=after; let mut checkpoint=false;let mut revision=snapshot.view.as_ref().expect("usable view").catalog.revision.clone();
            loop {
                state.current()?;
                let next = state.replica.next_batch(&cursor).map_err(ApiError::from)?;
                if !checkpoint && cursor.revision >= head.revision {
                    checkpoint=true;
                    yield change_marker("checkpoint",&cursor,&state.replica.snapshot(),&revision);
                }
                if let Some(batch)=next {
                    let mut events=Vec::new(); let mut size=0;
                    let mut catalog=Vec::new();
                    for entry in &batch.catalog {
                        let len=w::CatalogEntry::from(entry.clone()).encoded_len();
                        if size+len>CHUNK_BYTES && !catalog.is_empty() {yield ChangeFrame {kind:"changes",previous:batch.previous.clone().into(),cursor:batch.cursor.clone().into(),events:Vec::new(),catalog:std::mem::take(&mut catalog),catalog_revision:batch.catalog_revision.clone(),complete:false,meta:ResponseMeta::new(&state.replica.snapshot(),m::MarketTimestamp::now())};size=0;}
                        size+=len;catalog.push(entry.clone());
                    }
                    if !catalog.is_empty() {yield ChangeFrame {kind:"changes",previous:batch.previous.clone().into(),cursor:batch.cursor.clone().into(),events:Vec::new(),catalog,catalog_revision:batch.catalog_revision.clone(),complete:false,meta:ResponseMeta::new(&state.replica.snapshot(),m::MarketTimestamp::now())};}
                    size=0;revision=batch.catalog_revision.clone();
                    for event in project(&scope,&batch) {
                        let len = w::MarketEvent::from(event.clone()).encoded_len();
                        if size+len > CHUNK_BYTES && !events.is_empty() {
                            yield ChangeFrame {kind:"changes",previous:batch.previous.clone().into(),cursor:batch.cursor.clone().into(),events:std::mem::take(&mut events),catalog:Vec::new(),catalog_revision:revision.clone(),complete:false,meta:ResponseMeta::new(&state.replica.snapshot(),m::MarketTimestamp::now())}; size=0;
                        }
                        size+=len; events.push(event);
                    }
                    cursor=batch.cursor.clone();
                    yield ChangeFrame {kind:"changes",previous:batch.previous.clone().into(),cursor:cursor.clone().into(),events,catalog:Vec::new(),catalog_revision:revision.clone(),complete:true,meta:ResponseMeta::new(&state.replica.snapshot(),m::MarketTimestamp::now())};
                    continue;
                }
                let signal=tokio::select! {
                    _=stop.cancelled()=>break,
                    result=changed.changed()=>Some(result),
                    _=tokio::time::sleep(Duration::from_secs(15))=>None,
                };
                if let Some(result)=signal {result.map_err(|_|ApiError::unavailable())?;} else {yield change_marker("heartbeat",&cursor,&state.replica.snapshot(),&revision);}
            }
        }))
    }
    pub fn book_stream(
        &self,
        query: BookQuery,
        stop: CancellationToken,
    ) -> Result<Output<BookUpdate>> {
        let permit = self.stream_slot()?;
        let mut changed = self.replica.updates();
        let first = self.cached_book(&query)?;
        let state = self.clone();
        Ok(Box::pin(async_stream::try_stream! {
            let _permit=permit;
            let mut signature=book_signature(&first.0); let mut sent=tokio::time::Instant::now();
            yield BookUpdate {kind:"snapshot",response:first.0,json:first.1};
            let mut timer=tokio::time::interval(Duration::from_secs(1)); timer.tick().await;
            loop {
                let signal=tokio::select! { _=stop.cancelled()=>break, _=timer.tick()=>None, result=changed.changed()=>Some(result) };
                if let Some(result)=signal {result.map_err(|_|ApiError::unavailable())?;}
                let (response,json)=state.cached_book(&query)?;
                let next=book_signature(&response);
                // Full bounded replacements can coalesce; replay streams never do.
                if next != signature || sent.elapsed() >= Duration::from_secs(30) {
                    signature=next; sent=tokio::time::Instant::now();
                    yield BookUpdate {kind:"replace",response,json};
                }
            }
        }))
    }
}
fn book_signature(response: &BookResponse) -> String {
    fingerprint(&(
        &response.data.book,
        &response.data.asks.data,
        &response.data.bids.data,
        &response.included,
        &response.meta.replica.state,
        &response.meta.catalog_revision,
        response.meta.cursor.as_ref().map(|v| &v.generation),
    ))
}
fn change_marker(
    kind: &'static str,
    cursor: &m::Cursor,
    snapshot: &ReplicaSnapshot,
    revision: &str,
) -> ChangeFrame {
    ChangeFrame {
        kind,
        previous: cursor.clone().into(),
        cursor: cursor.clone().into(),
        events: Vec::new(),
        catalog: Vec::new(),
        catalog_revision: revision.into(),
        complete: true,
        meta: ResponseMeta::new(snapshot, m::MarketTimestamp::now()),
    }
}
fn deadline<T: Send + 'static>(mut source: Output<T>, duration: Duration) -> Output<T> {
    let (tx, mut rx) = tokio::sync::mpsc::channel(2);
    let (done, result) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let work = async {
            while let Some(value) = source.next().await {
                let value = value?;
                if tx.send(value).await.is_err() {
                    break;
                }
            }
            Ok(())
        };
        let outcome = match tokio::time::timeout(duration, work).await {
            Ok(value) => value,
            Err(_) => Err(ApiError::new(
                ErrorCode::CursorExpired,
                "snapshot stream deadline exceeded",
            )),
        };
        drop(source);
        drop(tx);
        let _ = done.send(outcome);
    });
    Box::pin(async_stream::try_stream! {
        while let Some(value)=rx.recv().await {yield value;}
        result.await.map_err(|_|ApiError::unavailable())??;
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn abandoned_snapshot_producer_drops_pinned_state_after_deadline() {
        let pin = Arc::new(());
        let weak = Arc::downgrade(&pin);
        let source: Output<usize> = Box::pin(async_stream::try_stream! {
            let _pin=pin;
            for n in 0..100 {yield n;}
        });
        let mut output = deadline(source, Duration::from_millis(20));
        // Do not poll the consumer while the producer fills its bounded queue.
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(weak.upgrade().is_none());
        assert_eq!(output.next().await.unwrap().unwrap(), 0);
        assert_eq!(output.next().await.unwrap().unwrap(), 1);
        assert!(matches!(
            output.next().await.unwrap().unwrap_err().code,
            ErrorCode::CursorExpired
        ));
        assert!(output.next().await.is_none());
    }
}
