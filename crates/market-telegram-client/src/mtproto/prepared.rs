//! Prepared reports use the same rich-result encoder, image cache and RPC scheduler as inline.
use super::{client::MtProtoActions, interactions::inline_result, invocations::Group};
use crate::{BotError, LaunchButton, PreparedMessage, Reply};
use grammers_client::tl;
use market_invocations::{Priority, Replay};

impl MtProtoActions {
    pub(super) async fn save_prepared(
        &self,
        user: u64,
        reply: &Reply,
        launch: Option<&LaunchButton>,
    ) -> Result<PreparedMessage, BotError> {
        let user = i64::try_from(user)
            .ok()
            .filter(|id| *id > 0)
            .ok_or(BotError::InvalidResponse)?;
        let profile = self.profile.get().ok_or(BotError::Transport)?;
        let reply = reply.clone().bounded();
        let rich = self.prepare_rich(Some(&reply)).await;
        let result = inline_result(
            &reply,
            Some(&profile.username),
            launch,
            rich,
            profile.inline_enabled,
        );
        let response = self
            .invocations
            .rpc(
                &self.client,
                tl::functions::messages::SavePreparedInlineMessage {
                    result,
                    // Bot-authorized user IDs do not require resolving a username or access hash.
                    user_id: tl::types::InputUser {
                        user_id: user,
                        access_hash: 0,
                    }
                    .into(),
                    peer_types: Some(vec![
                        tl::types::InlineQueryPeerTypePm {}.into(),
                        tl::types::InlineQueryPeerTypeChat {}.into(),
                        tl::types::InlineQueryPeerTypeMegagroup {}.into(),
                        tl::types::InlineQueryPeerTypeBroadcast {}.into(),
                    ]),
                },
                Group::Inline,
                Priority::Interactive,
                Replay::Safe,
            )
            .await?;
        let tl::enums::messages::BotPreparedInlineMessage::Message(response) = response;
        if response.id.is_empty() || i64::from(response.expire_date) <= unix_now() {
            return Err(BotError::InvalidResponse);
        }
        Ok(PreparedMessage {
            id: response.id,
            expires_at: response.expire_date.into(),
        })
    }
}
fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::{TelegramActions, Transport};
    /// Creates an expiring prepared ID, never sends a message or opens a fresh bot session.
    #[tokio::test]
    #[ignore = "requires explicit credentials, recipient and an existing bot session"]
    async fn prepare_live_rich_report_without_sending() {
        let credentials = crate::BotCredentials::new(
            std::env::var("TELEGRAM_BOT_TOKEN").unwrap(),
            std::env::var("TELEGRAM_API_ID").unwrap().parse().unwrap(),
            std::env::var("TELEGRAM_API_HASH").unwrap(),
        )
        .unwrap();
        assert_eq!(credentials.bot_id(), 395637519, "authorized test bot only");
        let user: u64 = std::env::var("MARKET_TEST_RECIPIENT")
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(user, 361959653);
        let dir = std::path::PathBuf::from(std::env::var("MARKET_TEST_SESSION_DIR").unwrap());
        assert!(
            dir.is_absolute()
                && dir
                    .join(format!(
                        "{}-{}.mtproto.sqlite",
                        credentials.bot_id(),
                        credentials.api_id
                    ))
                    .is_file(),
            "reuse an existing saved session"
        );
        let scheduler = market_invocations::Runtime::start(Default::default()).unwrap();
        let connection = crate::Connection::open(credentials, &dir, scheduler.handle())
            .await
            .unwrap();
        let actions = connection.transport.actions();
        let result = async {
            actions.identity().await?;
            let reply = Reply { title: Some("Gift report".into()), rich_html: Some("<h3>Gift report</h3><table compact><tr><td>Model</td><td>11.4</td></tr></table>".into()), ..Reply::text("Gift report · Model 11.4 GRAM") };
            actions.prepare_message(user, &reply, None).await
        }.await;
        let closed = connection.close().await;
        scheduler.shutdown().await;
        closed.unwrap();
        let prepared = result.unwrap();
        assert!(!prepared.id.is_empty());
        assert!(prepared.expires_at > unix_now());
    }
}
