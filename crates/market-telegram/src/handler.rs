//! Application command handler, independent of native RPC execution.
use crate::{BotError, IncomingMessage, MenuCommand, Reply};
use std::future::Future;
/// Parse before scheduling; opaque owned requests survive retries without retaining MTProto data.
pub trait MessageHandler: Send + Sync {
    type Request: Clone + Send;
    fn commands(&self) -> Vec<MenuCommand> {
        vec![]
    }
    fn launch_label(&self) -> &'static str {
        "Open app"
    }
    fn parse(&self, message: IncomingMessage<'_>, usernames: &[String]) -> Option<Self::Request>;
    fn parse_event(&self, event: &crate::Event, usernames: &[String]) -> Option<Self::Request> {
        if let crate::Event::Message(message) = event {
            self.parse(crate::transport::incoming(message), usernames)
        } else {
            None
        }
    }
    fn handle_with_progress(
        &self,
        request: Self::Request,
        _updates: tokio::sync::watch::Sender<Reply>,
    ) -> impl Future<Output = Result<Reply, BotError>> + Send {
        self.handle(request)
    }
    fn handle(
        &self,
        request: Self::Request,
    ) -> impl Future<Output = Result<Reply, BotError>> + Send;
}
