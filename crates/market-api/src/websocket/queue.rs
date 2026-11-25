use super::registry::{Key, Registry};
use crate::{config::WebSocketLimits, error::ApiError, models::ServerFrame, protocol::OperationId};
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc};
use tokio_util::sync::CancellationToken;
#[derive(Clone)]
pub(super) enum Payload {
    Encoded(actix_web::web::Bytes),
    MarketStatus(crate::models::MarketStatus),
    Catalog(market_actors::input::CatalogVersions),
    Error(ApiError),
    CommandError(ApiError),
}
impl Payload {
    #[cfg(test)]
    pub fn test_orders(result: Arc<str>) -> Self {
        Self::Encoded(serde_json::json!({"type":"orders.snapshot","id":"offers","sequence":0,"data":[],"range":{"start":null,"end":null},"padding":&*result}).to_string().into())
    }
    pub fn into_bytes(self, id: Option<&OperationId>) -> actix_web::web::Bytes {
        let text = match self {
            Self::Encoded(encoded) => return encoded,
            Self::MarketStatus(status) => {
                serde_json::to_string(&ServerFrame::MarketStatus { status })
                    .expect("market status serializes")
            }
            Self::Catalog(versions) => serde_json::to_string(&ServerFrame::Catalog {
                id: id.expect("correlated operation frame"),
                versions: crate::catalog::versions(versions),
            })
            .expect("catalog frame serializes"),
            Self::CommandError(error) => {
                serde_json::to_string(&ServerFrame::CommandError { id, error: &error })
                    .expect("command error serializes")
            }
            Self::Error(error) => serde_json::to_string(&ServerFrame::Error {
                id: id.expect("correlated operation frame"),
                error: &error,
            })
            .expect("public error serializes"),
        };
        text.into()
    }
}
#[derive(Clone)]
pub(super) enum Scope {
    Operation(Arc<Key>),
    Absent(OperationId),
    Global,
}
impl Scope {
    pub fn id(&self) -> Option<&OperationId> {
        match self {
            Self::Operation(key) => Some(&key.id),
            Self::Absent(id) => Some(id),
            Self::Global => None,
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
        if cancel.is_cancelled() {
            return Err(QueueError::Cancelled);
        }
        let text: actix_web::web::Bytes = payload.into_bytes(scope.id());
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
        let text: actix_web::web::Bytes = payload.into_bytes(scope.id());
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
    #[test]
    fn errors_keep_typed_correlation_and_the_uncorrelated_empty_id() {
        let id: OperationId = "request.1".into();
        for payload in [
            Payload::Error(ApiError::invalid("failed")),
            Payload::CommandError(ApiError::invalid("failed")),
        ] {
            let frame: serde_json::Value =
                serde_json::from_slice(&payload.into_bytes(Some(&id))).unwrap();
            assert_eq!(frame["id"], "request.1");
            assert_eq!(
                frame["error"],
                serde_json::json!({"code":"invalid_query","message":"failed"})
            );
        }
        let frame: serde_json::Value = serde_json::from_slice(
            &Payload::CommandError(ApiError::invalid("invalid API message")).into_bytes(None),
        )
        .unwrap();
        assert_eq!(
            frame,
            serde_json::json!({"type":"command_error","id":"","error":{"code":"invalid_query","message":"invalid API message"}})
        );
        let frame: serde_json::Value = serde_json::from_slice(
            &Payload::MarketStatus(crate::models::MarketStatus::Available).into_bytes(None),
        )
        .unwrap();
        assert_eq!(
            frame,
            serde_json::json!({"type":"market.status","status":"available"})
        );
    }
    #[tokio::test]
    async fn encoded_frames_share_the_serialization_buffer_through_the_queue() {
        let bytes = actix_web::web::Bytes::from(vec![b'x'; 1024]);
        let payload = Payload::Encoded(bytes.clone());
        assert_eq!(payload.clone().into_bytes(None).as_ptr(), bytes.as_ptr());
        let (tx, mut rx) = channel(&WebSocketLimits::default());
        tx.send(Scope::Global, payload, false, &CancellationToken::new())
            .await
            .unwrap();
        let frame = rx.recv().await.unwrap();
        assert_eq!(frame.text.as_ptr(), bytes.as_ptr());
        assert_eq!(frame.text, bytes);
    }
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
        let identity = Arc::new(old);
        let payload = Payload::test_orders(Arc::from(" ".repeat(600)));
        tx.send(
            Scope::Operation(identity.clone()),
            payload.clone(),
            false,
            &stop,
        )
        .await
        .unwrap();
        let inflight = rx.recv().await.unwrap();
        let Scope::Operation(shared) = &inflight.scope else {
            panic!("operation frame");
        };
        assert!(
            Arc::ptr_eq(shared, &identity),
            "frames retain the same operation identity"
        );
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
