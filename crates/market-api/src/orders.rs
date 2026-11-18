//! Public order presentation shared by HTTP reads and cached WebSocket window encoding.
use jiff::Timestamp;
use market_gifts::{
    Amount, CollectibleGift, GiftBook, GiftCriteria, GiftOrder, GiftOrderKind, GiftTarget, OrderId,
    Page, SyncStatus, UnsupportedGiftReason,
};
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Debug, Serialize, ToSchema)]
pub struct UnsupportedTargetReason {
    pub code: UnsupportedGiftReason,
    pub message: &'static str,
}
impl From<UnsupportedGiftReason> for UnsupportedTargetReason {
    fn from(code: UnsupportedGiftReason) -> Self {
        let message = match code {
            UnsupportedGiftReason::UnsupportedConstraints => {
                "This listing has conditions that cannot be displayed yet."
            }
            UnsupportedGiftReason::InvalidAttributes => {
                "This listing's gift details are unavailable."
            }
            UnsupportedGiftReason::NonConcreteGift => {
                "This listing does not identify a single gift with known conditions."
            }
            UnsupportedGiftReason::InvalidGiftId => "This listing's gift identity is unavailable.",
        };
        Self { code, message }
    }
}
#[derive(Debug, Serialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OrderTargetResponse<'a> {
    Gift { gift: &'a CollectibleGift },
    Criteria { criteria: &'a GiftCriteria },
    Unsupported { reason: UnsupportedTargetReason },
}
impl<'a> From<&'a GiftTarget> for OrderTargetResponse<'a> {
    fn from(value: &'a GiftTarget) -> Self {
        match value {
            GiftTarget::Gift { gift } => Self::Gift { gift },
            GiftTarget::Criteria { criteria } => Self::Criteria { criteria },
            GiftTarget::Unsupported { reason } => Self::Unsupported {
                reason: (*reason).into(),
            },
        }
    }
}

/// Stable public listing/execution fields. IDs are opaque and unique within a venue.
#[derive(Debug, Serialize, ToSchema)]
pub struct OrderResponse<'a> {
    pub id: &'a OrderId,
    pub book: &'a GiftBook,
    pub kind: GiftOrderKind,
    pub target: OrderTargetResponse<'a>,
    pub price: &'a Amount,
    /// One for a concrete gift; a positive count for criteria orders.
    #[schema(minimum = 1)]
    pub quantity: u32,
    /// When the source last observed the order, not its creation/listing time.
    #[schema(value_type = String, format = DateTime)]
    pub observed_at: Timestamp,
    #[schema(value_type = Option<String>, format = DateTime)]
    pub expires_at: Option<Timestamp>,
    #[schema(value_type = Option<String>, format = DateTime)]
    pub executed_at: Option<Timestamp>,
    /// Validated absolute HTTP(S) source link without embedded credentials; null if unavailable.
    pub market_url: Option<String>,
}
impl<'a> From<&'a GiftOrder> for OrderResponse<'a> {
    fn from(order: &'a GiftOrder) -> Self {
        Self {
            id: &order.id,
            book: &order.book,
            kind: order.kind,
            target: (&order.target).into(),
            price: &order.price,
            quantity: order.quantity,
            observed_at: order.observed_at,
            expires_at: order.expires_at,
            executed_at: order.executed_at,
            market_url: order.market_url.as_deref().and_then(market_link),
        }
    }
}
/// Paged public orders with continuation and replica freshness.
// Borrow actor data during serialization, without cloning large gift/criteria payloads.
#[derive(Debug, Serialize, ToSchema)]
pub struct OrderPageResponse<'a> {
    pub data: Vec<OrderResponse<'a>>,
    pub next_page_token: &'a str,
    pub cursor: &'a str,
    pub sync: &'a SyncStatus,
}
impl<'a> From<&'a Page<GiftOrder>> for OrderPageResponse<'a> {
    fn from(page: &'a Page<GiftOrder>) -> Self {
        Self {
            data: page.data.iter().map(Into::into).collect(),
            next_page_token: &page.next_page_token,
            cursor: &page.cursor,
            sync: &page.sync,
        }
    }
}
/// One order and its opaque, query-bound scroll position.
#[derive(Debug, Serialize, ToSchema)]
pub struct WindowOrderResponse<'a> {
    #[serde(flatten)]
    pub order: OrderResponse<'a>,
    /// Opaque, query-bound live scroll anchor; never an HTTP page token.
    pub position: &'a str,
}
#[derive(Debug, Serialize, ToSchema)]
pub struct OrderWindowResponse<'a> {
    pub data: Vec<WindowOrderResponse<'a>>,
    pub anchor: Option<&'a str>,
    pub has_before: bool,
    pub has_after: bool,
    pub cursor: &'a str,
}
impl<'a> From<&'a market_gifts::OrderWindow> for OrderWindowResponse<'a> {
    fn from(window: &'a market_gifts::OrderWindow) -> Self {
        Self {
            data: window
                .data
                .iter()
                .map(|entry| WindowOrderResponse {
                    order: (&entry.order).into(),
                    position: &entry.position,
                })
                .collect(),
            anchor: window.anchor.as_deref(),
            has_before: window.has_before,
            has_after: window.has_after,
            cursor: &window.cursor,
        }
    }
}
#[derive(Debug, Serialize, ToSchema)]
pub struct SnapshotWindowResponse<'a> {
    pub window: OrderWindowResponse<'a>,
    pub snapshot: &'a str,
    pub sync: &'a SyncStatus,
    pub revision: &'a str,
}
impl<'a> From<&'a market_gifts::SnapshotWindow> for SnapshotWindowResponse<'a> {
    fn from(value: &'a market_gifts::SnapshotWindow) -> Self {
        Self {
            window: (&value.window).into(),
            snapshot: &value.snapshot,
            sync: &value.sync,
            revision: &value.revision,
        }
    }
}
fn market_link(value: &str) -> Option<String> {
    if value.chars().any(char::is_control) {
        return None;
    }
    let url = url::Url::parse(value).ok()?;
    (matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none())
    .then(|| url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_links_are_absolute_http_links_without_credentials_or_controls() {
        for value in [
            "javascript:alert(1)",
            "data:text/html,bad",
            "tg://resolve?domain=gift",
            "//example.com/gift",
            "/gift/1",
            "not a url",
            "https://user:secret@example.com/gift",
            "https://user@example.com",
            "https://:secret@example.com",
            "https://",
            "https://example.com/a\nb",
            "https://example.com/\tgift",
        ] {
            assert!(market_link(value).is_none(), "{value}");
        }
        for (value, expected) in [
            (
                "https://t.me/market/app?startapp=gift42",
                "https://t.me/market/app?startapp=gift42",
            ),
            (
                "HTTP://EXAMPLE.COM:80/gift?q=1#details",
                "http://example.com/gift?q=1#details",
            ),
        ] {
            assert_eq!(market_link(value).as_deref(), Some(expected));
        }
    }
}
