use super::{BACKDROP, MODEL, NUMBER, PATTERN, Result, invalid};
use jiff::Timestamp;
use market_gifts::*;
use market_hub_protocol::wire as w;
use std::collections::BTreeMap;
fn required<T>(value: Option<T>) -> Result<T> {
    value.ok_or_else(|| invalid("missing gift market field"))
}
fn time(value: i64) -> Result<Timestamp> {
    Timestamp::from_millisecond(value).map_err(invalid)
}
pub fn book(value: w::BookKey) -> Result<GiftBook> {
    Ok(GiftBook {
        venue_id: value.venue_id.try_into().map_err(invalid)?,
        collection_id: value.item_type_id.try_into().map_err(invalid)?,
        currency_id: value.currency_id.try_into().map_err(invalid)?,
    })
}
type TargetResult<T> = std::result::Result<T, UnsupportedGiftReason>;
fn required_option(value: &w::AttributeValue) -> TargetResult<&str> {
    match value.value.as_ref() {
        Some(w::attribute_value::Value::OptionId(id)) => Ok(id),
        _ => Err(UnsupportedGiftReason::InvalidAttributes),
    }
}
fn option_values(value: &w::AttributePredicate) -> TargetResult<&[w::AttributeValue]> {
    use w::attribute_predicate::Predicate as P;
    match value
        .predicate
        .as_ref()
        .ok_or(UnsupportedGiftReason::InvalidAttributes)?
    {
        P::Eq(value) => Ok(std::slice::from_ref(value)),
        P::OneOf(values) if !values.values.is_empty() => Ok(&values.values),
        P::OneOf(_) => Err(UnsupportedGiftReason::InvalidAttributes),
        _ => Err(UnsupportedGiftReason::UnsupportedConstraints),
    }
}
fn options<T: TryFrom<String>>(value: w::AttributePredicate) -> TargetResult<Vec<T>> {
    let mut ids = option_values(&value)?
        .iter()
        .map(required_option)
        .collect::<TargetResult<Vec<_>>>()?;
    ids.sort();
    ids.dedup();
    ids.into_iter()
        .map(|id| {
            id.to_owned()
                .try_into()
                .map_err(|_| UnsupportedGiftReason::InvalidAttributes)
        })
        .collect()
}
fn number(value: w::AttributeValue) -> TargetResult<GiftNumber> {
    match value
        .value
        .ok_or(UnsupportedGiftReason::InvalidAttributes)?
    {
        w::attribute_value::Value::Integer(value) => u32::try_from(value)
            .map_err(|_| UnsupportedGiftReason::InvalidAttributes)
            .and_then(|n| GiftNumber::new(n).map_err(|_| UnsupportedGiftReason::InvalidAttributes)),
        _ => Err(UnsupportedGiftReason::InvalidAttributes),
    }
}
fn number_range(value: w::AttributePredicate) -> TargetResult<GiftNumberRange> {
    use w::attribute_predicate::Predicate as P;
    let range = match value
        .predicate
        .ok_or(UnsupportedGiftReason::InvalidAttributes)?
    {
        P::Eq(value) => {
            let n = number(value)?;
            GiftNumberRange {
                min: Some(n),
                max: Some(n),
            }
        }
        P::Range(value) => GiftNumberRange {
            min: value.min.map(number).transpose()?,
            max: value.max.map(number).transpose()?,
        },
        _ => return Err(UnsupportedGiftReason::UnsupportedConstraints),
    };
    range
        .validate()
        .map_err(|_| UnsupportedGiftReason::InvalidAttributes)?;
    Ok(range)
}

/// Apply the same quantity and trait rules as order projection to an active ask.
fn active_floor_order(value: w::Order, now_ms: i64) -> Result<Option<GiftOrder>> {
    if value.mode != w::OrderMode::Ask as i32
        || value.expires_at_ms.is_some_and(|expiry| expiry <= now_ms)
    {
        return Ok(None);
    }
    // Exhausted listings remain in generic snapshots but cannot supply a floor.
    if rust_decimal::Decimal::from_str_exact(&value.quantity).map_err(invalid)?
        <= rust_decimal::Decimal::ZERO
    {
        return Ok(None);
    }
    let order = order(value)?;
    if order.price.decimal() <= rust_decimal::Decimal::ZERO {
        return Ok(None);
    }
    if matches!(order.target, GiftTarget::Unsupported { .. }) {
        return Ok(None);
    }
    Ok(Some(order))
}

pub fn ask_quote(value: w::Order, now_ms: i64) -> Result<Option<AskQuote>> {
    let Some(order) = active_floor_order(value, now_ms)? else {
        return Ok(None);
    };
    let order = order.into_data();
    let (model_id, backdrop_id) = match order.target {
        GiftTarget::Gift { gift } => (gift.traits.model, gift.traits.backdrop),
        GiftTarget::Criteria { mut criteria } => (
            (criteria.models.len() == 1)
                .then(|| criteria.models.pop())
                .flatten(),
            (criteria.backdrops.len() == 1)
                .then(|| criteria.backdrops.pop())
                .flatten(),
        ),
        GiftTarget::Unsupported { .. } => return Ok(None),
    };
    Ok(Some(AskQuote {
        venue_id: order.book.venue_id,
        price: order.price,
        model_id,
        backdrop_id,
        expires_at: order.expires_at,
    }))
}

