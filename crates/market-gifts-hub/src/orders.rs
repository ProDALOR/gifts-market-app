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
fn required_option(value: &w::AttributeValue) -> Result<&str> {
    match required(value.value.as_ref())? {
        w::attribute_value::Value::OptionId(id) => Ok(id),
        _ => Err(invalid("gift trait must reference a catalog option")),
    }
}
fn option_values(value: &w::AttributePredicate) -> Result<&[w::AttributeValue]> {
    use w::attribute_predicate::Predicate as P;
    match required(value.predicate.as_ref())? {
        P::Eq(value) => Ok(std::slice::from_ref(value)),
        P::OneOf(values) if !values.values.is_empty() => Ok(&values.values),
        _ => Err(invalid("unsupported gift trait constraint")),
    }
}
fn options(value: w::AttributePredicate) -> Result<Vec<String>> {
    let mut ids = option_values(&value)?
        .iter()
        .map(required_option)
        .collect::<Result<Vec<_>>>()?;
    ids.sort();
    ids.dedup();
    Ok(ids.into_iter().map(str::to_owned).collect())
}
fn number(value: w::AttributeValue) -> Result<u32> {
    match required(value.value)? {
        w::attribute_value::Value::Integer(value) => u32::try_from(value)
            .ok()
            .filter(|n| *n > 0)
            .ok_or_else(|| invalid("gift number must be positive")),
        _ => Err(invalid("gift number must be an integer")),
    }
}
fn number_range(value: w::AttributePredicate) -> Result<GiftNumberRange> {
    use w::attribute_predicate::Predicate as P;
    let range = match required(value.predicate)? {
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
        _ => return Err(invalid("unsupported gift number constraint")),
    };
    range.validate().map_err(invalid)?;
    Ok(range)
}

/// Allocation-free model hint for scans. Eligibility still requires `floor_quote`.
pub fn concrete_model(value: &w::Order) -> Option<&str> {
    let values = option_values(value.attributes.get(MODEL)?).ok()?;
    let model = required_option(values.first()?).ok()?;
    values
        .iter()
        .all(|value| required_option(value).ok() == Some(model))
        .then_some(model)
}

#[derive(Debug)]
pub struct FloorQuote {
    pub model_id: ModelId,
    pub price: Amount,
    pub venue_id: VenueId,
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

/// Lowest eligible ask independent of model identity.
pub fn collection_floor_quote(value: w::Order, now_ms: i64) -> Result<Option<(Amount, VenueId)>> {
    Ok(active_floor_order(value, now_ms)?.map(|order| (order.price, order.book.venue_id)))
}
pub fn floor_quote(value: w::Order, now_ms: i64) -> Result<Option<FloorQuote>> {
    let Some(order) = active_floor_order(value, now_ms)? else {
        return Ok(None);
    };
    let model_id = match order.target {
        GiftTarget::Gift { gift } => gift.traits.model,
        GiftTarget::Criteria { mut criteria } if criteria.models.len() == 1 => {
            criteria.models.pop()
        }
        _ => None,
    };
    Ok(model_id.map(|model_id| FloorQuote {
        model_id,
        price: order.price,
        venue_id: order.book.venue_id,
    }))
}
fn criteria(attributes: BTreeMap<String, w::AttributePredicate>) -> Result<GiftCriteria> {
    let mut result = GiftCriteria::default();
    for (key, value) in attributes {
        match key.as_str() {
            MODEL => {
                result.models = options(value)?
                    .into_iter()
                    .map(|v| v.try_into().map_err(invalid))
                    .collect::<Result<_>>()?
            }
            BACKDROP => {
                result.backdrops = options(value)?
                    .into_iter()
                    .map(|v| v.try_into().map_err(invalid))
                    .collect::<Result<_>>()?
            }
            PATTERN => {
                result.patterns = options(value)?
                    .into_iter()
                    .map(|v| v.try_into().map_err(invalid))
                    .collect::<Result<_>>()?
            }
            NUMBER => result.number = Some(number_range(value)?),
            _ => return Err(invalid("gift has additional unsupported constraints")),
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
        Err(error) => {
            return GiftTarget::Unsupported {
                reason: error.to_string(),
            };
        }
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
                    reason: "identified gift has non-concrete traits".into(),
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
                    reason: "invalid collectible gift ID".into(),
                },
            }
        }
    }
}
pub fn order(value: w::Order) -> Result<GiftOrder> {
    let book = book(required(value.book)?)?;
    let decimal = rust_decimal::Decimal::from_str_exact(&value.quantity)
        .map_err(invalid)?
        .normalize();
    let quantity: u32 = decimal
        .to_string()
        .parse()
        .ok()
        .filter(|q| *q > 0)
        .ok_or_else(|| invalid("gift quantity must be a positive integer"))?;
    let market_url = ["url", "listing_url", "market_url", "link"]
        .into_iter()
        .find_map(|key| value.meta.get(key).cloned());
    Ok(GiftOrder {
        id: value.id.try_into().map_err(invalid)?,
        target: target(value.item_id, book.collection_id.clone(), value.attributes),
        book,
        mode: match w::OrderMode::try_from(value.mode) {
            Ok(w::OrderMode::Ask) => GiftOrderKind::Ask,
            Ok(w::OrderMode::Bid) => GiftOrderKind::Bid,
            Ok(w::OrderMode::Executed) => GiftOrderKind::Executed,
            _ => return Err(invalid("unknown gift order kind")),
        },
        price: value.price.try_into().map_err(invalid)?,
        quantity,
        observed_at: time(value.observed_at_ms)?,
        expires_at: value.expires_at_ms.map(time).transpose()?,
        executed_at: value.executed_at_ms.map(time).transpose()?,
        parent_order_id: value
            .parent_order_id
            .map(TryInto::try_into)
            .transpose()
            .map_err(invalid)?,
        market_url,
    })
}
