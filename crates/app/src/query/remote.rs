use super::*;
use crate::status::{ApiCursor, ResponseMeta, StatusData};
use market_view as m;
fn invalid(error: impl std::fmt::Display) -> ApiError {
    ApiError::new(
        ErrorCode::CoreUnavailable,
        format!("invalid core response: {error}"),
    )
}
fn required<T>(v: Option<T>) -> Result<T> {
    v.ok_or_else(|| invalid("missing field"))
}
pub(crate) fn meta(v: Option<w::ReadMeta>) -> Result<ResponseMeta> {
    let v = required(v)?;
    Ok(ResponseMeta {
        cursor: v.cursor.map(|c| ApiCursor {
            generation: c.generation,
            revision: c.revision,
        }),
        catalog_revision: Some(v.catalog_revision),
        served_at: v.served_at.parse().map_err(invalid)?,
        as_of: v.as_of.parse().map_err(invalid)?,
    })
}
fn page(v: Option<w::Page>) -> Option<Page> {
    v.map(|v| Page {
        next_cursor: v.next_cursor,
    })
}
fn feed(v: w::FeedStatus) -> Result<m::FeedStatus> {
    match m::Record::try_from(w::Record {
        record: Some(w::record::Record::Feed(v)),
    })
    .map_err(invalid)?
    {
        m::Record::Feed(feed) => Ok(feed),
        _ => unreachable!(),
    }
}
fn limit(value: Option<usize>) -> Result<Option<u32>> {
    value
        .map(|v| {
            if !(1..=200).contains(&v) {
                Err(ApiError::field("limit", "must be 1..200"))
            } else {
                Ok(v as u32)
            }
        })
        .transpose()
}
fn included(v: Option<w::Included>) -> Result<Option<Included>> {
    v.map(|v| {
        Ok(Included {
            items: v
                .items
                .into_iter()
                .map(|v| v.try_into().map_err(invalid))
                .collect::<Result<_>>()?,
            feeds: v.feeds.into_iter().map(feed).collect::<Result<_>>()?,
        })
    })
    .transpose()
}
pub(crate) fn book_result(v: w::BookResult) -> Result<BookResponse> {
    let asks = required(v.asks)?;
    let bids = required(v.bids)?;
    Ok(Response {
        data: BookData {
            book: BookId {
                item_type_id: v.item_type_id.parse().map_err(invalid)?,
                currency_id: v.currency_id.parse().map_err(invalid)?,
            },
            asks: OrderPage {
                data: Orders(
                    asks.data
                        .into_iter()
                        .map(|v| v.try_into().map_err(invalid))
                        .collect::<Result<_>>()?,
                ),
                page: page(asks.page).unwrap_or(Page { next_cursor: None }),
            },
            bids: OrderPage {
                data: Orders(
                    bids.data
                        .into_iter()
                        .map(|v| v.try_into().map_err(invalid))
                        .collect::<Result<_>>()?,
                ),
                page: page(bids.page).unwrap_or(Page { next_cursor: None }),
            },
        },
        meta: meta(v.meta)?,
        page: None,
        included: included(v.included)?,
    })
}
pub(crate) fn book_query(q: &BookQuery) -> Result<w::BookQuery> {
    Ok(w::BookQuery {
        item_type_id: q.item_type_id.to_string(),
        currency_id: q.currency_id.to_string(),
        feed_id: q.feed_id.as_ref().map(ToString::to_string),
        item_id: q.item_id.as_ref().map(ToString::to_string),
        attributes_json: q.attributes.clone(),
        price_min: q.price_min.clone(),
        price_max: q.price_max.clone(),
        limit: limit(q.limit)?,
        include: q.include.clone(),
    })
}
impl ApiState {
    pub async fn status(&self) -> Result<StatusResponse> {
        let v = self
            .call(|mut rpc, auth| async move {
                Ok(rpc
                    .get_status(request(w::Empty {}, auth))
                    .await?
                    .into_inner())
            })
            .await
            .map_err(ApiError::from)?;
        Ok(Response {
            data: StatusData {
                ready: v.ready,
                records: v.records as usize,
                feeds: v.feeds as usize,
                ready_feeds: v.ready_feeds as usize,
            },
            meta: meta(v.meta)?,
            page: None,
            included: None,
        })
    }
    pub async fn catalog(&self, q: &CatalogQuery) -> Result<CatalogResponse> {
        let q = w::CatalogQuery {
            kind: q.kind.clone(),
            id: q.id.clone(),
            item_type_id: q.item_type_id.as_ref().map(ToString::to_string),
            attribute_id: q.attribute_id.as_ref().map(ToString::to_string),
            q: q.q.clone(),
            limit: limit(q.limit)?,
            cursor: q.cursor.clone(),
        };
        let v = self
            .call(|mut rpc, auth| {
                let q = q.clone();
                async move { Ok(rpc.list_catalog(request(q, auth)).await?.into_inner()) }
            })
            .await
            .map_err(ApiError::from)?;
        Ok(Response {
            data: CatalogEntries(
                v.data
                    .into_iter()
                    .map(|v| v.try_into().map_err(invalid))
                    .collect::<Result<_>>()?,
            ),
            page: page(v.page),
            meta: meta(v.meta)?,
            included: None,
        })
    }
    pub async fn feeds(&self, q: &FeedsQuery) -> Result<FeedsResponse> {
        let q = w::FeedsQuery {
            feed_id: q.feed_id.as_ref().map(ToString::to_string),
        };
        let v = self
            .call(|mut rpc, auth| {
                let q = q.clone();
                async move { Ok(rpc.list_feeds(request(q, auth)).await?.into_inner()) }
            })
            .await
            .map_err(ApiError::from)?;
        Ok(Response {
            data: Feeds(v.data.into_iter().map(feed).collect::<Result<_>>()?),
            meta: meta(v.meta)?,
            page: None,
            included: None,
        })
    }
    pub async fn items(&self, q: &ItemsQuery) -> Result<ItemsResponse> {
        let q = w::ItemsQuery {
            feed_id: q.feed_id.as_ref().map(ToString::to_string),
            item_type_id: q.item_type_id.as_ref().map(ToString::to_string),
            item_id: q.item_id.as_ref().map(ToString::to_string),
            attributes_json: q.attributes.clone(),
            limit: limit(q.limit)?,
            cursor: q.cursor.clone(),
        };
        let v = self
            .call(|mut rpc, auth| {
                let q = q.clone();
                async move { Ok(rpc.list_items(request(q, auth)).await?.into_inner()) }
            })
            .await
            .map_err(ApiError::from)?;
        Ok(Response {
            data: Items(
                v.data
                    .into_iter()
                    .map(|v| v.try_into().map_err(invalid))
                    .collect::<Result<_>>()?,
            ),
            meta: meta(v.meta)?,
            page: page(v.page),
            included: None,
        })
    }
    pub async fn orders(&self, q: &OrdersQuery) -> Result<OrdersResponse> {
        let q = w::OrdersQuery {
            feed_id: q.feed_id.as_ref().map(ToString::to_string),
            item_type_id: q.item_type_id.as_ref().map(ToString::to_string),
            currency_id: q.currency_id.as_ref().map(ToString::to_string),
            side: q.side.map(|v| match v {
                m::OrderSide::Ask => w::OrderSide::Ask as i32,
                m::OrderSide::Bid => w::OrderSide::Bid as i32,
            }),
            item_id: q.item_id.as_ref().map(ToString::to_string),
            order_id: q.order_id.as_ref().map(ToString::to_string),
            attributes_json: q.attributes.clone(),
            price_min: q.price_min.clone(),
            price_max: q.price_max.clone(),
            limit: limit(q.limit)?,
            cursor: q.cursor.clone(),
            include: q.include.clone(),
        };
        let v = self
            .call(|mut rpc, auth| {
                let q = q.clone();
                async move { Ok(rpc.list_orders(request(q, auth)).await?.into_inner()) }
            })
            .await
            .map_err(ApiError::from)?;
        Ok(Response {
            data: Orders(
                v.data
                    .into_iter()
                    .map(|v| v.try_into().map_err(invalid))
                    .collect::<Result<_>>()?,
            ),
            meta: meta(v.meta)?,
            page: page(v.page),
            included: included(v.included)?,
        })
    }
    pub async fn orderbook(&self, q: &BookQuery) -> Result<BookResponse> {
        let q = book_query(q)?;
        let v = self
            .call(|mut rpc, auth| {
                let q = q.clone();
                async move { Ok(rpc.get_order_book(request(q, auth)).await?.into_inner()) }
            })
            .await
            .map_err(ApiError::from)?;
        book_result(v)
    }
    pub async fn trades(&self, q: &TradesQuery) -> Result<TradesResponse> {
        let q = w::TradesQuery {
            feed_id: q.feed_id.as_ref().map(ToString::to_string),
            item_type_id: q.item_type_id.as_ref().map(ToString::to_string),
            currency_id: q.currency_id.as_ref().map(ToString::to_string),
            item_id: q.item_id.as_ref().map(ToString::to_string),
            executed_from: q.executed_from.map(|t| t.to_string()),
            executed_until: q.executed_until.map(|t| t.to_string()),
            limit: limit(q.limit)?,
            cursor: q.cursor.clone(),
        };
        let v = self
            .call(|mut rpc, auth| {
                let q = q.clone();
                async move { Ok(rpc.list_trades(request(q, auth)).await?.into_inner()) }
            })
            .await
            .map_err(ApiError::from)?;
        let trades = v
            .data
            .into_iter()
            .map(|v| {
                let trade: m::Trade = v.try_into().map_err(invalid)?;
                market_json::source_value(&m::SourceEvent::Trade(trade)).map_err(invalid)
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Response {
            data: Trades(trades),
            meta: meta(v.meta)?,
            page: page(v.page),
            included: None,
        })
    }
    pub async fn watch_book(&self, q: &BookQuery) -> Result<tonic::Streaming<w::BookResult>> {
        let q = book_query(q)?;
        self.call(|mut rpc, auth| {
            let q = q.clone();
            async move {
                let mut request = tonic::Request::new(q);
                request.metadata_mut().insert("authorization", auth);
                Ok(tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    rpc.watch_order_book(request),
                )
                .await
                .map_err(|_| Status::deadline_exceeded("book subscription timed out"))??
                .into_inner())
            }
        })
        .await
        .map_err(Into::into)
    }
}

