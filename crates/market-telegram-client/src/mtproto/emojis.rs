//! Native inline emoji use verified alternatives; missing metadata keeps the gift fallback.
use super::{client::MtProtoActions, invocations::Group};
use crate::Reply;
use grammers_client::tl;
use market_invocations::{Priority, Replay};
use std::{collections::BTreeMap, time::Duration};

impl MtProtoActions {
    pub(super) async fn prepare_emojis(
        &self,
        reply: &Reply,
    ) -> Option<tl::enums::InputRichMessage> {
        let mut rich = super::interactions::rich(reply)?;
        if reply.emoji_documents.is_empty() {
            return Some(rich);
        }
        let lookup = self
            .documents
            .emoji_alternatives(&reply.emoji_documents, |ids| async move {
                self.invocations
                    .rpc(
                        &self.client,
                        tl::functions::messages::GetCustomEmojiDocuments { document_id: ids },
                        Group::Media,
                        Priority::Interactive,
                        Replay::Safe,
                    )
                    .await
            });
        let alternatives = match tokio::time::timeout(Duration::from_secs(3), lookup).await {
            Ok(Ok(values)) => values,
            _ => {
                tracing::warn!(
                    documents = reply.emoji_documents.len(),
                    "Inline emoji metadata unavailable; using gift icons"
                );
                BTreeMap::new()
            }
        };
        if let tl::enums::InputRichMessage::Html(html) = &mut rich {
            html.html = replace(&html.html, &reply.emoji_documents, &alternatives);
            if html.html.len() > 32768 {
                return None;
            }
        }
        Some(rich)
    }
}
fn replace(
    html: &str,
    ids: &[market_gifts::TelegramDocumentId],
    alternatives: &BTreeMap<i64, String>,
) -> String {
    let mut html = html.to_owned();
    for id in ids {
        let replacement = alternatives
            .get(&id.get())
            .map(|alt| {
                let alt = alt
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;");
                format!("<tg-emoji emoji-id=\"{}\">{alt}</tg-emoji>", id.get())
            })
            .unwrap_or_else(|| "🎁".into());
        html = html.replace(&Reply::emoji_placeholder(*id), &replacement);
    }
    html
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn alternatives_are_exact_and_missing_or_unverified_documents_use_plain_fallbacks() {
        let mut reply = Reply::default();
        let one = market_gifts::TelegramDocumentId::new(1).unwrap();
        let two = market_gifts::TelegramDocumentId::new(2).unwrap();
        let html = format!(
            "<table><tr><td>{}{}</td><td>{}</td></tr></table>",
            reply.custom_emoji(one),
            reply.custom_emoji(one),
            reply.custom_emoji(two)
        );
        assert_eq!(reply.emoji_documents.len(), 2);
        let rendered = replace(
            &html,
            &reply.emoji_documents,
            &BTreeMap::from([(1, "🌸".into())]),
        );
        assert_eq!(rendered.matches("emoji-id=\"1\">🌸").count(), 2);
        assert!(!rendered.contains("emoji-id=\"2\""));
        assert!(rendered.contains("<td>🎁</td>"));
        assert!(rendered.ends_with("</table>"));
    }
}
