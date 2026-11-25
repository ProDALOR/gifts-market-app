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
pub enum ReplyButton {
    Url { text: String, url: Url },
    Callback { text: String, data: Vec<u8> },
    SwitchInline { text: String, query: String },
    Preview { text: String, payload: String },
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reply {
    pub kind: ReplyKind,
    /// Lightweight fallback, also used in inline search result descriptions.
    pub text: String,
    pub title: Option<String>,
    pub summary: Option<String>,
    pub rich_html: Option<String>,
    /// Inline emoji placeholders resolved from Telegram metadata before sending.
    pub emoji_documents: Vec<market_gifts::TelegramDocumentId>,
    pub artwork: Option<market_media::Artwork>,
    pub buttons: Vec<Vec<ReplyButton>>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ReplyKind {
    #[default]
    Message,
    /// A callback notification that leaves the existing message intact.
    Notice,
}

impl Reply {
    pub fn notice(text: impl Into<String>) -> Self {
        Self {
            kind: ReplyKind::Notice,
            ..Self::text(text)
        }
    }
    pub fn custom_emoji(&mut self, document: market_gifts::TelegramDocumentId) -> String {
        if !self.emoji_documents.contains(&document) {
            self.emoji_documents.push(document);
        }
        Self::emoji_placeholder(document)
    }
    pub(crate) fn emoji_placeholder(document: market_gifts::TelegramDocumentId) -> String {
        format!("<tg-emoji emoji-id=\"{}\">🎁</tg-emoji>", document.get())
    }
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Default::default()
        }
    }
    pub fn bounded(mut self) -> Self {
        if self.kind == ReplyKind::Notice {
            self.text = self.text.chars().take(200).collect();
            self.rich_html = None;
            self.artwork = None;
            self.emoji_documents.clear();
            self.buttons.clear();
        }
        self.emoji_documents.truncate(64);
        if let Some(title) = &mut self.title {
            *title = title.chars().take(100).collect();
        }
        if let Some(summary) = &mut self.summary {
            *summary = summary.chars().take(180).collect();
        }
        // Drop oversized rich markup intact; never truncate inside HTML/entities.
        if self
            .rich_html
            .as_ref()
            .is_some_and(|html| html.len() > 32768 || html.trim().is_empty())
        {
            self.rich_html = None;
        }
        self.buttons.truncate(8);
        for row in &mut self.buttons {
            row.truncate(4);
            row.retain(|button| match button {
                ReplyButton::Callback { data, .. } => !data.is_empty() && data.len() <= 64,
                ReplyButton::Url { url, .. } => {
                    matches!(url.scheme(), "http" | "https")
                        && url.username().is_empty()
                        && url.password().is_none()
                }
                ReplyButton::Preview { payload, .. } => {
                    !payload.is_empty()
                        && payload.len() <= 512
                        && payload
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                }
                ReplyButton::SwitchInline { query, .. } => query.len() <= 256,
            });
        }
        self.buttons.retain(|row| !row.is_empty());
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn callback_notices_are_bounded_and_cannot_retain_message_content() {
        let reply = Reply {
            rich_html: Some("<h1>Old content</h1>".into()),
            emoji_documents: vec![market_gifts::TelegramDocumentId::new(1).unwrap()],
            buttons: vec![vec![ReplyButton::Callback {
                text: "Refresh".into(),
                data: b"x".to_vec(),
            }]],
            ..Reply::notice("x".repeat(250))
        }
        .bounded();
        assert_eq!(reply.kind, ReplyKind::Notice);
        assert_eq!(reply.text.len(), 200);
        assert!(reply.rich_html.is_none());
        assert!(reply.emoji_documents.is_empty());
        assert!(reply.buttons.is_empty());
    }
    #[test]
    fn invalid_buttons_and_oversized_rich_markup_fall_back_without_truncating_html() {
        let reply = Reply {
            text: "Valid fallback".into(),
            rich_html: Some(format!("<p>{}</p>", "x".repeat(32768))),
            buttons: vec![vec![
                ReplyButton::Callback {
                    text: "Bad data".into(),
                    data: vec![0; 65],
                },
                ReplyButton::Preview {
                    text: "Invalid payload".into(),
                    payload: "javascript:alert(1)".into(),
                },
                ReplyButton::Url {
                    text: "Credentials".into(),
                    url: Url::parse("https://user:secret@example.com").unwrap(),
                },
            ]],
            ..Default::default()
        }
        .bounded();
        assert_eq!(reply.text, "Valid fallback");
        assert!(reply.rich_html.is_none());
        assert!(reply.buttons.is_empty());
    }
}