pub(crate) fn sync_response(frame: w::SyncFrame) -> Result<serde_json::Value> {
    let records = frame
        .records
        .into_iter()
        .map(|r| {
            let r: m::Record = r.try_into().map_err(invalid)?;
            market_json::record_value(&r).map_err(invalid)
        })
        .collect::<Result<Vec<_>>>()?;
    let events = frame
        .events
        .into_iter()
        .map(|e| {
            let e: m::MarketEvent = e.try_into().map_err(invalid)?;
            market_json::event_value(&e).map_err(invalid)
        })
        .collect::<Result<Vec<_>>>()?;
    let catalog = frame
        .catalog
        .into_iter()
        .map(|e| {
            let e: m::CatalogEntry = e.try_into().map_err(invalid)?;
            market_json::catalog_value(&e).map_err(invalid)
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(
        serde_json::json!({"cursor":frame.cursor.map(|c|ApiCursor {generation:c.generation,revision:c.revision}),"kind":w::FrameKind::try_from(frame.kind).map_err(invalid)?.as_str_name().trim_start_matches("FRAME_KIND_").to_ascii_lowercase(),"catalog_revision":frame.catalog_revision,"catalog":catalog,"records":records,"events":events,"complete":frame.complete,"total_records":frame.total_records.to_string(),"record_bytes":frame.record_bytes.to_string()}),
    )
}
