//! Gift projection at the generic replica boundary. Generic attributes stay private.
use crate::error::{DataError, Result};
use jiff::Timestamp;
use market_gifts::*;
use market_hub_protocol::wire as w;
use std::collections::BTreeMap;
pub(crate) const MODEL: &str = "tg.model";
pub(crate) const BACKDROP: &str = "tg.backdrop";
pub(crate) const PATTERN: &str = "tg.pattern";
pub(crate) const NUMBER: &str = "tg.number";
pub(crate) fn invalid(value: impl std::fmt::Display) -> DataError {
    DataError::unavailable(value.to_string())
}
fn required<T>(value: Option<T>) -> Result<T> {
    value.ok_or_else(|| invalid("missing gift market field"))
}
fn time(value: i64) -> Result<Timestamp> {
    Timestamp::from_millisecond(value).map_err(invalid)
}
pub(crate) fn book(value: w::BookKey) -> Result<GiftBook> {
    Ok(GiftBook {
        venue_id: value.venue_id.try_into().map_err(invalid)?,
        collection_id: value.item_type_id.try_into().map_err(invalid)?,
        currency_id: value.currency_id.try_into().map_err(invalid)?,
    })
}
pub(crate) fn sync(value: market_replica::SyncStatus) -> Result<SyncStatus> {
    Ok(SyncStatus {
        ready: value.ready,
        state: match value.state {
            market_replica::SyncState::Starting => SyncState::Starting,
            market_replica::SyncState::Syncing => SyncState::Syncing,
            market_replica::SyncState::Live => SyncState::Live,
            market_replica::SyncState::Stale => SyncState::Stale,
        },
        updated_at: value.updated_at.map(time).transpose()?,
    })
}
fn required_option(value: w::AttributeValue) -> Result<String> {
    match required(value.value)? {
        w::attribute_value::Value::OptionId(id) => Ok(id),
        _ => Err(invalid("gift trait must reference a catalog option")),
    }
}
fn options(value: w::AttributePredicate) -> Result<Vec<String>> {
    use w::attribute_predicate::Predicate as P;
    let mut ids = match required(value.predicate)? {
        P::Eq(value) => vec![required_option(value)?],
        P::OneOf(values) if !values.values.is_empty() => values
            .values
            .into_iter()
            .map(required_option)
            .collect::<Result<_>>()?,
        _ => return Err(invalid("unsupported gift trait constraint")),
    };
    ids.sort();
    ids.dedup();
    Ok(ids)
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
    if range.min.zip(range.max).is_some_and(|(min, max)| min > max) {
        return Err(invalid("invalid gift number range"));
    }
    Ok(range)
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
                reason: error.message().to_owned(),
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
pub(crate) fn order(value: w::Order) -> Result<GiftOrder> {
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
pub(crate) fn orders(result: market_replica::ReadResult) -> Result<Page<GiftOrder>> {
    Ok(Page {
        sync: sync(result.sync)?,
        cursor: result.page.cursor,
        next_page_token: result.page.next_page_token,
        data: result
            .page
            .entities
            .into_iter()
            .map(|entity| match required(entity.entity)? {
                w::entity::Entity::Order(value) => order(value),
                _ => Err(invalid("expected a gift order")),
            })
            .collect::<Result<_>>()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn eq(value: w::attribute_value::Value) -> w::AttributePredicate {
        w::AttributePredicate {
            predicate: Some(w::attribute_predicate::Predicate::Eq(w::AttributeValue {
                value: Some(value),
            })),
        }
    }
    fn fixture() -> w::Order {
        w::Order {
            id: "order".into(),
            book: Some(w::BookKey {
                venue_id: "venue".into(),
                item_type_id: "collection".into(),
                currency_id: "GRAM".into(),
            }),
            mode: w::OrderMode::Ask as i32,
            item_id: Some("9007199254740993".into()),
            price: "10000000000000000.000000001".into(),
            quantity: "1.00".into(),
            attributes: [
                (
                    MODEL.into(),
                    eq(w::attribute_value::Value::OptionId("model".into())),
                ),
                (NUMBER.into(), eq(w::attribute_value::Value::Integer(42))),
            ]
            .into(),
            ..Default::default()
        }
    }
    #[test]
    fn projection_preserves_exact_amounts_and_distinct_concrete_identity() {
        let projected = order(fixture()).unwrap();
        assert_eq!(projected.price.as_str(), "10000000000000000.000000001");
        assert_eq!(projected.quantity, 1);
        let GiftTarget::Gift { gift } = projected.target else {
            panic!("expected concrete gift")
        };
        assert_eq!(gift.id.as_str(), "9007199254740993");
        assert_eq!(gift.number, Some(42));
        assert_eq!(gift.traits.model.as_deref(), Some("model"));
        let mut conditional = fixture();
        conditional.item_id = None;
        conditional.mode = w::OrderMode::Bid as i32;
        assert!(matches!(
            order(conditional).unwrap().target,
            GiftTarget::Criteria { .. }
        ));
    }
    #[test]
    fn unsupported_or_nonconcrete_conditions_are_never_reported_as_any_or_as_a_collectible() {
        let mut unknown = fixture();
        unknown.attributes.insert(
            "future.condition".into(),
            eq(w::attribute_value::Value::Boolean(true)),
        );
        assert!(matches!(
            order(unknown).unwrap().target,
            GiftTarget::Unsupported { .. }
        ));
        let mut multiple = fixture();
        multiple.attributes.insert(
            MODEL.into(),
            w::AttributePredicate {
                predicate: Some(w::attribute_predicate::Predicate::OneOf(
                    w::AttributeValues {
                        values: vec![
                            w::AttributeValue {
                                value: Some(w::attribute_value::Value::OptionId("a".into())),
                            },
                            w::AttributeValue {
                                value: Some(w::attribute_value::Value::OptionId("b".into())),
                            },
                        ],
                    },
                )),
            },
        );
        assert!(matches!(
            order(multiple.clone()).unwrap().target,
            GiftTarget::Unsupported { .. }
        ));
        multiple.item_id = None;
        let GiftTarget::Criteria { criteria } = order(multiple).unwrap().target else {
            panic!("expected selection")
        };
        assert_eq!(criteria.models.len(), 2);
        let mut fractional = fixture();
        fractional.quantity = "1.5".into();
        assert!(order(fractional).is_err());
        let mut negative = fixture();
        negative.price = "-1".into();
        assert!(order(negative).is_err());
    }
    #[test]
    fn gift_filters_compile_to_canonical_hub_constraints_and_validate_ranges() {
        let mut query = GiftOrdersQuery::new(GiftFilters::new("collection".into(), "GRAM".into()));
        query.filters.models = vec!["b".into(), "a".into(), "a".into()];
        query.filters.number = Some(GiftNumberRange {
            min: Some(1),
            max: Some(100),
        });
        let request = crate::query::orders(query.clone(), w::ReadMode::Live).unwrap();
        let Some(w::selection::Source::Scope(scope)) = request.selection.unwrap().source else {
            panic!("missing scope")
        };
        assert_eq!(scope.item_type_id.as_deref(), Some("collection"));
        let Some(w::attribute_predicate::Predicate::OneOf(values)) =
            &scope.attributes[MODEL].predicate
        else {
            panic!("missing models")
        };
        assert_eq!(values.values.len(), 2);
        let mut invalid = query;
        invalid.filters.number = Some(GiftNumberRange {
            min: Some(100),
            max: Some(1),
        });
        assert!(crate::query::orders(invalid, w::ReadMode::Live).is_err());
    }
}
