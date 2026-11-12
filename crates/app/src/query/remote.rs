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
impl ApiState {
    async fn book_query(&self, q: &BookQuery) -> Result<w::BookQuery> {
        Ok(w::BookQuery {
            item_type_id: q.item_type_id.to_string(),
            currency_id: q.currency_id.to_string(),
            feed_id: q.feed_id.as_ref().map(ToString::to_string),
            item_id: q.item_id.as_ref().map(ToString::to_string),
            attributes: self
                .attributes(Some(&q.item_type_id), q.attributes.as_deref())
                .await?,
            price_min: q.price_min.clone(),
            price_max: q.price_max.clone(),
            limit: limit(q.limit)?,
            include_items: q.include_items,
            include_feeds: q.include_feeds,
        })
    }
}
impl ApiState {
    /// Only the JSON boundary needs schema lookup. Core receives explicit typed values
    /// and remains responsible for catalog rules and applicability validation.
    async fn attributes(
        &self,
        kind: Option<&m::ItemTypeId>,
        raw: Option<&str>,
    ) -> Result<Option<w::AttributeMap>> {
        let Some(raw) = raw else {
            return Ok(None);
        };
        if raw.len() > 16 * 1024 {
            return Err(ApiError::field(
                "attributes",
                "candidate attributes are too large",
            ));
        }
        let kind =
            kind.ok_or_else(|| ApiError::field("item_type_id", "required for attributes"))?;
        let values = market_json::parse_raw_attributes(raw.as_bytes())
            .map_err(|e| ApiError::field("attributes", e))?;
        if values.is_empty() {
            return Ok(Some(w::AttributeMap::default()));
        }
        // Each item type has at most 64 attributes; this page contains every applicable schema.
        let query = w::CatalogQuery {
            kind: Some("attribute".into()),
            item_type_id: Some(kind.to_string()),
            limit: Some(200),
            ..Default::default()
        };
        let page = self
            .call(|mut rpc, auth| async move {
                Ok(rpc.list_catalog(request(query, auth)).await?.into_inner())
            })
            .await
            .map_err(ApiError::from)?;
        if page.page.and_then(|p| p.next_cursor).is_some() {
            return Err(invalid("attribute schema exceeds catalog limit"));
        }
        let schemas = page
            .data
            .into_iter()
            .map(|v| m::CatalogEntry::try_from(v).map_err(invalid))
            .collect::<Result<Vec<_>>>()?;
        let attributes = values
            .into_iter()
            .map(|(id, value)| {
                let schema = schemas
                    .iter()
                    .find_map(|entry| match entry {
                        m::CatalogEntry::Attribute(v) if v.id == id => Some(v),
                        _ => None,
                    })
                    .ok_or_else(|| {
                        ApiError::field(
                            "attributes",
                            "attribute is not permitted for this item type",
                        )
                    })?;
                let value = market_json::decode_scalar(schema.value_type, value)
                    .map_err(|e| ApiError::field("attributes", e))?;
                Ok((id.to_string(), value.into()))
            })
            .collect::<Result<_>>()?;
        Ok(Some(w::AttributeMap { attributes }))
    }
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
            attributes: self
                .attributes(q.item_type_id.as_ref(), q.attributes.as_deref())
                .await?,
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
            attributes: self
                .attributes(q.item_type_id.as_ref(), q.attributes.as_deref())
                .await?,
            price_min: q.price_min.clone(),
            price_max: q.price_max.clone(),
            limit: limit(q.limit)?,
            cursor: q.cursor.clone(),
            include_items: q.include_items,
            include_feeds: q.include_feeds,
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
        let q = self.book_query(q).await?;
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
        let q = self.book_query(q).await?;
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
        serde_json::json!({"cursor":frame.cursor.map(|c|ApiCursor {generation:c.generation,revision:c.revision}),"kind":w::FrameKind::try_from(frame.kind).map_err(invalid)?.as_str_name().trim_start_matches("FRAME_KIND_").to_ascii_lowercase(),"catalog":catalog,"records":records,"events":events,"complete":frame.complete,"total_records":frame.total_records.to_string(),"record_bytes":frame.record_bytes.to_string()}),
    )
}
