//! Reconnect-safe capability for the worker's authenticated transport. No second session.
use crate::{BotError, LaunchButton, PreparedMessage, Reply, transport::TelegramActions};
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, RwLock},
};
use tokio_util::sync::CancellationToken;
type SendReport = dyn Fn(u64, Reply) -> Pin<Box<dyn Future<Output = Result<PreparedMessage, BotError>> + Send>>
    + Send
    + Sync;
struct Live {
    send: Box<SendReport>,
    stopped: CancellationToken,
}
#[derive(Clone, Default)]
pub struct PreparedMessages(Arc<RwLock<Option<Arc<Live>>>>);
pub(crate) struct Binding {
    owner: PreparedMessages,
    live: Arc<Live>,
}
impl PreparedMessages {
    pub(crate) fn bind<T: TelegramActions + 'static>(
        &self,
        actions: T,
        launch: Option<LaunchButton>,
    ) -> Binding {
        let live = Arc::new(Live {
            stopped: CancellationToken::new(),
            send: Box::new(move |user, reply| {
                let (actions, launch) = (actions.clone(), launch.clone());
                Box::pin(
                    async move { actions.prepare_message(user, &reply, launch.as_ref()).await },
                )
            }),
        });
        let previous = self
            .0
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .replace(live.clone());
        if let Some(previous) = previous {
            previous.stopped.cancel();
        }
        Binding {
            owner: self.clone(),
            live,
        }
    }
    pub async fn prepare(&self, user: u64, reply: Reply) -> Result<PreparedMessage, BotError> {
        let live = self
            .0
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
            .ok_or(BotError::Transport)?;
        tokio::select! { biased;
            _ = live.stopped.cancelled() => Err(BotError::Transport),
            result = (live.send)(user, reply) => result,
        }
    }
}
impl Drop for Binding {
    fn drop(&mut self) {
        self.live.stopped.cancel();
        let mut current = self.owner.0.write().unwrap_or_else(|p| p.into_inner());
        if current.as_ref().is_some_and(|v| Arc::ptr_eq(v, &self.live)) {
            *current = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Clone)]
    struct Actions(bool);
    impl TelegramActions for Actions {
        async fn identity(&self) -> Result<crate::BotUser, BotError> {
            unreachable!()
        }
        async fn configure(
            &self,
            _: &[crate::MenuCommand],
            _: Option<&LaunchButton>,
        ) -> Result<(), BotError> {
            Ok(())
        }
        async fn send(
            &self,
            _: &crate::Event,
            _: Option<&Reply>,
            _: Option<&LaunchButton>,
        ) -> Result<(), BotError> {
            Ok(())
        }
        async fn prepare_message(
            &self,
            user: u64,
            reply: &Reply,
            _: Option<&LaunchButton>,
        ) -> Result<PreparedMessage, BotError> {
            if self.0 {
                std::future::pending::<()>().await;
            }
            assert_eq!(user, 7);
            assert_eq!(reply.text, "report");
            Ok(PreparedMessage {
                id: "new-generation".into(),
                expires_at: i64::MAX,
            })
        }
    }
    #[tokio::test]
    async fn reconnect_cancels_old_preparations_and_old_cleanup_cannot_clear_new_actions() {
        let messages = PreparedMessages::default();
        assert_eq!(
            messages
                .prepare(7, Reply::text("report"))
                .await
                .unwrap_err(),
            BotError::Transport
        );
        let old = messages.bind(Actions(true), None);
        let pending = tokio::spawn({
            let messages = messages.clone();
            async move { messages.prepare(7, Reply::text("report")).await }
        });
        tokio::task::yield_now().await;
        let new = messages.bind(Actions(false), None);
        assert_eq!(pending.await.unwrap().unwrap_err(), BotError::Transport);
        drop(old);
        assert_eq!(
            messages.prepare(7, Reply::text("report")).await.unwrap().id,
            "new-generation"
        );
        drop(new);
        assert_eq!(
            messages
                .prepare(7, Reply::text("report"))
                .await
                .unwrap_err(),
            BotError::Transport
        );
    }
}
