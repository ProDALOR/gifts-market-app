//! Cancellable typed requests and subscriptions, independent of session I/O.
use super::{
    queue::{Payload, QueueError, Scope, Sender},
    registry::Key,
};
use crate::{
    MarketApi,
    config::WebSocketLimits,
    error::{ApiError, Result},
    models::{ApiRequest, ApiResult, ServerFrame},
};
use actix_web::web::Bytes;
use market_actors::{DataUpdate, DataWatch};
use std::{future::Future, sync::Arc};
use tokio_util::sync::CancellationToken;

pub(super) enum Operation {
    Portfolio(market_portfolio::PortfolioQuery),
    Valuation(Box<market_valuation::ValuationQuery>),
    Request(ApiRequest),
    Orders(Box<market_actors::input::GiftOrderQuery>),
    Catalog(Vec<market_actors::input::CollectionRef>),
}
#[derive(Debug)]
pub(super) enum End {
    TerminalQueued,
    Cancelled,
    SlowConsumer,
    Failed,
}
fn end(result: std::result::Result<(), QueueError>) -> End {
    match result {
        Ok(()) => End::TerminalQueued,
        Err(QueueError::Cancelled) => End::Cancelled,
        Err(QueueError::SlowConsumer) => End::SlowConsumer,
    }
}
struct OperationContext {
    // Encoding jobs and outgoing frames share this identity without cloning its String.
    key: Arc<Key>,
    tx: Sender,
    cancel: CancellationToken,
    limits: WebSocketLimits,
}
impl OperationContext {
    async fn send(&self, payload: Payload, terminal: bool) -> std::result::Result<(), QueueError> {
        self.tx
            .send(
                Scope::Operation(self.key.clone()),
                payload,
                terminal,
                &self.cancel,
            )
            .await
    }
    async fn fail(&self, error: ApiError) -> End {
        end(self.send(Payload::Error(error), true).await)
    }
    async fn finish(&self, result: Result<Bytes>) -> End {
        let payload = result.map(Payload::Encoded).unwrap_or_else(Payload::Error);
        end(self.send(payload, true).await)
    }
    async fn setup<T>(
        &self,
        future: impl Future<Output = Result<T>>,
    ) -> std::result::Result<T, End> {
        let result = tokio::select! { biased;
            _ = self.cancel.cancelled() => return Err(End::Cancelled),
            result = tokio::time::timeout(self.limits.setup_timeout, future) =>
                result.unwrap_or_else(|_| Err(ApiError::timed_out("Subscription setup timed out"))),
        };
        match result {
            Ok(value) => Ok(value),
            Err(error) => Err(self.fail(error).await),
        }
    }
}
pub(super) async fn run(
    market: MarketApi,
    key: Key,
    operation: Operation,
    tx: Sender,
    cancel: CancellationToken,
    limits: WebSocketLimits,
) -> End {
    let _cancel_on_exit = cancel.clone().drop_guard();
    let context = OperationContext {
        key: Arc::new(key),
        tx,
        cancel,
        limits,
    };
    match operation {
        Operation::Portfolio(query) => portfolio_watch(&market, &context, query).await,
        Operation::Valuation(query) => valuation_watch(&market, &context, *query).await,
        Operation::Request(query) => request(&market, &context, query).await,
        Operation::Orders(query) => {
            data_watch(&market, &context, market.watch_orders(*query)).await
        }
        Operation::Catalog(ids) => {
            data_watch(&market, &context, market.subscribe_catalog(ids)).await
        }
    }
}
fn progress_only(
    previous: &market_portfolio::PortfolioState,
    current: &market_portfolio::PortfolioState,
) -> bool {
    previous.user_id == current.user_id
        && previous.inventory == current.inventory
        && match (&previous.valuation, &current.valuation) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
}
async fn portfolio_watch(
    market: &MarketApi,
    context: &OperationContext,
    query: market_portfolio::PortfolioQuery,
) -> End {
    let mut watch = match context.setup(market.portfolio_watch(query)).await {
        Ok(value) => value,
        Err(end) => return end,
    };
    let mut value = watch.current();
    let mut previous: Option<Arc<market_portfolio::PortfolioState>> = None;
    loop {
        let progress = previous.as_ref().is_some_and(|p| progress_only(p, &value));
        let key = context.key.clone();
        let source = value.clone();
        let encoded = market.encode_response(move |writer| {
            let frame = if progress {
                ServerFrame::PortfolioProgress {
                    id: &key.id,
                    refresh_id: source.refresh_id,
                    progress: &source.progress,
                    refresh_after: source.refresh_after,
                    error: source.error.as_deref(),
                }
            } else {
                ServerFrame::PortfolioSnapshot {
                    id: &key.id,
                    data: crate::portfolio::PortfolioResponse::from(source.as_ref()),
                }
            };
            serde_json::to_writer(writer, &frame)
        });
        let result = tokio::select! { biased; _ = context.cancel.cancelled() => return End::Cancelled, result = encoded => result };
        let bytes = match result {
            Ok(value) => value,
            Err(error) => return context.fail(error).await,
        };
        if let Err(error) = context.send(Payload::Encoded(bytes), false).await {
            return end(Err(error));
        }
        previous = Some(value);
        value = tokio::select! { biased;
            _ = context.cancel.cancelled() => return End::Cancelled,
            result = watch.changed() => match result {
                Ok(value) => value,
                Err(error) => return context.fail(ApiError::unavailable(error.to_string())).await,
            },
        };
    }
}
async fn valuation_watch(
    market: &MarketApi,
    context: &OperationContext,
    query: market_valuation::ValuationQuery,
) -> End {
    let mut watch = match context.setup(market.watch_valuation(query)).await {
        Ok(value) => value,
        Err(end) => return end,
    };
    let mut value = watch.current();
    loop {
        let key = context.key.clone();
        let encoded = market.encode_response(move |writer| {
            serde_json::to_writer(
                writer,
                &ServerFrame::ValuationSnapshot {
                    id: &key.id,
                    data: crate::valuation::ValuationResponse::from(value.as_ref()),
                },
            )
        });
        let result = tokio::select! { biased; _ = context.cancel.cancelled() => return End::Cancelled, result = encoded => result };
        let bytes = match result {
            Ok(value) => value,
            Err(error) => return context.fail(error).await,
        };
        if let Err(error) = context.send(Payload::Encoded(bytes), false).await {
            return end(Err(error));
        }
        value = tokio::select! { biased;
            _ = context.cancel.cancelled() => return End::Cancelled,
            result = watch.changed() => match result { Ok(value) => value, Err(error) => return context.fail(error.into()).await },
        };
    }
}
async fn request(market: &MarketApi, context: &OperationContext, query: ApiRequest) -> End {
    let work = async {
        match query {
            ApiRequest::PortfolioQuery { id, params } => {
                let data = market.portfolio_query(params).await?;
                market
                    .encode_response(move |writer| {
                        serde_json::to_writer(
                            writer,
                            &ServerFrame::Result(ApiResult::PortfolioQuery {
                                id: &id,
                                data: &data,
                            }),
                        )
                    })
                    .await
            }
            ApiRequest::PortfolioRefresh { id, params } => {
                let data = market.portfolio_refresh(params).await?;
                market
                    .encode_response(move |writer| {
                        serde_json::to_writer(
                            writer,
                            &ServerFrame::Result(ApiResult::PortfolioRefresh {
                                id: &id,
                                data: crate::portfolio::PortfolioResponse::from(data.as_ref()),
                            }),
                        )
                    })
                    .await
            }
            ApiRequest::ValuationBatch { id, params } => {
                let report = market.valuation_batch(*params).await?;
                market
                    .encode_response(move |writer| {
                        serde_json::to_writer(
                            writer,
                            &ServerFrame::Result(ApiResult::ValuationBatch {
                                id: &id,
                                data: crate::valuation::ValuationBatchResponse::from(
                                    report.as_ref(),
                                ),
                            }),
                        )
                    })
                    .await
            }
            ApiRequest::Valuation { id, params } => {
                let report = market.valuation(*params).await?;
                market
                    .encode_response(move |writer| {
                        serde_json::to_writer(
                            writer,
                            &ServerFrame::Result(ApiResult::Valuation {
                                id: &id,
                                data: crate::valuation::ValuationResponse::from(report.as_ref()),
                            }),
                        )
                    })
                    .await
            }
            ApiRequest::Orders { id, params } => {
                let read = market.query_orders(*params).await?;
                market
                    .encode_response(move |writer| {
                        serde_json::to_writer(
                            writer,
                            &ServerFrame::Result(ApiResult::Orders {
                                id: &id,
                                data: (&read).into(),
                            }),
                        )
                    })
                    .await
            }
        }
    };
    let result = tokio::select! { biased;
        _ = context.cancel.cancelled() => return End::Cancelled,
        result = tokio::time::timeout(context.limits.request_timeout, work) => result.unwrap_or_else(|_| Err(ApiError::timed_out("Request timed out"))),
    };
    context.finish(result).await
}
async fn data_watch(
    market: &MarketApi,
    context: &OperationContext,
    setup: impl Future<Output = Result<DataWatch>>,
) -> End {
    let DataWatch {
        lease: _lease,
        initial,
        mut updates,
    } = match context.setup(setup).await {
        Ok(value) => value,
        Err(end) => return end,
    };
    let mut initial = Some(initial);
    let mut previous = None;
    let mut epoch: Option<market_actors::ReadRevision> = None;
    let mut sequence = 0u64;
    loop {
        let first = initial.is_some();
        let update = initial
            .take()
            .unwrap_or_else(|| updates.borrow_and_update().clone());
        let message = match update {
            DataUpdate::Connecting => None,
            DataUpdate::Catalog(versions) => Some((Payload::Catalog(versions), false)),
            DataUpdate::Error(error) => Some((Payload::Error(error.into()), true)),
            DataUpdate::Orders {
                result,
                version: next_epoch,
                ..
            } => {
                let reset = epoch
                    .as_ref()
                    .is_some_and(|old| !old.same_dataset(&next_epoch));
                if !reset
                    && previous
                        .as_ref()
                        .is_some_and(|old| Arc::ptr_eq(old, &result))
                {
                    None
                } else {
                    let next_sequence = if previous.is_none() {
                        Some(0)
                    } else {
                        sequence
                            .checked_add(1)
                            .filter(|n| *n <= 9_007_199_254_740_991)
                    };
                    let Some(next_sequence) = next_sequence else {
                        return context
                            .fail(ApiError::unavailable(
                                "Subscription sequence exhausted; restart read",
                            ))
                            .await;
                    };
                    let key = context.key.clone();
                    let baseline = previous.clone().filter(|_| !reset);
                    let current = result.clone();
                    let encoded = market.encode_response(move |writer| {
                        crate::encoding::write_order_update(
                            writer,
                            &key.id,
                            baseline.as_deref(),
                            &current,
                            next_sequence,
                        )
                    });
                    let encoded = tokio::select! { biased; _ = context.cancel.cancelled() => return End::Cancelled, result = encoded => result };
                    match encoded {
                        Ok(bytes) => {
                            previous = Some(result);
                            epoch = Some(next_epoch);
                            sequence = next_sequence;
                            Some((Payload::Encoded(bytes), false))
                        }
                        Err(error) => Some((Payload::Error(error), true)),
                    }
                }
            }
        };
        if let Some((payload, terminal)) = message {
            let delivered = context.send(payload, terminal).await;
            if delivered.is_err() || terminal {
                return end(delivered);
            }
        }
        // The seed is caller-specific. Consume the receiver's current value before waiting.
        if first {
            continue;
        }
        let changed = tokio::select! { biased; _ = context.cancel.cancelled() => return End::Cancelled, result = updates.changed() => result };
        if changed.is_err() {
            return context
                .fail(ApiError::unavailable("Subscription feed closed"))
                .await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;
    fn context() -> (
        OperationContext,
        tokio::sync::mpsc::Receiver<super::super::queue::Frame>,
    ) {
        let limits = WebSocketLimits {
            setup_timeout: Duration::from_millis(20),
            ..Default::default()
        };
        let (tx, rx) = super::super::queue::channel(&limits);
        (
            OperationContext {
                key: Arc::new(Key {
                    id: "setup.1".into(),
                    generation: 1,
                }),
                tx,
                cancel: CancellationToken::new(),
                limits,
            },
            rx,
        )
    }
    #[tokio::test(start_paused = true)]
    async fn subscription_setup_has_one_deadline_and_correlated_terminal_error() {
        let (context, mut rx) = context();
        assert!(matches!(
            context.setup(std::future::pending::<Result<()>>()).await,
            Err(End::TerminalQueued)
        ));
        let frame = rx.recv().await.unwrap();
        assert!(frame.terminal);
        let message: serde_json::Value = serde_json::from_slice(&frame.text).unwrap();
        assert_eq!(message["id"], "setup.1");
        assert_eq!(message["error"]["code"], "timed_out");
    }
    #[tokio::test]
    async fn cancellation_drops_pending_setup_without_queuing_an_error() {
        struct Released(Arc<AtomicBool>);
        impl Drop for Released {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let (mut context, mut rx) = context();
        context.limits.setup_timeout = Duration::from_secs(60);
        let stop = context.cancel.clone();
        let started = Arc::new(tokio::sync::Notify::new());
        let released = Arc::new(AtomicBool::new(false));
        let task = {
            let released = released.clone();
            let started = started.clone();
            tokio::spawn(async move {
                context
                    .setup(async {
                        let _released = Released(released);
                        started.notify_one();
                        std::future::pending::<Result<()>>().await
                    })
                    .await
            })
        };
        started.notified().await;
        stop.cancel();
        assert!(matches!(task.await.unwrap(), Err(End::Cancelled)));
        assert!(released.load(Ordering::SeqCst));
        assert!(rx.try_recv().is_err());
    }
    #[test]
    fn portfolio_progress_requires_the_entire_inventory_and_report_to_be_unchanged() {
        let original: market_portfolio::PortfolioState = serde_json::from_value(serde_json::json!({
            "user_id":7,"refresh_id":1,
            "progress":{"stage":"ready","checked":1,"valued":1,"collectibles":1},
            "inventory":{"checked_at":"2026-01-01T00:00:00Z","count":1,"scanned":1,"regular":0,"skipped":0,"complete":true},
            "valuation":{"currency_id":"GRAM","evaluated_at":"2026-01-01T00:00:00Z","stale":false,"total":"0","priced":0,"unpriced":0,"items":[]}
        })).unwrap();
        let mut next = original.clone();
        next.refresh_id += 1;
        next.progress.stage = market_portfolio::PortfolioStage::Queued;
        assert!(progress_only(&original, &next));
        next.inventory.as_mut().unwrap().complete = false;
        assert!(
            !progress_only(&original, &next),
            "same timestamp must not hide changed inventory metadata"
        );
        next = original.clone();
        next.valuation = next.valuation.map(|v| Arc::new((*v).clone()));
        assert!(
            !progress_only(&original, &next),
            "replacement reports require a full snapshot"
        );
    }
}
