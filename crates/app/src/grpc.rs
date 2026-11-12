//! Read-only downstream service. Core credentials and registration never cross this boundary.
use crate::wire as a;
use crate::{
    error::ApiError,
    query::{self as q, *},
    status::ResponseMeta,
};
use futures_util::StreamExt;
use market_view as m;
use market_view::protocol::wire as w;
use std::{collections::BTreeMap, pin::Pin};
use tokio::net::TcpListener;
use tokio_stream::{Stream, wrappers::TcpListenerStream};
use tokio_util::sync::CancellationToken;
use tonic::{
    Request, Response, Status,
    transport::{Server, ServerTlsConfig},
};

const MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;
type Output<T> = Pin<Box<dyn Stream<Item = Result<T, Status>> + Send>>;
#[derive(Clone)]
struct Service {
    state: ApiState,
    stop: CancellationToken,
}
fn parse<T: std::str::FromStr>(value: String, field: &str) -> Result<T, Status>
where
    T::Err: std::fmt::Display,
{
    value.parse().map_err(|e| ApiError::field(field, e).into())
}
fn optional<T: std::str::FromStr>(value: Option<String>, field: &str) -> Result<Option<T>, Status>
where
    T::Err: std::fmt::Display,
{
    value.map(|v| parse(v, field)).transpose()
}
fn attributes(values: BTreeMap<String, w::AttributeValue>) -> Result<Option<String>, Status> {
    if values.is_empty() {
        return Ok(None);
    }
    let attrs: m::Attributes = values
        .into_iter()
        .map(|(key, value)| {
            Ok((
                parse(key, "attributes")?,
                m::AttributeValue::try_from(value)
                    .map_err(|e| Status::from(ApiError::field("attributes", e)))?,
            ))
        })
        .collect::<Result<_, Status>>()?;
    Ok(Some(
        serde_json::to_string(&market_json::attributes(&attrs)).expect("attributes JSON"),
    ))
}
fn book(v: a::BookQuery) -> Result<BookQuery, Status> {
    Ok(BookQuery {
        item_type_id: parse(v.item_type_id, "item_type_id")?,
        currency_id: parse(v.currency_id, "currency_id")?,
        feed_id: optional(v.feed_id, "feed_id")?,
        item_id: optional(v.item_id, "item_id")?,
        attributes: attributes(v.attributes)?,
        price_min: v.price_min,
        price_max: v.price_max,
        limit: v.limit.map(|v| v as usize),
        include: v.include,
    })
}
fn meta(v: ResponseMeta) -> a::ResponseMeta {
    a::ResponseMeta {
        cursor: v.cursor.map(|c| w::Cursor {
            generation: c.generation,
            revision: c.revision,
        }),
        catalog_revision: v.catalog_revision.unwrap_or_default(),
        served_at: v.served_at.to_string(),
        as_of: v.as_of.to_string(),
        replica_state: match v.replica.state {
            crate::status::SyncState::Live => "live",
            crate::status::SyncState::Cached => "cached",
            crate::status::SyncState::Syncing => "syncing",
        }
        .into(),
        last_contact_at: v.replica.last_contact_at.map(|t| t.to_string()),
    }
}
fn page(v: Option<Page>) -> Option<a::Page> {
    v.map(|v| a::Page {
        next_cursor: v.next_cursor,
    })
}
fn included(v: Option<Included>) -> Option<a::Included> {
    v.map(|v| a::Included {
        items: v.items.into_iter().map(Into::into).collect(),
        feeds: v.feeds.into_iter().map(feed).collect(),
    })
}
fn book_result(v: BookResponse) -> a::BookResult {
    a::BookResult {
        item_type_id: v.data.book.item_type_id.to_string(),
        currency_id: v.data.book.currency_id.to_string(),
        asks: Some(a::BookSidePage {
            data: v.data.asks.data.0.into_iter().map(Into::into).collect(),
            page: page(Some(v.data.asks.page)),
        }),
        bids: Some(a::BookSidePage {
            data: v.data.bids.data.0.into_iter().map(Into::into).collect(),
            page: page(Some(v.data.bids.page)),
        }),
        meta: Some(meta(v.meta)),
        included: included(v.included),
    }
}
#[tonic::async_trait]
impl a::market_api_server::MarketApi for Service {
    async fn get_status(&self, _: Request<w::Empty>) -> Result<Response<a::ApiStatus>, Status> {
        let v = self.state.status();
        Ok(Response::new(a::ApiStatus {
            ready: v.data.ready,
            records: v.data.records as u64,
            feeds: v.data.feeds as u32,
            ready_feeds: v.data.ready_feeds as u32,
            meta: Some(meta(v.meta)),
        }))
    }
    async fn list_catalog(
        &self,
        r: Request<a::CatalogQuery>,
    ) -> Result<Response<a::CatalogResult>, Status> {
        let v = r.into_inner();
        let v = self.state.catalog(&CatalogQuery {
            kind: v.kind,
            id: v.id,
            item_type_id: optional(v.item_type_id, "item_type_id")?,
            attribute_id: optional(v.attribute_id, "attribute_id")?,
            q: v.q,
            limit: v.limit.map(|n| n as usize),
            cursor: v.cursor,
        })?;
        Ok(Response::new(a::CatalogResult {
            data: v.data.0.into_iter().map(Into::into).collect(),
            page: page(v.page),
            meta: Some(meta(v.meta)),
        }))
    }
    async fn list_feeds(
        &self,
        r: Request<a::FeedsQuery>,
    ) -> Result<Response<a::FeedsResult>, Status> {
        let v = self.state.feeds(&FeedsQuery {
            feed_id: optional(r.into_inner().feed_id, "feed_id")?,
        })?;
        Ok(Response::new(a::FeedsResult {
            data: v.data.0.into_iter().map(feed).collect(),
            meta: Some(meta(v.meta)),
        }))
    }
    async fn list_items(
        &self,
        r: Request<a::ItemsQuery>,
    ) -> Result<Response<a::ItemsResult>, Status> {
        let v = r.into_inner();
        let v = self.state.items(&ItemsQuery {
            feed_id: optional(v.feed_id, "feed_id")?,
            item_type_id: optional(v.item_type_id, "item_type_id")?,
            item_id: optional(v.item_id, "item_id")?,
            attributes: attributes(v.attributes)?,
            limit: v.limit.map(|n| n as usize),
            cursor: v.cursor,
        })?;
        Ok(Response::new(a::ItemsResult {
            data: v.data.0.into_iter().map(Into::into).collect(),
            page: page(v.page),
            meta: Some(meta(v.meta)),
        }))
    }
    async fn list_orders(
        &self,
        r: Request<a::OrdersQuery>,
    ) -> Result<Response<a::OrdersResult>, Status> {
        let v = r.into_inner();
        let v = self.state.orders(&OrdersQuery {
            item_type_id: optional(v.item_type_id, "item_type_id")?,
            currency_id: optional(v.currency_id, "currency_id")?,
            side: v
                .side
                .map(|side| match w::OrderSide::try_from(side) {
                    Ok(w::OrderSide::Ask) => Ok(m::OrderSide::Ask),
                    Ok(w::OrderSide::Bid) => Ok(m::OrderSide::Bid),
                    _ => Err(Status::from(ApiError::field("side", "must be ask or bid"))),
                })
                .transpose()?,
            feed_id: optional(v.feed_id, "feed_id")?,
            item_id: optional(v.item_id, "item_id")?,
            order_id: optional(v.order_id, "order_id")?,
            attributes: attributes(v.attributes)?,
            price_min: v.price_min,
            price_max: v.price_max,
            limit: v.limit.map(|n| n as usize),
            cursor: v.cursor,
            include: v.include,
        })?;
        Ok(Response::new(a::OrdersResult {
            data: v.data.0.into_iter().map(Into::into).collect(),
            page: page(v.page),
            meta: Some(meta(v.meta)),
            included: included(v.included),
        }))
    }
    async fn get_order_book(
        &self,
        r: Request<a::BookQuery>,
    ) -> Result<Response<a::BookResult>, Status> {
        Ok(Response::new(book_result(
            self.state.orderbook(&book(r.into_inner())?)?,
        )))
    }
    type WatchOrderBookStream = Output<a::BookFrame>;
    async fn watch_order_book(
        &self,
        r: Request<a::BookQuery>,
    ) -> Result<Response<Self::WatchOrderBookStream>, Status> {
        let stream = self
            .state
            .book_stream(book(r.into_inner())?, self.stop.clone())?;
        Ok(Response::new(Box::pin(stream.map(|v| {
            v.map(|v| a::BookFrame {
                r#type: v.kind.into(),
                response: Some(book_result((*v.response).clone())),
            })
            .map_err(Into::into)
        }))))
    }
    type SnapshotStream = Output<a::ApiSnapshotFrame>;
    async fn snapshot(
        &self,
        r: Request<a::SnapshotQuery>,
    ) -> Result<Response<Self::SnapshotStream>, Status> {
        let v = r.into_inner();
        let (_, stream) = self.state.snapshot_stream(q::Scope {
            feed_id: optional(v.feed_id, "feed_id")?,
            item_type_id: optional(v.item_type_id, "item_type_id")?,
        })?;
        let stop = self.stop.clone();
        Ok(Response::new(Box::pin(
            stream.take_until(stop.cancelled_owned()).map(|v| {
                v.map(|v| a::ApiSnapshotFrame {
                    meta: Some(meta(v.meta)),
                    catalog: v.catalog.into_iter().map(Into::into).collect(),
                    records: v.records.into_iter().map(Into::into).collect(),
                    complete: v.complete,
                })
                .map_err(Into::into)
            }),
        )))
    }
    type WatchChangesStream = Output<a::ApiChangeFrame>;
    async fn watch_changes(
        &self,
        r: Request<a::ApiWatchQuery>,
    ) -> Result<Response<Self::WatchChangesStream>, Status> {
        let v = r.into_inner();
        let after = v.after.ok_or_else(|| {
            Status::from(ApiError::field(
                "after",
                "required; obtain a complete Snapshot first",
            ))
        })?;
        let stream = self.state.changes_stream(
            after.into(),
            q::Scope {
                feed_id: optional(v.feed_id, "feed_id")?,
                item_type_id: optional(v.item_type_id, "item_type_id")?,
            },
            self.stop.clone(),
        )?;
        Ok(Response::new(Box::pin(stream.map(|v| {
            v.map(|v| a::ApiChangeFrame {
                r#type: v.kind.into(),
                previous: Some(m::Cursor::from(v.previous).into()),
                cursor: Some(m::Cursor::from(v.cursor).into()),
                events: v.events.into_iter().map(Into::into).collect(),
                catalog: v.catalog.into_iter().map(Into::into).collect(),
                catalog_revision: v.catalog_revision,
                meta: Some(meta(v.meta)),
                complete: v.complete,
            })
            .map_err(Into::into)
        }))))
    }
}
/// The optional downstream key is independent of node registration credentials.
pub async fn serve(
    state: ApiState,
    listener: TcpListener,
    tls: Option<ServerTlsConfig>,
    token: Option<String>,
    stop: CancellationToken,
) -> anyhow::Result<()> {
    use sha2::{Digest, Sha256};
    use subtle::ConstantTimeEq;
    let expected = token.map(|token| Sha256::digest(token.as_bytes()));
    let auth = move |r: Request<()>| -> Result<Request<()>, Status> {
        if let Some(expected) = expected {
            let supplied = r
                .metadata()
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("Bearer "))
                .unwrap_or_default();
            if !bool::from(expected.ct_eq(&Sha256::digest(supplied.as_bytes()))) {
                return Err(Status::unauthenticated("invalid API credential"));
            }
        }
        Ok(r)
    };
    let service = a::market_api_server::MarketApiServer::new(Service {
        state,
        stop: stop.clone(),
    })
    .max_decoding_message_size(64 * 1024)
    .max_encoding_message_size(MAX_MESSAGE_BYTES);
    let service = tonic::service::interceptor::InterceptedService::new(service, auth);
    let mut server = Server::builder()
        .concurrency_limit_per_connection(16)
        .max_concurrent_streams(32)
        .load_shed(true);
    if let Some(tls) = tls {
        server = server.tls_config(tls)?;
    }
    server
        .add_service(service)
        .serve_with_incoming_shutdown(TcpListenerStream::new(listener), stop.cancelled_owned())
        .await?;
    Ok(())
}

fn feed(v: m::FeedStatus) -> w::FeedStatus {
    let record: w::Record = m::Record::Feed(v).into();
    let Some(w::record::Record::Feed(v)) = record.record else {
        unreachable!()
    };
    v
}