fn criteria(attributes: BTreeMap<String, w::AttributePredicate>) -> TargetResult<GiftCriteria> {
    let mut result = GiftCriteria::default();
    for (key, value) in attributes {
        match key.as_str() {
            MODEL => result.models = options(value)?,
            BACKDROP => result.backdrops = options(value)?,
            PATTERN => result.patterns = options(value)?,
            NUMBER => result.number = Some(number_range(value)?),
            _ => return Err(UnsupportedGiftReason::UnsupportedConstraints),
        }
    }
    Ok(result)
}
fn target(
    id: Option<String>,
    collection_id: CollectionId,
    attributes: BTreeMap<String, w::AttributePredicate>,
) -> GiftTarget {
    let criteria = match criteria(attributes) {
        Ok(value) => value,
        Err(reason) => return GiftTarget::Unsupported { reason },
    };
    match id {
        None => GiftTarget::Criteria { criteria },
        Some(id) => {
            let number = criteria
                .number
                .as_ref()
                .and_then(|range| range.min.zip(range.max))
                .filter(|(min, max)| min == max)
                .map(|(min, _)| min);
            if criteria.models.len() > 1
                || criteria.backdrops.len() > 1
                || criteria.patterns.len() > 1
                || (criteria.number.is_some() && number.is_none())
            {
                return GiftTarget::Unsupported {
                    reason: UnsupportedGiftReason::NonConcreteGift,
                };
            }
            match id.try_into() {
                Ok(id) => GiftTarget::Gift {
                    gift: CollectibleGift {
                        id,
                        collection_id,
                        number,
                        traits: GiftTraits {
                            model: criteria.models.into_iter().next(),
                            backdrop: criteria.backdrops.into_iter().next(),
                            pattern: criteria.patterns.into_iter().next(),
                        },
                    },
                },
                Err(_) => GiftTarget::Unsupported {
                    reason: UnsupportedGiftReason::InvalidGiftId,
                },
            }
        }
    }
}
/// Exhausted active orders remain generic replica facts but are absent from gift reads.
/// History quantity zero and negative quantities remain invalid rather than disappearing.
pub fn is_exhausted(value: &w::Order) -> Result<bool> {
    let quantity = rust_decimal::Decimal::from_str_exact(&value.quantity).map_err(invalid)?;
    Ok(matches!(
        w::OrderMode::try_from(value.mode),
        Ok(w::OrderMode::Ask | w::OrderMode::Bid)
    ) && quantity == rust_decimal::Decimal::ZERO)
}
pub fn order(value: w::Order) -> Result<GiftOrder> {
    order_ref(&value)
}
/// Borrow raw replica data and copy only gift fields retained by the projection.
pub fn order_ref(value: &w::Order) -> Result<GiftOrder> {
    if value.mode != w::OrderMode::Executed as i32 && value.parent_order_id.is_some() {
        return Err(invalid("active gift orders cannot have execution lineage"));
    }
    let book = book(required(value.book.clone())?)?;
    let quantity = gift_quantity(&value.quantity)?;
    let market_url = ["url", "listing_url", "market_url", "link"]
        .into_iter()
        .find_map(|key| value.meta.get(key).cloned());
    let order = GiftOrderData {
        id: value.id.clone().try_into().map_err(invalid)?,
        target: target(
            value.item_id.clone(),
            book.collection_id.clone(),
            value.attributes.clone(),
        ),
        book,
        kind: match w::OrderMode::try_from(value.mode) {
            Ok(w::OrderMode::Ask) => GiftOrderKind::Ask,
            Ok(w::OrderMode::Bid) => GiftOrderKind::Bid,
            Ok(w::OrderMode::Executed) => GiftOrderKind::Executed,
            _ => return Err(invalid("unknown gift order kind")),
        },
        price: value.price.clone().try_into().map_err(invalid)?,
        quantity,
        observed_at: time(value.observed_at_ms)?,
        expires_at: value.expires_at_ms.map(time).transpose()?,
        executed_at: value.executed_at_ms.map(time).transpose()?,
        market_url,
    };
    order.try_into().map_err(invalid)
}

fn gift_quantity(value: &str) -> Result<u32> {
    let decimal = rust_decimal::Decimal::from_str_exact(value)
        .map_err(invalid)?
        .normalize();
    decimal
        .to_string()
        .parse::<u32>()
        .ok()
        .filter(|quantity| *quantity > 0)
        .ok_or_else(|| invalid("gift quantity must be a positive integer"))
}
/// Gift facts from a listing, without constructing pricing, timestamps or presentation fields.
/// Expired listings may still establish identity; exhausted ones cannot.
pub fn concrete_gift(value: w::Order) -> Result<Option<CollectibleGift>> {
    if is_exhausted(&value)? {
        return Ok(None);
    }
    gift_quantity(&value.quantity)?;
    let collection = required(value.book)?
        .item_type_id
        .try_into()
        .map_err(invalid)?;
    Ok(match target(value.item_id, collection, value.attributes) {
        GiftTarget::Gift { gift } => Some(gift),
        _ => None,
    })
}
