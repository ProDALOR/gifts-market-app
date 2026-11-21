//! Tracked, cancellable and bounded response encoding shared by HTTP and WebSocket.
use crate::{
    error::{ApiError, Result},
    models::{ApiResult, ServerFrame},
    orders::*,
};
use actix_web::web::Bytes;
use market_actors::{OrderRead, OrderResult};
use std::{
    collections::HashMap,
    io::{self, Write},
    sync::Arc,
};
use tokio::sync::Semaphore;
use tokio_util::task::TaskTracker;
pub(crate) enum OrderEvent {
    Read {
        id: String,
        read: Box<OrderRead>,
    },
    Update {
        id: String,
        previous: Option<Arc<OrderResult>>,
        current: Arc<OrderResult>,
        sequence: u64,
        reset: bool,
    },
}
pub(crate) enum ValuationEnvelope {
    Rest,
    Query(String),
    Snapshot(String),
}
enum ResponseEvent {
    Orders(OrderEvent),
    Valuation {
        report: Arc<market_valuation::ValuationReport>,
        envelope: ValuationEnvelope,
    },
}
pub(crate) struct ResponseEncoder {
    slots: Arc<Semaphore>,
    jobs: TaskTracker,
    limit: usize,
    stop: tokio_util::sync::CancellationToken,
    registration: std::sync::Mutex<()>,
}
impl ResponseEncoder {
    pub fn new(limit: usize) -> Self {
        Self {
            slots: Arc::new(Semaphore::new(4)),
            jobs: TaskTracker::new(),
            limit,
            stop: tokio_util::sync::CancellationToken::new(),
            registration: std::sync::Mutex::new(()),
        }
    }
    pub async fn encode(&self, event: OrderEvent) -> Result<Bytes> {
        self.encode_response(ResponseEvent::Orders(event)).await
    }
    pub async fn valuation(
        &self,
        report: Arc<market_valuation::ValuationReport>,
        envelope: ValuationEnvelope,
    ) -> Result<Bytes> {
        self.encode_response(ResponseEvent::Valuation { report, envelope })
            .await
    }
    async fn encode_response(&self, event: ResponseEvent) -> Result<Bytes> {
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
                encode_response(event, limit, stop)
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
struct Limited {
    bytes: Vec<u8>,
    limit: usize,
    overflow: bool,
    stop: tokio_util::sync::CancellationToken,
}
impl Write for Limited {
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
    event: ResponseEvent,
    limit: usize,
    stop: tokio_util::sync::CancellationToken,
) -> Result<Bytes> {
    let mut writer = Limited {
        bytes: vec![],
        limit,
        overflow: false,
        stop,
    };
    let encoded = match event {
        ResponseEvent::Valuation { report, envelope } => {
            let data = crate::valuation::ValuationResponse::from(report.as_ref());
            match envelope {
                ValuationEnvelope::Rest => serde_json::to_writer(&mut writer, &data),
                ValuationEnvelope::Query(id) => serde_json::to_writer(
                    &mut writer,
                    &ServerFrame::Result(ApiResult::Valuation { id, data }),
                ),
                ValuationEnvelope::Snapshot(id) => {
                    serde_json::to_writer(&mut writer, &ServerFrame::ValuationSnapshot { id, data })
                }
            }
        }
        ResponseEvent::Orders(OrderEvent::Read { id, read }) => serde_json::to_writer(
            &mut writer,
            &ServerFrame::Result(ApiResult::Orders {
                id,
                data: read.as_ref().into(),
            }),
        ),
        ResponseEvent::Orders(OrderEvent::Update {
            id,
            previous,
            current,
            sequence,
            reset,
        }) => {
            if let Some(previous) = previous.filter(|_| !reset) {
                fn key(
                    row: &market_actors::OrderedEntry,
                ) -> (&market_gifts::VenueId, &market_gifts::OrderId) {
                    (&row.order.book.venue_id, &row.order.id)
                }
                let old: HashMap<_, _> = previous.data.iter().map(|row| (key(row), row)).collect();
                let new: HashMap<_, _> = current.data.iter().map(|row| (key(row), row)).collect();
                let upserts = current
                    .data
                    .iter()
                    .filter(|row| old.get(&key(row)).is_none_or(|old| **old != **row))
                    .map(Into::into)
                    .collect();
                let removed = previous
                    .data
                    .iter()
                    .filter(|row| !new.contains_key(&key(row)))
                    .map(|row| OrderIdentity {
                        venue_id: &row.order.book.venue_id,
                        id: &row.order.id,
                    })
                    .collect();
                serde_json::to_writer(
                    &mut writer,
                    &ServerFrame::OrdersChanged {
                        id,
                        sequence,
                        upserts,
                        removed,
                    },
                )
            } else {
                serde_json::to_writer(
                    &mut writer,
                    &ServerFrame::OrdersSnapshot {
                        id,
                        sequence,
                        data: (&*current).into(),
                    },
                )
            }
        }
    };
    if encoded.is_err() {
        return Err(if writer.overflow {
            ApiError::capacity("Response exceeds byte budget; narrow the query")
        } else {
            ApiError::unavailable("Response encoding failed")
        });
    }
    // Bytes takes ownership of the serialization buffer without copying it.
    Ok(Bytes::from(writer.bytes))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn valuation_report() -> Arc<market_valuation::ValuationReport> {
        Arc::new(serde_json::from_value(serde_json::json!({"target":{"kind":"selection","collection_id":"gift","criteria":{}},"currency_id":"GRAM","evaluated_at":"2026-01-01T00:00:00Z","stale":false,"results":[{"strategy":"ask_floor","data":{"price":{"value":"123.456789"},"evidence":{"gifts":2,"listings":3}}}]})).unwrap())
    }
    #[tokio::test]
    async fn valuation_encoding_is_bounded_and_shared_by_rest_requests_and_watches() {
        let encoder = ResponseEncoder::new(4096);
        let rest = encoder
            .valuation(valuation_report(), ValuationEnvelope::Rest)
            .await
            .unwrap();
        let rest: serde_json::Value = serde_json::from_slice(&rest).unwrap();
        for envelope in [
            ValuationEnvelope::Query("read".into()),
            ValuationEnvelope::Snapshot("watch".into()),
        ] {
            let encoded = encoder
                .valuation(valuation_report(), envelope)
                .await
                .unwrap();
            let frame: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
            assert_eq!(frame["data"], rest);
        }
        let small = ResponseEncoder::new(64);
        for envelope in [
            ValuationEnvelope::Rest,
            ValuationEnvelope::Query("read".into()),
            ValuationEnvelope::Snapshot("watch".into()),
        ] {
            let error = small
                .valuation(valuation_report(), envelope)
                .await
                .unwrap_err();
            assert!(matches!(
                error.code,
                crate::error::ErrorCode::CapacityExceeded
            ));
        }
        small.shutdown().await;
        assert!(
            small
                .valuation(valuation_report(), ValuationEnvelope::Rest)
                .await
                .is_err()
        );
        encoder.shutdown().await;
    }
    #[test]
    fn cancelled_encoding_stops_writing_without_consuming_the_byte_budget() {
        let stop = tokio_util::sync::CancellationToken::new();
        let mut writer = Limited {
            bytes: vec![],
            limit: 10,
            overflow: false,
            stop: stop.clone(),
        };
        writer.write_all(b"123").unwrap();
        stop.cancel();
        assert!(writer.write_all(b"456").is_err());
        assert_eq!(writer.bytes, b"123");
        assert!(!writer.overflow);
    }
    #[tokio::test]
    async fn shutdown_rejects_queued_encoders_and_cannot_register_jobs_after_draining() {
        let encoder = Arc::new(ResponseEncoder::new(4096));
        let held = encoder.slots.clone().acquire_many_owned(4).await.unwrap();
        let waiting = {
            let encoder = encoder.clone();
            tokio::spawn(async move {
                encoder
                    .valuation(valuation_report(), ValuationEnvelope::Rest)
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
                .valuation(valuation_report(), ValuationEnvelope::Rest)
                .await
                .is_err()
        );
        assert!(encoder.jobs.is_empty());
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
    async fn order_encoding_enforces_bytes_and_keeps_the_shared_read_contract() {
        let encoder = ResponseEncoder::new(1024);
        let read = OrderRead {
            result: empty(),
            snapshot: None,
        };
        let expected = serde_json::to_value(OrderReadResponse::from(&read)).unwrap();
        assert_eq!(
            expected,
            serde_json::json!({"data":[],"range":{"start":null,"end":null}})
        );
        let text = encoder
            .encode(OrderEvent::Read {
                id: "x".into(),
                read: Box::new(read),
            })
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&text).unwrap();
        assert_eq!(value["data"], expected["data"]);
        assert_eq!(value["range"], expected["range"]);
        let snapshot = encoder
            .encode(OrderEvent::Update {
                id: "x".into(),
                previous: None,
                current: Arc::new(empty()),
                sequence: 0,
                reset: false,
            })
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&snapshot).unwrap(),
            serde_json::json!({"type":"orders.snapshot","id":"x","sequence":0,"data":[],"range":{"start":null,"end":null}})
        );
        let delta = encoder
            .encode(OrderEvent::Update {
                id: "x".into(),
                previous: Some(Arc::new(empty())),
                current: Arc::new(empty()),
                sequence: 1,
                reset: false,
            })
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&delta).unwrap(),
            serde_json::json!({"type":"orders.changed","id":"x","sequence":1,"upserts":[],"removed":[]})
        );
        let small = ResponseEncoder::new(8);
        assert!(
            small
                .encode(OrderEvent::Update {
                    id: "x".into(),
                    previous: None,
                    current: Arc::new(empty()),
                    sequence: 0,
                    reset: false,
                })
                .await
                .is_err()
        );
        small.shutdown().await;
        encoder.shutdown().await;
    }
}
