//! Named bot icons. Raw document IDs require custom-emoji metadata verification.
use market_gifts::{InvalidValue, TelegramDocumentId};
pub use market_media::UiIcon as ReportIcon;
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
struct CustomEmoji {
    id: TelegramDocumentId,
    alternative: String,
}

/// Shared defaults come from market-media; per-report gift emoji may override UI roles.
#[derive(Debug, Clone, Default)]
pub struct ReportIcons(BTreeMap<ReportIcon, CustomEmoji>);

impl ReportIcons {
    /// Use a verified custom emoji document and its exact Telegram `alt` emoji.
    /// Eligibility to send custom emoji must be enabled for the bot by Telegram.
    pub fn with_custom(
        mut self,
        icon: ReportIcon,
        id: TelegramDocumentId,
        alternative: impl Into<String>,
    ) -> Result<Self, InvalidValue> {
        let alternative = alternative.into();
        if alternative.trim().is_empty()
            || alternative.chars().count() > 32
            || alternative.chars().any(char::is_control)
        {
            return Err(InvalidValue(
                "custom emoji requires a short, non-empty alternative emoji",
            ));
        }
        self.0.insert(icon, CustomEmoji { id, alternative });
        Ok(self)
    }
    pub(crate) fn text(&self, icon: ReportIcon) -> &str {
        self.0
            .get(&icon)
            .map_or(icon.definition().alternative, |emoji| &emoji.alternative)
    }
    pub(crate) fn html(&self, icon: ReportIcon) -> String {
        let alternative = escape(self.text(icon));
        match self.0.get(&icon) {
            Some(emoji) => format!(
                "<tg-emoji emoji-id=\"{}\">{alternative}</tg-emoji>",
                emoji.id.get()
            ),
            None => format!(
                "<tg-emoji emoji-id=\"{}\">{alternative}</tg-emoji>",
                icon.definition().document.get()
            ),
        }
    }
    pub(crate) fn currency_html(&self, currency: &str) -> String {
        match Self::currency(currency) {
            Some(icon) => format!("{} {}", self.html(icon), escape(currency)),
            None => escape(currency),
        }
    }
    pub(crate) fn currency(currency: &str) -> Option<ReportIcon> {
        match currency {
            "GRAM" => Some(ReportIcon::Gram),
            "XTR" => Some(ReportIcon::Stars),
            _ => None,
        }
    }
}

