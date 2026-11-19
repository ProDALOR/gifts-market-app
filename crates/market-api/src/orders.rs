//! Public order presentation shared by HTTP reads and cached WebSocket window encoding.
use jiff::Timestamp;
use market_gifts::{
    Amount, BackdropId, GiftBook, GiftId, GiftNumber, GiftNumberRange, GiftOrder, GiftOrderKind,
    GiftTarget, GiftTraits, ModelId, OrderId, PatternId, UnsupportedGiftReason,
};
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Debug, Serialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OrderTargetResponse<'a> {
    Gift {
        id: &'a GiftId,
        #[serde(skip_serializing_if = "Option::is_none")]
        number: Option<GiftNumber>,
        traits: &'a GiftTraits,
    },
    Criteria {
        models: &'a [ModelId],
        backdrops: &'a [BackdropId],
        patterns: &'a [PatternId],
        #[serde(skip_serializing_if = "Option::is_none")]
        number: Option<&'a GiftNumberRange>,
    },
    Unsupported {
        code: UnsupportedGiftReason,
        message: &'static str,
    },
}
impl<'a> From<&'a GiftTarget> for OrderTargetResponse<'a> {
    fn from(value: &'a GiftTarget) -> Self {
        match value {
            GiftTarget::Gift { gift } => Self::Gift {
                id: &gift.id,
                number: gift.number,
                traits: &gift.traits,
            },
            GiftTarget::Criteria { criteria } => Self::Criteria {
                models: &criteria.models,
                backdrops: &criteria.backdrops,
                patterns: &criteria.patterns,
                number: criteria.number.as_ref(),
            },
            GiftTarget::Unsupported { reason } => Self::Unsupported {
                code: *reason,
                message: match reason {
                    UnsupportedGiftReason::UnsupportedConstraints => {
                        "This listing has conditions that cannot be displayed yet."
                    }
                    UnsupportedGiftReason::InvalidAttributes => {
                        "This listing's gift details are unavailable."
                    }
                    UnsupportedGiftReason::NonConcreteGift => {
                        "This listing does not identify a single gift with known conditions."
                    }
                    UnsupportedGiftReason::InvalidGiftId => {
                        "This listing's gift identity is unavailable."
                    }
                },
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
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub expires_at: Option<Timestamp>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub executed_at: Option<Timestamp>,
    /// Validated absolute HTTP(S) source link without embedded credentials; omitted if unavailable.
    #[serde(skip_serializing_if = "Option::is_none")]
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
/// One opaque key provides both ordering and navigation within its query.
#[derive(Debug, Serialize, ToSchema)]
pub struct OrderedOrderResponse<'a> {
    #[serde(flatten)]
    pub order: OrderResponse<'a>,
    #[schema(value_type=market_gifts::OrderKey)]
    pub key: &'a market_actors::ReadPosition,
}
impl<'a> From<&'a market_actors::OrderedEntry> for OrderedOrderResponse<'a> {
    fn from(row: &'a market_actors::OrderedEntry) -> Self {
        Self {
            order: (&row.order).into(),
            key: &row.key,
        }
    }
}
#[derive(Debug, Serialize)]
pub struct OrderRangeResponse<'a> {
    pub start: Option<&'a market_actors::ReadPosition>,
    pub end: Option<&'a market_actors::ReadPosition>,
}
#[derive(Debug, Serialize, ToSchema)]
pub struct OrderReadResponse<'a> {
    #[serde(flatten)]
    pub orders: OrderSnapshotResponse<'a>,
    #[schema(value_type=Option<market_gifts::OrderKey>, required=true)]
    pub next: Option<&'a market_actors::ReadPosition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type=Option<market_gifts::ReadSnapshotId>)]
    pub snapshot: Option<String>,
}
/// Complete live coverage, without finite-query continuation or snapshot leases.
#[derive(Debug, Serialize, ToSchema)]
pub struct OrderSnapshotResponse<'a> {
    pub data: Vec<OrderedOrderResponse<'a>>,
    #[schema(value_type=market_gifts::OrderRange)]
    pub range: OrderRangeResponse<'a>,
}
impl<'a> From<&'a market_actors::OrderResult> for OrderSnapshotResponse<'a> {
    fn from(result: &'a market_actors::OrderResult) -> Self {
        Self {
            data: result.data.iter().map(Into::into).collect(),
            range: OrderRangeResponse {
                start: result.range.start.as_ref(),
                end: result.range.end.as_ref(),
            },
        }
    }
}
impl<'a> From<&'a market_actors::OrderRead> for OrderReadResponse<'a> {
    fn from(read: &'a market_actors::OrderRead) -> Self {
        Self {
            orders: (&read.result).into(),
            next: read.next.as_ref(),
            snapshot: read.snapshot.map(|id| id.to_string()),
        }
    }
}
#[derive(Debug, Serialize, ToSchema)]
pub struct OrderIdentity<'a> {
    pub venue_id: &'a market_gifts::VenueId,
    pub id: &'a OrderId,
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
    use market_gifts::GiftCriteria;
    use serde_json::json;

    #[test]
    fn target_schemas_describe_flat_payloads_without_strict_allof_wrappers() {
        let schema =
            serde_json::to_value(<OrderTargetResponse<'_> as utoipa::PartialSchema>::schema())
                .unwrap();
        let variants = schema["oneOf"].as_array().unwrap();
        for (variant, (kind, fields, required)) in variants.iter().zip([
            (
                "gift",
                vec!["id", "kind", "number", "traits"],
                vec!["id", "traits", "kind"],
            ),
            (
                "criteria",
                vec!["backdrops", "kind", "models", "number", "patterns"],
                vec!["models", "backdrops", "patterns", "kind"],
            ),
            (
                "unsupported",
                vec!["code", "kind", "message"],
                vec!["code", "message", "kind"],
            ),
        ]) {
            assert_eq!(variant["properties"]["kind"]["enum"], json!([kind]));
            assert_eq!(
                variant["properties"]
                    .as_object()
                    .unwrap()
                    .keys()
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
                fields
            );
            assert_eq!(variant["required"], json!(required));
            assert!(variant.get("allOf").is_none());
        }
        let schema =
            serde_json::to_value(<OrderResponse<'_> as utoipa::PartialSchema>::schema()).unwrap();
        let required = schema["required"].as_array().unwrap();
        for field in ["expires_at", "executed_at", "market_url"] {
            assert!(schema["properties"].get(field).is_some());
            assert!(!required.contains(&json!(field)));
        }
    }

    #[test]
    fn targets_are_flat_and_order_gifts_use_the_books_collection() {
        let gift = market_gifts::CollectibleGift {
            id: "gift".into(),
            collection_id: "collection".into(),
            number: Some(GiftNumber::new(42).unwrap()),
            traits: GiftTraits {
                model: Some("model".into()),
                ..Default::default()
            },
        };
        for (target, expected) in [
            (
                GiftTarget::Gift { gift },
                json!({"kind":"gift","id":"gift","number":42,"traits":{"model":"model"}}),
            ),
            (
                GiftTarget::Criteria {
                    criteria: GiftCriteria::default(),
                },
                json!({"kind":"criteria","models":[],"backdrops":[],"patterns":[]}),
            ),
            (
                GiftTarget::Unsupported {
                    reason: UnsupportedGiftReason::InvalidGiftId,
                },
                json!({"kind":"unsupported","code":"invalid_gift_id","message":"This listing's gift identity is unavailable."}),
            ),
        ] {
            assert_eq!(
                serde_json::to_value(OrderTargetResponse::from(&target)).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn absent_description_is_omitted_but_present_links_and_execution_times_remain() {
        let mut draft = market_gifts::GiftOrderData {
            id: "order".into(),
            book: GiftBook {
                venue_id: "venue".into(),
                collection_id: "collection".into(),
                currency_id: "GRAM".into(),
            },
            kind: GiftOrderKind::Ask,
            target: GiftTarget::Gift {
                gift: market_gifts::CollectibleGift {
                    id: "gift".into(),
                    collection_id: "collection".into(),
                    number: None,
                    traits: GiftTraits::default(),
                },
            },
            price: "1.25".into(),
            quantity: 1,
            observed_at: Timestamp::UNIX_EPOCH,
            expires_at: None,
            executed_at: None,
            market_url: None,
        };
        let order = GiftOrder::try_from(draft.clone()).unwrap();
        let value = serde_json::to_value(OrderResponse::from(&order)).unwrap();
        assert_eq!(
            value["target"],
            json!({"kind":"gift","id":"gift","traits":{}})
        );
        for field in ["expires_at", "executed_at", "market_url"] {
            assert!(value.get(field).is_none(), "{field}");
        }
        draft.kind = GiftOrderKind::Executed;
        draft.executed_at = Some(Timestamp::UNIX_EPOCH);
        draft.market_url = Some("https://example.com/gift".into());
        let order = GiftOrder::try_from(draft).unwrap();
        let value = serde_json::to_value(OrderResponse::from(&order)).unwrap();
        assert_eq!(value["executed_at"], "1970-01-01T00:00:00Z");
        assert_eq!(value["market_url"], "https://example.com/gift");
    }

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
