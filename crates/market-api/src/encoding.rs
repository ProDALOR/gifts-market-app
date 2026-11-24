//! Tracked, cancellable and bounded serialization shared by HTTP and WebSocket.
use crate::{
    error::{ApiError, Result},
    models::ServerFrame,
    orders::*,
    protocol::OperationId,
};
use actix_web::web::Bytes;
use market_actors::OrderResult;
use std::{
    collections::HashMap,
    io::{self, Write},
    sync::Arc,
};
use tokio::sync::Semaphore;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

pub(crate) struct ResponseEncoder {
    slots: Arc<Semaphore>,
    jobs: TaskTracker,
    limit: usize,
    stop: CancellationToken,
    registration: std::sync::Mutex<()>,
}
impl ResponseEncoder {
    pub fn new(limit: usize) -> Self {
        Self {
            slots: Arc::new(Semaphore::new(4)),
            jobs: TaskTracker::new(),
            limit,
            stop: CancellationToken::new(),
            registration: std::sync::Mutex::new(()),
        }
    }
    /// Build borrowed public DTOs inside the admitted job, owning only their source data.
    pub async fn encode<F>(&self, serialize: F) -> Result<Bytes>
    where
        F: FnOnce(&mut ResponseWriter) -> serde_json::Result<()> + Send + 'static,
    {
        let stop = self.stop.child_token();
        let _cancel = stop.clone().drop_guard();
        let permit = self
            .slots
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| ApiError::unavailable("Response encoding stopped"))?;
        let limit = self.limit;
        // Shutdown and job registration share a gate; no job can appear after drain.
        let job = {
            let _registration = self
                .registration
                .lock()
                .expect("encoding registration lock");
            if self.stop.is_cancelled() {
                return Err(ApiError::unavailable("Response encoding stopped"));
            }
            self.jobs.spawn_blocking(move || {
                let _permit = permit;
                encode_response(serialize, limit, stop)
            })
        };
        job.await
            .map_err(|_| ApiError::unavailable("Response encoding failed"))?
    }
    pub async fn shutdown(&self) {
        {
            let _registration = self
                .registration
                .lock()
                .expect("encoding registration lock");
            self.stop.cancel();
            self.slots.close();
            self.jobs.close();
        }
        self.jobs.wait().await;
    }
}
/// Fields stay private so serialization cannot bypass cancellation or the byte budget.
pub(crate) struct ResponseWriter {
    bytes: Vec<u8>,
    limit: usize,
    overflow: bool,
    stop: CancellationToken,
}
impl Write for ResponseWriter {
    fn write(&mut self, value: &[u8]) -> io::Result<usize> {
        if self.stop.is_cancelled() {
            return Err(io::Error::other("response encoding cancelled"));
        }
        if value.len() > self.limit.saturating_sub(self.bytes.len()) {
            self.overflow = true;
            return Err(io::Error::other("response byte budget exceeded"));
        }
        self.bytes.extend_from_slice(value);
        Ok(value.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn encode_response(
    serialize: impl FnOnce(&mut ResponseWriter) -> serde_json::Result<()>,
    limit: usize,
    stop: CancellationToken,
) -> Result<Bytes> {
    // Queued blocking work may start after its HTTP/WS future was cancelled.
    if stop.is_cancelled() {
        return Err(ApiError::unavailable("Response encoding stopped"));
    }
    let mut writer = ResponseWriter {
        bytes: vec![],
        limit,
        overflow: false,
        stop,
    };
    if serialize(&mut writer).is_err() {
        return Err(if writer.overflow {
            ApiError::capacity("Response exceeds byte budget; narrow the query")
        } else {
            ApiError::unavailable("Response encoding failed")
        });
    }
    Ok(Bytes::from(writer.bytes))
}

/// A missing baseline initializes/resets a watch. Delta order follows the source rows.
pub(crate) fn write_order_update(
    writer: impl Write,
    id: &OperationId,
    previous: Option<&OrderResult>,
    current: &OrderResult,
    sequence: u64,
) -> serde_json::Result<()> {
    let Some(previous) = previous else {
        return serde_json::to_writer(
            writer,
            &ServerFrame::OrdersSnapshot {
                id,
                sequence,
                data: current.into(),
            },
        );
    };
    fn key(row: &market_actors::OrderedEntry) -> (&market_gifts::VenueId, &market_gifts::OrderId) {
        (&row.order.book.venue_id, &row.order.id)
    }
    let mut old: HashMap<_, _> = previous.data.iter().map(|row| (key(row), row)).collect();
    let upserts = current
        .data
        .iter()
        .filter_map(|row| {
            let prior = old.remove(&key(row));
            prior.is_none_or(|prior| prior != row).then(|| row.into())
        })
        .collect();
    let removed = previous
        .data
        .iter()
        .filter(|row| old.contains_key(&key(row)))
        .map(|row| OrderIdentity {
            venue_id: &row.order.book.venue_id,
            id: &row.order.id,
        })
        .collect();
    serde_json::to_writer(
        writer,
        &ServerFrame::OrdersChanged {
            id,
            sequence,
            upserts,
            removed,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ApiResult;
    use market_actors::OrderRead;
    use serde_json::json;
    fn valuation_report() -> Arc<market_valuation::ValuationReport> {
        Arc::new(serde_json::from_value(json!({"target":{"kind":"selection","collection_id":"gift","criteria":{}},"currency_id":"GRAM","evaluated_at":"2026-01-01T00:00:00Z","stale":false,"results":[{"strategy":"ask_floor","data":{"value":"123.456789","count":2}}]})).unwrap())
    }
    #[tokio::test]
    async fn public_valuation_frames_and_rest_share_bounded_serialization() {
        let encoder = ResponseEncoder::new(4096);
        let report = valuation_report();
        let source = report.clone();
        let rest = encoder
            .encode(move |w| {
                serde_json::to_writer(
                    w,
                    &crate::valuation::ValuationResponse::from(source.as_ref()),
                )
            })
            .await
            .unwrap();
        let rest: serde_json::Value = serde_json::from_slice(&rest).unwrap();
        for snapshot in [false, true] {
            let source = report.clone();
            let bytes = encoder
                .encode(move |w| {
                    let id: OperationId = "value.1".into();
                    let data = crate::valuation::ValuationResponse::from(source.as_ref());
                    let frame = if snapshot {
                        ServerFrame::ValuationSnapshot { id: &id, data }
                    } else {
                        ServerFrame::Result(ApiResult::Valuation { id: &id, data })
                    };
                    serde_json::to_writer(w, &frame)
                })
                .await
                .unwrap();
            let frame: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(frame["data"], rest);
            assert_eq!(frame["id"], "value.1");
        }
        let small = ResponseEncoder::new(64);
        let error = small
            .encode(move |w| {
                serde_json::to_writer(
                    w,
                    &crate::valuation::ValuationResponse::from(report.as_ref()),
                )
            })
            .await
            .unwrap_err();
        assert!(matches!(
            error.code,
            crate::error::ErrorCode::CapacityExceeded
        ));
        small.shutdown().await;
        encoder.shutdown().await;
    }
    #[tokio::test]
    async fn public_portfolio_frames_keep_progress_shape_and_byte_limits() {
        let source: Arc<market_portfolio::PortfolioState> = Arc::new(serde_json::from_value(json!({"user_id":7,"refresh_id":3,"progress":{"stage":"queued","checked":0,"pages":0,"valued":0,"collectibles":0}})).unwrap());
        let encoder = ResponseEncoder::new(4096);
        let data = source.clone();
        let rest = encoder
            .encode(move |w| serde_json::to_writer(w, data.as_ref()))
            .await
            .unwrap();
        let rest: serde_json::Value = serde_json::from_slice(&rest).unwrap();
        for kind in 0..4 {
            let data = source.clone();
            let bytes = encoder
                .encode(move |w| {
                    let id: OperationId = "portfolio.1".into();
                    let frame = match kind {
                        0 => ServerFrame::Result(ApiResult::PortfolioQuery {
                            id: &id,
                            data: &data,
                        }),
                        1 => ServerFrame::Result(ApiResult::PortfolioRefresh {
                            id: &id,
                            data: &data,
                        }),
                        2 => ServerFrame::PortfolioSnapshot {
                            id: &id,
                            data: &data,
                        },
                        _ => ServerFrame::PortfolioProgress {
                            id: &id,
                            refresh_id: data.refresh_id,
                            progress: &data.progress,
                            refresh_after: data.refresh_after,
                            error: data.error.as_deref(),
                        },
                    };
                    serde_json::to_writer(w, &frame)
                })
                .await
                .unwrap();
            let frame: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(frame["id"], "portfolio.1");
            if kind < 3 {
                assert_eq!(frame["data"], rest);
            } else {
                assert_eq!(
                    frame,
                    json!({"type":"portfolio.progress","id":"portfolio.1","refresh_id":3,"progress":rest["progress"],"refresh_after":null,"error":null})
                );
            }
        }
        let small = ResponseEncoder::new(8);
        assert!(matches!(
            small
                .encode(move |w| serde_json::to_writer(w, source.as_ref()))
                .await
                .unwrap_err()
                .code,
            crate::error::ErrorCode::CapacityExceeded
        ));
        small.shutdown().await;
        encoder.shutdown().await;
    }
    #[test]
    fn cancellation_stops_projection_and_writing_without_consuming_the_byte_budget() {
        let stop = CancellationToken::new();
        stop.cancel();
        let mut called = false;
        assert!(
            encode_response(
                |_| {
                    called = true;
                    Ok(())
                },
                10,
                stop.clone()
            )
            .is_err()
        );
        assert!(!called);
        let mut writer = ResponseWriter {
            bytes: vec![b'x'],
            limit: 10,
            overflow: false,
            stop,
        };
        assert!(writer.write_all(b"123").is_err());
        assert_eq!(writer.bytes, b"x");
        assert!(!writer.overflow);
    }
    #[tokio::test]
    async fn shutdown_rejects_queued_encoders_and_prevents_registration_after_drain() {
        let encoder = Arc::new(ResponseEncoder::new(4096));
        let held = encoder.slots.clone().acquire_many_owned(4).await.unwrap();
        let waiting = {
            let encoder = encoder.clone();
            tokio::spawn(async move {
                encoder
                    .encode(|w| serde_json::to_writer(w, &"unused"))
                    .await
            })
        };
        tokio::task::yield_now().await;
        encoder.shutdown().await;
        assert!(waiting.await.unwrap().is_err());
        drop(held);
        assert!(encoder.jobs.is_empty());
        assert!(
            encoder
                .encode(|w| serde_json::to_writer(w, &"unused"))
                .await
                .is_err()
        );
        assert!(encoder.jobs.is_empty());
    }
    #[tokio::test]
    async fn dropping_an_encoding_future_cancels_and_drains_its_active_writer() {
        use std::sync::Barrier;
        let encoder = Arc::new(ResponseEncoder::new(4096));
        let (started, waiting) = tokio::sync::oneshot::channel();
        let (finished, cancelled) = tokio::sync::oneshot::channel();
        let release = Arc::new(Barrier::new(2));
        let task = {
            let encoder = encoder.clone();
            let release = release.clone();
            tokio::spawn(async move {
                encoder
                    .encode(move |writer| {
                        let _ = started.send(());
                        release.wait();
                        let result = serde_json::to_writer(writer, &"cancelled response");
                        let _ = finished.send(result.is_err());
                        result
                    })
                    .await
            })
        };
        waiting.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        release.wait();
        // Assert caller cancellation before shutdown can cancel the parent token.
        assert!(cancelled.await.unwrap());
        encoder.shutdown().await;
        assert!(encoder.jobs.is_empty());
    }
    fn row(venue: &str, id: &str, price: &str) -> market_actors::OrderedEntry {
        use base64::{
            Engine,
            alphabet::Alphabet,
            engine::{GeneralPurpose, general_purpose::NO_PAD},
        };
        let mut bytes = vec![1]; // Identity sort, active asks.
        bytes.extend(venue.bytes());
        bytes.extend([0, 0]);
        bytes.extend(id.bytes());
        bytes.extend([0, 0, 128]);
        let encoding = GeneralPurpose::new(
            &Alphabet::new("0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ_abcdefghijklmnopqrstuvwxyz~")
                .unwrap(),
            NO_PAD,
        );
        let key = market_actors::ReadPosition::decode(&encoding.encode(bytes)).unwrap();
        let order = market_gifts::GiftOrderData {
            id: id.to_owned().try_into().unwrap(),
            book: market_gifts::GiftBook {
                venue_id: venue.to_owned().try_into().unwrap(),
                collection_id: "gift".into(),
                currency_id: "GRAM".into(),
            },
            kind: market_gifts::GiftOrderKind::Ask,
            target: market_gifts::GiftTarget::Criteria {
                criteria: Default::default(),
            },
            price: price.to_owned().try_into().unwrap(),
            quantity: 1,
            observed_at: "2026-01-01T00:00:00Z".parse().unwrap(),
            expires_at: None,
            executed_at: None,
            market_url: None,
        }
        .try_into()
        .unwrap();
        market_actors::OrderedEntry { order, key }
    }
    #[test]
    fn deltas_keep_venue_identity_and_deterministic_upsert_and_removal_order() {
        let previous = OrderResult {
            data: vec![
                row("v1", "remove-z", "1"),
                row("v1", "same", "1"),
                row("v2", "same", "2"),
                row("v1", "remove-a", "1"),
            ],
            ..empty()
        };
        let current = OrderResult {
            data: vec![
                row("v1", "new", "3"),
                row("v2", "same", "4"),
                row("v1", "same", "1"),
            ],
            ..empty()
        };
        let mut bytes = Vec::new();
        write_order_update(&mut bytes, &"orders.1".into(), Some(&previous), &current, 3).unwrap();
        let frame: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(frame["sequence"], 3);
        assert_eq!(
            frame["upserts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| (
                    v["book"]["venue_id"].as_str().unwrap(),
                    v["id"].as_str().unwrap()
                ))
                .collect::<Vec<_>>(),
            vec![("v1", "new"), ("v2", "same")]
        );
        assert_eq!(
            frame["removed"],
            json!([{"venue_id":"v1","id":"remove-z"},{"venue_id":"v1","id":"remove-a"}])
        );
        bytes.clear();
        write_order_update(&mut bytes, &"orders.1".into(), None, &current, 4).unwrap();
        let reset: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(reset["type"], "orders.snapshot");
        assert_eq!(reset["sequence"], 4);
        assert_eq!(reset["data"].as_array().unwrap().len(), 3);
    }
    fn empty() -> OrderResult {
        OrderResult {
            data: vec![],
            range: market_actors::KeyRange {
                start: None,
                end: None,
            },
        }
    }
    #[tokio::test]
    async fn order_reads_snapshots_and_deltas_keep_the_public_contract() {
        let encoder = ResponseEncoder::new(1024);
        let read = OrderRead {
            result: empty(),
            snapshot: None,
        };
        let text = encoder
            .encode(move |w| {
                let id: OperationId = "x".into();
                serde_json::to_writer(
                    w,
                    &ServerFrame::Result(ApiResult::Orders {
                        id: &id,
                        data: (&read).into(),
                    }),
                )
            })
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&text).unwrap(),
            json!({"type":"result","method":"orders.query","id":"x","data":[],"range":{"start":null,"end":null}})
        );
        for (sequence, baseline) in [(0, None), (1, Some(empty()))] {
            let snapshot = baseline.is_none();
            let bytes = encoder
                .encode(move |w| {
                    write_order_update(w, &"x".into(), baseline.as_ref(), &empty(), sequence)
                })
                .await
                .unwrap();
            let expected = if snapshot {
                json!({"type":"orders.snapshot","id":"x","sequence":0,"data":[],"range":{"start":null,"end":null}})
            } else {
                json!({"type":"orders.changed","id":"x","sequence":1,"upserts":[],"removed":[]})
            };
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
                expected
            );
        }
        encoder.shutdown().await;
    }
}
