//! Bot presentation icons. Custom emoji IDs are independent of gift media document IDs.
use market_gifts::{InvalidValue, TelegramDocumentId};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReportIcon {
    Collection,
    Model,
    Backdrop,
    ModelBackdrop,
    Symbol,
    Estimate,
    History,
    Gram,
    Stars,
    Warning,
}
impl ReportIcon {
    fn fallback(self) -> &'static str {
        match self {
            Self::Collection => "🎁",
            Self::Model => "🧸",
            Self::Backdrop => "🎨",
            Self::ModelBackdrop => "🧩",
            Self::Symbol => "✨",
            Self::Estimate => "💰",
            Self::History => "🕘",
            Self::Gram => "🔻",
            Self::Stars => "⭐",
            Self::Warning => "⚠️",
        }
    }
}

#[derive(Debug, Clone)]
struct CustomEmoji {
    id: TelegramDocumentId,
    alternative: String,
}

/// Shared by all report entry points. Uses a verified TON emoji and ordinary emoji for other roles.
#[derive(Debug, Clone)]
pub struct ReportIcons(BTreeMap<ReportIcon, CustomEmoji>);
impl Default for ReportIcons {
    fn default() -> Self {
        // Verified cyan TON outline from https://t.me/addemoji/Toncoin_emoji.
        Self(BTreeMap::from([(
            ReportIcon::Gram,
            CustomEmoji {
                id: TelegramDocumentId::new(5224544724483842608).expect("known TON custom emoji"),
                alternative: "🔻".into(),
            },
        )]))
    }
}

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
            .map_or(icon.fallback(), |emoji| &emoji.alternative)
    }
    pub(crate) fn html(&self, icon: ReportIcon) -> String {
        let alternative = escape(self.text(icon));
        match self.0.get(&icon) {
            Some(emoji) => format!(
                "<tg-emoji emoji-id=\"{}\">{alternative}</tg-emoji>",
                emoji.id.get()
            ),
            None => alternative,
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
    fn gram_uses_the_verified_ton_outline_without_a_diamond_fallback() {
        let icons = ReportIcons::default();
        let html = icons.currency_html("GRAM");
        assert!(html.contains("emoji-id=\"5224544724483842608\""));
        assert!(html.ends_with("</tg-emoji> GRAM"));
        assert!(!html.contains('💎'));
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
        assert_eq!(icons.html(ReportIcon::Backdrop), "🎨");
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
