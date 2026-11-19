//! Tracked, bounded encoding of order snapshots and atomic range changes.
use crate::{
    error::{ApiError, Result},
    models::{ApiResult, ServerFrame},
    orders::*,
};
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
pub(crate) struct OrderEncoder {
    slots: Arc<Semaphore>,
    jobs: TaskTracker,
    limit: usize,
}
impl OrderEncoder {
    pub fn new(limit: usize) -> Self {
        Self {
            slots: Arc::new(Semaphore::new(4)),
            jobs: TaskTracker::new(),
            limit,
        }
    }
    pub async fn encode(&self, event: OrderEvent) -> Result<Arc<str>> {
        let permit = self
            .slots
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| ApiError::unavailable("Order encoding stopped"))?;
        let limit = self.limit;
        self.jobs
            .spawn_blocking(move || {
                let _permit = permit;
                encode(event, limit)
            })
            .await
            .map_err(|_| ApiError::unavailable("Order encoding failed"))?
    }
    pub async fn shutdown(&self) {
        self.jobs.close();
        self.jobs.wait().await;
    }
}
struct Limited {
    bytes: Vec<u8>,
    limit: usize,
    overflow: bool,
}
impl Write for Limited {
    fn write(&mut self, value: &[u8]) -> io::Result<usize> {
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
fn encode(event: OrderEvent, limit: usize) -> Result<Arc<str>> {
    let mut writer = Limited {
        bytes: vec![],
        limit,
        overflow: false,
    };
    let encoded = match event {
        OrderEvent::Read { id, read } => serde_json::to_writer(
            &mut writer,
            &ServerFrame::Result(ApiResult::Orders {
                id,
                data: read.as_ref().into(),
            }),
        ),
        OrderEvent::Update {
            id,
            previous,
            current,
            sequence,
            reset,
        } => {
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
                        data: OrderReadResponse::latest(&current),
                    },
                )
            }
        }
    };
    if encoded.is_err() {
        return Err(if writer.overflow {
            ApiError::capacity("Order response exceeds byte budget; narrow the range")
        } else {
            ApiError::unavailable("Order encoding failed")
        });
    }
    String::from_utf8(writer.bytes)
        .map(Arc::from)
        .map_err(|_| ApiError::unavailable("Invalid encoded order response"))
}
#[cfg(test)]
mod tests {
    use super::*;
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
        let encoder = OrderEncoder::new(1024);
        let read = OrderRead {
            result: empty(),
            snapshot: None,
            next: None,
        };
        let expected = serde_json::to_value(OrderReadResponse::from(&read)).unwrap();
        let text = encoder
            .encode(OrderEvent::Read {
                id: "x".into(),
                read: Box::new(read),
            })
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["data"], expected);
        let small = OrderEncoder::new(8);
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
