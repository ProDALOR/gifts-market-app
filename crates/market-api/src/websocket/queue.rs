use super::registry::{Key, OperationId, Registry};
use crate::{config::WebSocketLimits, error::ApiError, models::ServerFrame};
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc};
use tokio_util::sync::CancellationToken;
#[derive(Clone)]
pub(super) enum Payload {
    ValuationSnapshot(Arc<market_valuation::ValuationReport>),
    Valuation(Arc<market_valuation::ValuationReport>),
    Orders(Arc<str>),
    MarketStatus(crate::models::MarketStatus),
    Catalog(market_gifts::CatalogVersions),
    Error(ApiError),
    CommandError(ApiError),
}
impl Payload {
    #[cfg(test)]
    pub fn test_orders(result: Arc<str>) -> Self {
        Self::Orders(serde_json::json!({"type":"orders.snapshot","id":"offers","sequence":0,"data":[],"range":{"start":null,"end":null},"padding":&*result}).to_string().into())
    }
    pub fn text(&self, id: &str) -> String {
        match self {
            Self::ValuationSnapshot(data) => {
                serde_json::to_string(&ServerFrame::ValuationSnapshot {
                    id: id.into(),
                    data: crate::valuation::ValuationResponse::from(data.as_ref()),
                })
                .expect("market values serialize")
            }
            Self::Valuation(report) => {
                serde_json::to_string(&ServerFrame::Result(crate::models::ApiResult::Valuation {
                    id: id.into(),
                    data: crate::valuation::ValuationResponse::from(&**report),
                }))
                .expect("target valuation serializes")
            }
            Self::Orders(encoded) => encoded.to_string(),
            Self::MarketStatus(status) => {
                serde_json::to_string(&ServerFrame::MarketStatus { status: *status })
                    .expect("market status serializes")
            }
            Self::Catalog(versions) => serde_json::to_string(&ServerFrame::Catalog {
                id: id.into(),
                versions: crate::catalog::versions(versions.clone()),
            })
            .expect("catalog frame serializes"),
            Self::CommandError(error) => serde_json::to_string(&ServerFrame::CommandError {
                id: id.into(),
                error: error.clone(),
            })
            .expect("command error serializes"),
            Self::Error(error) => serde_json::to_string(&ServerFrame::Error {
                id: id.into(),
                error: error.clone(),
            })
            .expect("public error serializes"),
        }
    }
}
#[derive(Clone)]
pub(super) enum Scope {
    Operation(Key),
    Absent(OperationId),
    Global,
}
impl Scope {
    pub fn id(&self) -> &str {
        match self {
            Self::Operation(key) => key.id.as_str(),
            Self::Absent(id) => id.as_str(),
            Self::Global => "",
        }
    }
    pub fn current(&self, registry: &Registry) -> bool {
        match self {
            Self::Operation(key) => registry.current(key),
            Self::Absent(id) => registry.key(id).is_none(),
            Self::Global => true,
        }
    }
}
pub(super) struct Frame {
    pub scope: Scope,
    pub text: actix_web::web::Bytes,
    pub terminal: bool,
    _bytes: OwnedSemaphorePermit,
}
#[derive(Clone)]
pub(super) struct Sender {
    tx: mpsc::Sender<Frame>,
    bytes: Arc<Semaphore>,
    limits: WebSocketLimits,
}
#[derive(Debug, PartialEq, Eq)]
pub(super) enum QueueError {
    Cancelled,
    SlowConsumer,
}
pub(super) fn channel(limits: &WebSocketLimits) -> (Sender, mpsc::Receiver<Frame>) {
    let (tx, rx) = mpsc::channel(limits.queue_messages);
    (
        Sender {
            tx,
            bytes: Arc::new(Semaphore::new(limits.queue_bytes)),
            limits: limits.clone(),
        },
        rx,
    )
}
impl Sender {
    pub fn status(&self, status: crate::models::MarketStatus) -> Result<(), QueueError> {
        self.try_payload(Scope::Global, Payload::MarketStatus(status))
    }
    pub async fn send(
        &self,
        scope: Scope,
        payload: Payload,
        terminal: bool,
        cancel: &CancellationToken,
    ) -> Result<(), QueueError> {
        let text: actix_web::web::Bytes = payload.text(scope.id()).into();
        let size = text.len();
        if size > self.limits.queue_bytes {
            return Err(QueueError::SlowConsumer);
        }
        let work = async {
            let permit = self
                .bytes
                .clone()
                .acquire_many_owned(size as u32)
                .await
                .map_err(|_| QueueError::SlowConsumer)?;
            self.tx
                .send(Frame {
                    scope,
                    text,
                    terminal,
                    _bytes: permit,
                })
                .await
                .map_err(|_| QueueError::SlowConsumer)
        };
        tokio::select! {biased;_=cancel.cancelled()=>Err(QueueError::Cancelled),result=tokio::time::timeout(self.limits.write_timeout,work)=>result.unwrap_or(Err(QueueError::SlowConsumer))}
    }
    pub fn error(&self, id: OperationId, error: ApiError) -> Result<(), QueueError> {
        self.try_payload(Scope::Absent(id), Payload::Error(error))
    }
    pub fn reject(&self, scope: Scope, error: ApiError) -> Result<(), QueueError> {
        self.try_payload(scope, Payload::CommandError(error))
    }
    fn try_payload(&self, scope: Scope, payload: Payload) -> Result<(), QueueError> {
        let text: actix_web::web::Bytes = payload.text(scope.id()).into();
        let size = text.len();
        let permit = self
            .bytes
            .clone()
            .try_acquire_many_owned(u32::try_from(size).map_err(|_| QueueError::SlowConsumer)?)
            .map_err(|_| QueueError::SlowConsumer)?;
        self.tx
            .try_send(Frame {
                scope,
                text,
                terminal: false,
                _bytes: permit,
            })
            .map_err(|_| QueueError::SlowConsumer)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn queue_budget_includes_inflight_frames_and_cancelled_generations_are_filtered() {
        let limits = WebSocketLimits {
            queue_bytes: 1024,
            queue_messages: 2,
            write_timeout: std::time::Duration::from_millis(10),
            ..Default::default()
        };
        let (tx, mut rx) = channel(&limits);
        let stop = CancellationToken::new();
        let id = OperationId::parse("offers".into()).unwrap();
        let mut registry = Registry::new(1, 1);
        let (old, _) = registry.reserve(id.clone(), &stop).unwrap();
        let payload = Payload::test_orders(Arc::from(" ".repeat(600)));
        tx.send(Scope::Operation(old), payload.clone(), false, &stop)
            .await
            .unwrap();
        let inflight = rx.recv().await.unwrap();
        assert_eq!(
            tx.send(Scope::Global, payload.clone(), false, &stop).await,
            Err(QueueError::SlowConsumer)
        );
        registry.remove(&id);
        registry.reserve(id, &stop).unwrap();
        assert!(!inflight.scope.current(&registry));
        drop(inflight);
        tx.send(Scope::Global, payload, false, &stop).await.unwrap();
    }
}
