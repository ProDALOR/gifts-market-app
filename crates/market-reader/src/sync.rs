//! Strict, bounded assembly of core snapshots and atomic delta chunks.
use market_view::protocol::{
    MAX_BATCH_RECORDS, MAX_MESSAGE_BYTES,
    catalog::{self, CatalogSnapshot},
    wire as w,
};
use market_view::{self as m, Cursor, Mutation, RecordKey};
use prost::Message;
use std::sync::Arc;
use tonic::{
    Request, Status,
    metadata::{Ascii, MetadataValue},
    transport::Channel,
};
#[derive(Clone)]
pub struct Limits {
    pub max_records: usize,
    pub max_record_bytes: usize,
    pub contact_timeout: std::time::Duration,
    pub transaction_timeout: std::time::Duration,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_records: 1_000_000,
            max_record_bytes: 256 * 1024 * 1024,
            contact_timeout: std::time::Duration::from_secs(10),
            transaction_timeout: std::time::Duration::from_secs(60),
        }
    }
}
pub struct Snapshot {
    pub cursor: Cursor,
    pub catalog: Arc<CatalogSnapshot>,
    pub records: Vec<m::Record>,
}
pub enum Update {
    Snapshot(Snapshot),
    Delta(m::ChangeBatch),
    Checkpoint(Cursor),
    Heartbeat(Cursor),
}
pub struct Reader {
    stream: tonic::Streaming<w::SyncFrame>,
    cursor: Option<Cursor>,
    snapshot: Option<Staging>,
    delta: Option<m::ChangeBatch>,
    delta_bytes: usize,
    limits: Limits,
    transaction_started: Option<tokio::time::Instant>,
}
struct Staging {
    cursor: Cursor,
    revision: String,
    total: u64,
    bytes: u64,
    entries: Vec<m::CatalogEntry>,
    catalog_bytes: usize,
    catalog: Option<Arc<CatalogSnapshot>>,
    records: Vec<m::Record>,
    received_bytes: usize,
    last: Option<RecordKey>,
}
impl Reader {
    pub async fn connect(
        channel: Channel,
        auth: &MetadataValue<Ascii>,
        after: Option<Cursor>,
        projection: w::ChangeProjection,
        limits: Limits,
    ) -> Result<Self, Status> {
        let mut client = w::market_replication_client::MarketReplicationClient::new(channel)
            .max_decoding_message_size(MAX_MESSAGE_BYTES)
            .max_encoding_message_size(MAX_MESSAGE_BYTES);
        let mut request = Request::new(w::SyncRequest {
            after: after.clone().map(Into::into),
            projection: projection as i32,
        });
        request.metadata_mut().insert("authorization", auth.clone());
        let stream = client.sync(request).await?.into_inner();
        Ok(Self {
            stream,
            cursor: after,
            snapshot: None,
            delta: None,
            delta_bytes: 0,
            limits,
            transaction_started: None,
        })
    }
    pub async fn message(&mut self) -> Result<Option<Update>, Status> {
        loop {
            let now = tokio::time::Instant::now();
            let deadline =
                self.transaction_started
                    .map_or(now + self.limits.contact_timeout, |start| {
                        (start + self.limits.transaction_timeout)
                            .min(now + self.limits.contact_timeout)
                    });
            let Some(frame) = tokio::time::timeout_at(deadline, self.stream.message())
                .await
                .map_err(|_| {
                    Status::deadline_exceeded("sync contact/transaction deadline exceeded")
                })??
            else {
                if self.snapshot.is_some() || self.delta.is_some() {
                    return Err(Status::data_loss("incomplete sync transaction"));
                }
                return Ok(None);
            };
            let cursor: Cursor = frame
                .cursor
                .clone()
                .ok_or_else(|| invalid("sync cursor missing"))?
                .into();
            if cursor.generation.is_empty() {
                return Err(invalid("sync generation missing"));
            }
            let kind =
                w::FrameKind::try_from(frame.kind).map_err(|_| invalid("unknown sync frame"))?;
            match kind {
                w::FrameKind::SnapshotStart => {
                    if self.cursor.is_some()
                        || self.snapshot.is_some()
                        || self.delta.is_some()
                        || !frame.catalog.is_empty()
                        || !frame.records.is_empty()
                        || !frame.events.is_empty()
                        || frame.complete
                    {
                        return Err(invalid("unexpected snapshot header"));
                    }
                    if frame.catalog_revision.is_empty()
                        || frame.catalog_revision.len() > 256
                        || frame.total_records > self.limits.max_records as u64
                        || frame.record_bytes > self.limits.max_record_bytes as u64
                    {
                        return Err(Status::resource_exhausted("oversized snapshot"));
                    }
                    self.transaction_started = Some(tokio::time::Instant::now());
                    self.snapshot = Some(Staging {
                        cursor,
                        revision: frame.catalog_revision,
                        total: frame.total_records,
                        bytes: frame.record_bytes,
                        entries: Vec::new(),
                        catalog_bytes: 0,
                        catalog: None,
                        records: Vec::new(),
                        received_bytes: 0,
                        last: None,
                    });
                }
                w::FrameKind::SnapshotData | w::FrameKind::SnapshotComplete => {
                    let staging = self
                        .snapshot
                        .as_mut()
                        .ok_or_else(|| invalid("snapshot data without header"))?;
                    if cursor != staging.cursor
                        || frame.catalog_revision != staging.revision
                        || !frame.events.is_empty()
                    {
                        return Err(invalid("snapshot identity changed"));
                    }
                    if kind == w::FrameKind::SnapshotData
                        && (frame.complete
                            || (frame.catalog.is_empty() && frame.records.is_empty()))
                    {
                        return Err(invalid("invalid snapshot chunk"));
                    }
                    if kind == w::FrameKind::SnapshotComplete
                        && (!frame.complete
                            || !frame.catalog.is_empty()
                            || !frame.records.is_empty())
                    {
                        return Err(invalid("invalid snapshot completion"));
                    }
                    if staging.catalog.is_some() && !frame.catalog.is_empty() {
                        return Err(invalid("catalog follows snapshot records"));
                    }
                    for wire in frame.catalog {
                        staging.catalog_bytes =
                            staging.catalog_bytes.saturating_add(wire.encoded_len());
                        if staging.catalog_bytes > catalog::MAX_CATALOG_BYTES
                            || staging.entries.len() >= catalog::MAX_CATALOG_ENTRIES
                        {
                            return Err(Status::resource_exhausted("oversized catalog"));
                        }
                        staging.entries.push(
                            wire.try_into()
                                .map_err(|_| invalid("invalid catalog entry"))?,
                        );
                    }
                    if !frame.records.is_empty() || kind == w::FrameKind::SnapshotComplete {
                        if staging.catalog.is_none() {
                            let catalog = CatalogSnapshot::received(
                                cursor.generation.clone(),
                                staging.revision.clone(),
                                Arc::new(
                                    m::Catalog::from_entries(std::mem::take(&mut staging.entries))
                                        .map_err(|_| invalid("invalid snapshot catalog"))?,
                                ),
                            );
                            staging.catalog = Some(Arc::new(catalog));
                        }
                        let catalog = staging.catalog.as_ref().expect("snapshot catalog");
                        for wire in frame.records {
                            staging.received_bytes =
                                staging.received_bytes.saturating_add(wire.encoded_len());
                            let record: m::Record = wire
                                .try_into()
                                .map_err(|_| invalid("invalid snapshot record"))?;
                            let key = record.key();
                            if staging.last.as_ref().is_some_and(|last| &key <= last) {
                                return Err(invalid("snapshot key order is invalid"));
                            }
                            staging.last = Some(key);
                            catalog
                                .catalog
                                .validate_mutation(&Mutation::Upsert(record.clone()))
                                .map_err(|_| invalid("invalid snapshot record"))?;
                            staging.records.push(record);
                        }
                        if staging.records.len() as u64 > staging.total
                            || staging.received_bytes as u64 > staging.bytes
                        {
                            return Err(invalid("snapshot exceeds declared size"));
                        }
                    }
                    if kind == w::FrameKind::SnapshotComplete {
                        let staging = self.snapshot.take().expect("snapshot staging");
                        if staging.records.len() as u64 != staging.total
                            || staging.received_bytes as u64 != staging.bytes
                        {
                            return Err(invalid("incomplete snapshot"));
                        }
                        self.cursor = Some(cursor.clone());
                        self.transaction_started = None;
                        return Ok(Some(Update::Snapshot(Snapshot {
                            cursor,
                            catalog: staging.catalog.expect("completed catalog"),
                            records: staging.records,
                        })));
                    }
                }
                w::FrameKind::Delta => {
                    let previous = self
                        .cursor
                        .as_ref()
                        .ok_or_else(|| invalid("delta before snapshot"))?;
                    if self.snapshot.is_some()
                        || cursor.generation != previous.generation
                        || previous.revision.checked_add(1) != Some(cursor.revision)
                        || !frame.records.is_empty()
                        || frame.catalog_revision.is_empty()
                        || frame.catalog_revision.len() > 256
                    {
                        return Err(invalid("invalid delta position"));
                    }
                    if self.delta.is_none() {
                        self.transaction_started = Some(tokio::time::Instant::now());
                    }
                    let delta = self.delta.get_or_insert_with(|| m::ChangeBatch {
                        cursor: cursor.clone(),
                        catalog_revision: frame.catalog_revision.clone(),
                        catalog: Vec::new(),
                        events: Vec::new(),
                    });
                    if delta.cursor != cursor || delta.catalog_revision != frame.catalog_revision {
                        return Err(invalid("delta chunk identity changed"));
                    }
                    self.delta_bytes = self.delta_bytes.saturating_add(frame.encoded_len());
                    if self.delta_bytes > catalog::MAX_CATALOG_BYTES + MAX_MESSAGE_BYTES
                        || delta.catalog.len() + frame.catalog.len() > catalog::MAX_CATALOG_ENTRIES
                        || delta.events.len() + frame.events.len() > MAX_BATCH_RECORDS
                    {
                        return Err(Status::resource_exhausted("oversized delta transaction"));
                    }
                    for entry in frame.catalog {
                        delta.catalog.push(
                            entry
                                .try_into()
                                .map_err(|_| invalid("invalid catalog delta"))?,
                        );
                    }
                    for event in frame.events {
                        delta.events.push(
                            event
                                .try_into()
                                .map_err(|_| invalid("invalid market event"))?,
                        );
                    }
                    if frame.complete {
                        self.cursor = Some(cursor);
                        self.delta_bytes = 0;
                        self.transaction_started = None;
                        return Ok(Some(Update::Delta(
                            self.delta.take().expect("complete delta"),
                        )));
                    }
                }
                w::FrameKind::Checkpoint | w::FrameKind::Heartbeat => {
                    if self.snapshot.is_some()
                        || self.delta.is_some()
                        || self.cursor.as_ref() != Some(&cursor)
                        || !frame.catalog.is_empty()
                        || !frame.records.is_empty()
                        || !frame.events.is_empty()
                        || !frame.complete
                    {
                        return Err(invalid("invalid sync control frame"));
                    }
                    return Ok(Some(if kind == w::FrameKind::Checkpoint {
                        Update::Checkpoint(cursor)
                    } else {
                        Update::Heartbeat(cursor)
                    }));
                }
                _ => return Err(invalid("unsupported sync frame")),
            }
        }
    }
}
fn invalid(message: &'static str) -> Status {
    Status::data_loss(message)
}
