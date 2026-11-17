//! Native application-facing contracts. Grammers delivery types remain private.
use crate::{BotError, GiftSource};
use std::future::Future;
use url::Url;

#[derive(Debug, Clone, Copy)]
pub struct IncomingMessage<'a> {
    pub private: bool,
    pub outgoing: bool,
    pub user_id: Option<u64>,
    pub message_id: i32,
    pub text: Option<&'a str>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuCommand {
    pub name: &'static str,
    pub description: &'static str,
}
#[derive(Debug, Clone)]
pub struct LaunchButton {
    pub text: String,
    pub url: Url,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub text: String,
}
impl Reply {
    pub(crate) fn bounded(mut self) -> Self {
        if self.text.trim().is_empty() {
            self.text = "This command produced no result.".into();
        }
        if self.text.encode_utf16().count() > 4096 {
            let mut units = 0;
            self.text = self
                .text
                .chars()
                .take_while(|ch| {
                    units += ch.len_utf16();
                    units <= 4095
                })
                .collect();
            self.text.push('…');
        }
        self
    }
}
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
    fn handle<S: GiftSource>(
        &self,
        request: Self::Request,
        gifts: &S,
    ) -> impl Future<Output = Result<Reply, BotError>> + Send;
}
