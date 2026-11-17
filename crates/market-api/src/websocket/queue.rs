use super::registry::{Key, Registry, SubscriptionId};
use crate::{
    config::WebSocketLimits,
    error::ApiError,
    models::{ServerFrame, SubscriptionStatus},
};
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc};
use tokio_util::sync::CancellationToken;
#[derive(Clone)]
pub(super) enum Payload {
    Connecting,
    Page {
        page: Arc<str>,
        sync: market_gifts::SyncStatus,
        revision: String,
    },
    Sync {
        sync: market_gifts::SyncStatus,
        revision: String,
    },
    Catalog(market_gifts::CatalogVersions),
    Error(ApiError),
}
impl Payload {
    #[cfg(test)]
    pub fn test_page(page: Arc<str>) -> Self {
        Self::Page {
            page,
            sync: market_gifts::SyncStatus {
                ready: false,
                state: market_gifts::SyncState::Starting,
                updated_at: None,
            },
            revision: "1".into(),
        }
    }

    pub fn text(&self, id: &str) -> String {
        match self {
            // IDs here have already passed SubscriptionId validation. Page bytes are
            // bounded JSON produced by serde, shared independently of the frame ID.
            Self::Page {
                page,
                sync,
                revision,
            } => {
                let id = serde_json::to_string(id).expect("string serializes");
                let sync = serde_json::to_string(sync).expect("sync serializes");
                let revision = serde_json::to_string(revision).expect("revision serializes");
                format!(
                    "{{\"type\":\"page\",\"id\":{id},\"page\":{page},\"sync\":{sync},\"revision\":{revision}}}"
                )
            }
            Self::Sync { sync, revision } => serde_json::to_string(&ServerFrame::Sync {
                id: id.into(),
                sync: sync.clone(),
                revision: revision.clone(),
            })
            .expect("sync frame serializes"),
            Self::Connecting => serde_json::to_string(&ServerFrame::Status {
                id: id.into(),
                state: SubscriptionStatus::Connecting,
            })
            .expect("public status serializes"),
            Self::Catalog(versions) => serde_json::to_string(&ServerFrame::Catalog {
                id: id.into(),
                versions: crate::catalog::versions(versions.clone()),
            })
            .expect("catalog frame serializes"),
            Self::Error(error) => serde_json::to_string(&ServerFrame::Error {
                id: id.into(),
                error: error.clone(),
            })
            .expect("public error serializes"),
        }
    }
    fn bytes(&self, id: &str) -> usize {
        match self {
            Self::Page {
                page,
                sync,
                revision,
            } => {
                // Serialize only the small envelope when reserving the byte budget.
                let overhead = Self::Sync {
                    sync: sync.clone(),
                    revision: revision.clone(),
                }
                .text(id)
                .len();
                overhead + page.len() + 8 // page field and page/sync tag length difference
            }
            _ => self.text(id).len(),
        }
    }
}
#[derive(Clone)]
pub(super) enum Scope {
    Subscription(Key),
    Absent(SubscriptionId),
    Global,
}
impl Scope {
    pub fn id(&self) -> &str {
        match self {
            Self::Subscription(key) => key.id.as_str(),
            Self::Absent(id) => id.as_str(),
            Self::Global => "",
        }
    }
    pub fn current(&self, registry: &Registry) -> bool {
        match self {
            Self::Subscription(key) => registry.current(key),
            Self::Absent(id) => registry.key(id).is_none(),
            Self::Global => true,
        }
    }
}
pub(super) struct Frame {
    pub scope: Scope,
    pub payload: Payload,
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
    pub async fn send(
        &self,
        scope: Scope,
        payload: Payload,
        terminal: bool,
        cancel: &CancellationToken,
    ) -> Result<(), QueueError> {
        let size = payload.bytes(scope.id());
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
                    payload,
                    terminal,
                    _bytes: permit,
                })
                .await
                .map_err(|_| QueueError::SlowConsumer)
        };
        tokio::select! {biased;_=cancel.cancelled()=>Err(QueueError::Cancelled),result=tokio::time::timeout(self.limits.write_timeout,work)=>result.unwrap_or(Err(QueueError::SlowConsumer))}
    }
    pub fn error(&self, scope: Scope, error: ApiError) -> Result<(), QueueError> {
        let payload = Payload::Error(error);
        let size = payload.bytes(scope.id());
        let permit = self
            .bytes
            .clone()
            .try_acquire_many_owned(u32::try_from(size).map_err(|_| QueueError::SlowConsumer)?)
            .map_err(|_| QueueError::SlowConsumer)?;
        self.tx
            .try_send(Frame {
                scope,
                payload,
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
            queue_bytes: 256,
            queue_messages: 2,
            write_timeout: std::time::Duration::from_millis(10),
            ..Default::default()
        };
        let (tx, mut rx) = channel(&limits);
        let stop = CancellationToken::new();
        let id = SubscriptionId::parse("offers".into()).unwrap();
        let mut registry = Registry::new(1);
        let (old, _) = registry.reserve(id.clone(), &stop).unwrap();
        let payload = Payload::test_page(Arc::from(" ".repeat(80)));
        tx.send(Scope::Subscription(old), payload.clone(), false, &stop)
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
    #[test]
    fn shared_page_envelopes_preserve_json_and_do_not_escape_subscription_ids_incorrectly() {
        let id = SubscriptionId::parse("offers._-1".into()).unwrap();
        assert!(SubscriptionId::parse("bad\"id".into()).is_err());
        let text = Payload::test_page(Arc::from("{\"data\":[]}")).text(id.as_str());
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&text).unwrap(),
            serde_json::json!({"type":"page","id":"offers._-1","page":{"data":[]},"sync":{"ready":false,"state":"starting","updated_at":null},"revision":"1"})
        );
    }
}