pub(crate) fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bot_defaults_use_shared_documents_and_exact_alternatives() {
        let icons = ReportIcons::default();
        for &icon in ReportIcon::ALL {
            let definition = icon.definition();
            assert_eq!(icons.text(icon), definition.alternative);
            assert_eq!(
                icons.html(icon),
                format!(
                    "<tg-emoji emoji-id=\"{}\">{}</tg-emoji>",
                    definition.document.get(),
                    definition.alternative
                )
            );
        }
    }

    /// Download/render curated icons with production admission, disk caching and Grammers.
    #[tokio::test]
    #[ignore = "requires explicit bot credentials; downloads only allowlisted shared UI icons"]
    async fn cache_live_shared_icons() {
        use market_telegram_client::{
            BotCredentials, Connection,
            transport::{TelegramActions, Transport},
        };
        let credentials = BotCredentials::new(
            std::env::var("TELEGRAM_BOT_TOKEN").unwrap(),
            std::env::var("TELEGRAM_API_ID").unwrap().parse().unwrap(),
            std::env::var("TELEGRAM_API_HASH").unwrap(),
        )
        .unwrap();
        let expected: u64 = std::env::var("MARKET_TEST_BOT_ID")
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(credentials.bot_id(), expected);
        let session = std::path::PathBuf::from(
            std::env::var("MARKET_TEST_SESSION_DIR")
                .expect("reuse an authorized session directory for live previews"),
        );
        let scheduler = market_invocations::Runtime::start(Default::default()).unwrap();
        let connection = Connection::open(credentials, &session, scheduler.handle())
            .await
            .unwrap();
        let actions = connection.transport.actions();
        assert_eq!(actions.identity().await.unwrap().id, expected);
        let (cache, mut downloads) =
            market_media::MediaCache::open(std::env::var("MARKET_TEST_MEDIA_DIR").unwrap())
                .await
                .unwrap();
        let worker = tokio::spawn(async move {
            downloads.run(|key| actions.media(key)).await;
        });
        let mut failed = vec![];
        for &icon in ReportIcon::ALL {
            match cache.preview(icon.definition().document).await {
                Ok(svg) => {
                    if matches!(icon, ReportIcon::Info | ReportIcon::Close) {
                        assert!(
                            std::str::from_utf8(svg.bytes())
                                .unwrap()
                                .contains("fill-rule=\"evenodd\""),
                            "build the current market-media-render next to the test executable before this live check"
                        );
                    }
                    // Rendering and validation succeeded; reject empty first frames too.
                    if !std::str::from_utf8(svg.bytes()).unwrap().contains("<path") {
                        failed.push(icon.name());
                    }
                    eprintln!(
                        "{}: {} bytes ({})",
                        icon.name(),
                        svg.bytes().len(),
                        svg.content_type()
                    );
                }
                Err(error) => {
                    eprintln!("{}: {error:?}", icon.name());
                    failed.push(icon.name());
                }
            }
        }
        cache.shutdown().await;
        worker.await.unwrap();
        connection.close().await.unwrap();
        scheduler.shutdown().await;
        assert!(failed.is_empty(), "failed static icons: {failed:?}");
    }
    /// Compare explicitly selected public emoji without changing the shared registry.
    #[tokio::test]
    #[ignore = "sends one icon comparison; requires explicit bot, recipient, authorized session and candidate IDs"]
    async fn send_live_icon_comparison() {
        use grammers_session::types::{PeerAuth, PeerId, PeerRef};
        use market_telegram_client::{
            BotCredentials, Connection, Reply,
            interactions::{CallbackTarget, Event},
            transport::{Message, TelegramActions, Transport},
        };
        let credentials = BotCredentials::new(
            std::env::var("TELEGRAM_BOT_TOKEN").unwrap(),
            std::env::var("TELEGRAM_API_ID").unwrap().parse().unwrap(),
            std::env::var("TELEGRAM_API_HASH").unwrap(),
        )
        .unwrap();
        let expected: u64 = std::env::var("MARKET_TEST_BOT_ID")
            .unwrap()
            .parse()
            .unwrap();
        let recipient: u64 = std::env::var("MARKET_TEST_RECIPIENT")
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(credentials.bot_id(), expected);
        let candidates = std::env::var("MARKET_TEST_ICON_CANDIDATES")
            .unwrap()
            .split(',')
            .map(|value| market_media::DocumentId::new(value.parse().unwrap()).unwrap())
            .collect::<Vec<_>>();
        assert!((1..=3).contains(&candidates.len()));
        let session = std::env::var("MARKET_TEST_SESSION_DIR").unwrap();
        let scheduler = market_invocations::Runtime::start(Default::default()).unwrap();
        let connection = Connection::open(
            credentials,
            std::path::Path::new(&session),
            scheduler.handle(),
        )
        .await
        .unwrap();
        let actions = connection.transport.actions();
        assert_eq!(actions.identity().await.unwrap().id, expected);
        let (cache, mut downloads) =
            market_media::MediaCache::open(std::env::var("MARKET_TEST_MEDIA_DIR").unwrap())
                .await
                .unwrap();
        let media = actions.clone();
        let worker = tokio::spawn(async move {
            downloads.run(|key| media.media(key)).await;
        });
        let output =
            std::path::PathBuf::from(std::env::var("MARKET_TEST_ICON_PREVIEW_DIR").unwrap());
        std::fs::create_dir_all(&output).unwrap();
        let mut reply = Reply::default();
        let mut html = "<h3>GRAM</h3><table compact><tr><th>Current</th>".to_owned();
        for i in 1..=candidates.len() {
            html.push_str(&format!("<th>{i}</th>"));
        }
        html.push_str("</tr><tr>");
        for document in std::iter::once(ReportIcon::Gram.definition().document)
            .chain(candidates.iter().copied())
        {
            let svg = cache.preview(document).await.unwrap();
            std::fs::write(output.join(format!("{}.svg", document.get())), svg.bytes()).unwrap();
            let emoji = reply.custom_emoji(TelegramDocumentId::new(document.get()).unwrap());
            html.push_str(&format!("<td>{emoji} <b>100</b></td>"));
        }
        html.push_str("</tr></table>");
        reply.text = "GRAM icon comparison".into();
        for (i, document) in candidates.iter().enumerate() {
            html.push_str(&format!("<footer>{}: {}</footer>", i + 1, document.get()));
            reply
                .text
                .push_str(&format!("\n{}: {}", i + 1, document.get()));
        }
        reply.rich_html = Some(html);
        let event = Event::Message(Message {
            peer: PeerRef {
                id: PeerId::user(recipient as i64).unwrap(),
                auth: PeerAuth::from_hash(0),
            },
            id: (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as i32
                & i32::MAX)
                .max(1),
            outgoing: false,
            text: String::new(),
        });
        let sent = actions.send_progress(&event, &reply.bounded(), None).await;
        cache.shutdown().await;
        worker.await.unwrap();
        connection.close().await.unwrap();
        scheduler.shutdown().await;
        let Some(Event::Callback {
            target: CallbackTarget::Message(sent),
            ..
        }) = sent.unwrap()
        else {
            panic!("missing Telegram message receipt")
        };
        eprintln!(
            "Sent icon comparison to authorized recipient {recipient}; Telegram message_id={}",
            sent.id
        );
    }
    #[test]
    fn gram_uses_the_tgiosicons_document_and_its_exact_alternative() {
        let icons = ReportIcons::default();
        let html = icons.currency_html("GRAM");
        let definition = ReportIcon::Gram.definition();
        assert_eq!(definition.document.get(), 5935758241748230351);
        assert!(html.contains(&format!(
            "emoji-id=\"{}\">{}</tg-emoji>",
            definition.document.get(),
            definition.alternative
        )));
        assert!(html.ends_with("</tg-emoji> GRAM"));
        assert!(!html.contains("5224544724483842608"));
        assert_eq!(icons.currency_html("USD"), "USD");
    }
    #[test]
    fn custom_emoji_keeps_plain_alternative_and_escapes_html() {
        let icons = ReportIcons::default()
            .with_custom(
                ReportIcon::Model,
                TelegramDocumentId::new(123).unwrap(),
                "<&\"",
            )
            .unwrap();
        assert_eq!(icons.text(ReportIcon::Model), "<&\"");
        assert_eq!(
            icons.html(ReportIcon::Model),
            "<tg-emoji emoji-id=\"123\">&lt;&amp;&quot;</tg-emoji>"
        );
        let backdrop = ReportIcon::Backdrop.definition();
        assert_eq!(
            icons.html(ReportIcon::Backdrop),
            format!(
                "<tg-emoji emoji-id=\"{}\">{}</tg-emoji>",
                backdrop.document.get(),
                backdrop.alternative
            )
        );
        for alternative in ["", " ", "\n", &"a".repeat(33)] {
            assert!(
                ReportIcons::default()
                    .with_custom(
                        ReportIcon::Model,
                        TelegramDocumentId::new(123).unwrap(),
                        alternative
                    )
                    .is_err()
            );
        }
        assert_eq!(ReportIcons::currency("XTR"), Some(ReportIcon::Stars));
        assert_eq!(ReportIcons::currency("OTHER"), None);
    }
}
