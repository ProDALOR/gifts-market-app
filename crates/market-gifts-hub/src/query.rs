//! Pure translation of gift queries into generic hub reads. Service limits belong to callers.
use super::{BACKDROP, MODEL, NUMBER, PATTERN};
use market_gifts::*;
use market_hub_protocol::wire as w;
use std::collections::BTreeMap;
fn selection<T: AsRef<str>>(ids: Vec<T>) -> w::AttributePredicate {
    let mut ids: Vec<_> = ids.into_iter().map(|id| id.as_ref().to_owned()).collect();
    ids.sort();
    ids.dedup();
    let values: Vec<_> = ids
        .into_iter()
        .map(|id| w::AttributeValue {
            value: Some(w::attribute_value::Value::OptionId(id)),
        })
        .collect();
    w::AttributePredicate {
        predicate: Some(if values.len() == 1 {
            w::attribute_predicate::Predicate::Eq(values.into_iter().next().unwrap())
        } else {
            w::attribute_predicate::Predicate::OneOf(w::AttributeValues { values })
        }),
    }
}
fn number(value: GiftNumber) -> w::AttributeValue {
    w::AttributeValue {
        value: Some(w::attribute_value::Value::Integer(i64::from(value.get()))),
    }
}
fn scope(filters: GiftFilters, mode: w::OrderMode) -> Result<w::Scope, InvalidValue> {
    filters.validate()?;
    let mut attributes = BTreeMap::new();
    if !filters.models.is_empty() {
        attributes.insert(MODEL.into(), selection(filters.models));
    }
    if !filters.backdrops.is_empty() {
        attributes.insert(BACKDROP.into(), selection(filters.backdrops));
    }
    if !filters.patterns.is_empty() {
        attributes.insert(PATTERN.into(), selection(filters.patterns));
    }
    if let Some(range) = filters.number {
        attributes.insert(
            NUMBER.into(),
            w::AttributePredicate {
                predicate: Some(w::attribute_predicate::Predicate::Range(
                    w::AttributeRange {
                        min: range.min.map(number),
                        max: range.max.map(number),
                    },
                )),
            },
        );
    }
    Ok(w::Scope {
        venue_id: filters.venue_id.map(Into::into),
        item_type_id: Some(filters.collection_id.into()),
        currency_id: Some(filters.currency_id.into()),
        modes: vec![mode as i32],
        attributes,
        price: filters.price.map(|range| w::PriceRange {
            min: range.min.map(Into::into),
            max: range.max.map(Into::into),
        }),
    })
}
fn sort(value: GiftSort) -> i32 {
    (match value {
        GiftSort::Identity => w::ReadSort::Identity,
        GiftSort::PriceAsc => w::ReadSort::PriceAsc,
        GiftSort::PriceDesc => w::ReadSort::PriceDesc,
        GiftSort::TimeAsc => w::ReadSort::TimeAsc,
        GiftSort::TimeDesc => w::ReadSort::TimeDesc,
        GiftSort::NumberAsc => w::ReadSort::NumberAsc,
        GiftSort::NumberDesc => w::ReadSort::NumberDesc,
    }) as i32
}
pub fn compile(query: &GiftOrderQuery) -> Result<w::ReadRequest, InvalidValue> {
    if query.source != OrderSource::History && query.period.is_some() {
        return Err(InvalidValue("period is only valid for history"));
    }
    if let Some(period) = &query.period {
        period.validate()?;
    }
    Ok(w::ReadRequest {
        selection: Some(w::Selection {
            source: Some(w::selection::Source::Scope(scope(
                query.filters.clone(),
                match query.source {
                    OrderSource::Asks => w::OrderMode::Ask,
                    OrderSource::Bids => w::OrderMode::Bid,
                    OrderSource::History => w::OrderMode::Executed,
                },
            )?)),
            executed_at: query.period.as_ref().map(|p| w::Period {
                from_ms: p.from.map(|t| t.as_millisecond()),
                until_ms: p.until.map(|t| t.as_millisecond()),
            }),
            ..Default::default()
        }),
        limit: query.limit,
        mode: w::ReadMode::Live as i32,
        sort: sort(query.sort()),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gift_queries_compile_canonical_traits_and_exact_prices_without_service_policy() {
        let mut query = GiftOrderQuery::new(GiftFilters::new("collection".into(), "GRAM".into()));
        query.filters.models = vec!["b".into(), "a".into(), "a".into()];
        query.filters.patterns = vec!["pattern".into()];
        query.filters.number = Some(GiftNumberRange {
            min: Some(GiftNumber::new(1).unwrap()),
            max: Some(GiftNumber::new(42).unwrap()),
        });
        query.filters.price = Some(PriceRange {
            min: Some("1.000000001".into()),
            max: None,
        });
        query.source = OrderSource::Bids;
        query.sort = Some(GiftSort::NumberDesc);
        query.limit = 500;
        let read = compile(&query).unwrap();
        assert_eq!(read.mode, w::ReadMode::Live as i32);
        assert_eq!(read.sort, w::ReadSort::NumberDesc as i32);
        assert_eq!(read.limit, 500);
        assert!(read.page_token.is_empty());
        let Some(w::selection::Source::Scope(scope)) = read.selection.unwrap().source else {
            panic!("missing scope")
        };
        assert_eq!(scope.modes, vec![w::OrderMode::Bid as i32]);
        assert_eq!(scope.price.unwrap().min.as_deref(), Some("1.000000001"));
        let Some(w::attribute_predicate::Predicate::OneOf(values)) =
            &scope.attributes[MODEL].predicate
        else {
            panic!("expected selection")
        };
        assert_eq!(
            values
                .values
                .iter()
                .map(|v| match &v.value {
                    Some(w::attribute_value::Value::OptionId(id)) => id.as_str(),
                    _ => panic!("expected option"),
                })
                .collect::<Vec<_>>(),
            vec!["a", "b"]
        );
        query.filters.number = Some(GiftNumberRange {
            min: Some(GiftNumber::new(43).unwrap()),
            max: Some(GiftNumber::new(42).unwrap()),
        });
        assert!(compile(&query).is_err());
    }
    #[test]
    fn history_uses_shared_period_validation_and_preserves_execution_bounds() {
        let mut query = GiftOrderQuery::new(GiftFilters::new("collection".into(), "GRAM".into()));
        query.source = OrderSource::History;
        let from: jiff::Timestamp = "2026-01-01T00:00:00Z".parse().unwrap();
        let until: jiff::Timestamp = "2026-01-02T00:00:00Z".parse().unwrap();
        query.period = Some(Period {
            from: Some(from),
            until: Some(until),
        });
        let read = compile(&query).unwrap();
        let selection = read.selection.unwrap();
        let period = selection.executed_at.unwrap();
        assert_eq!(period.from_ms, Some(from.as_millisecond()));
        assert_eq!(period.until_ms, Some(until.as_millisecond()));
        let Some(w::selection::Source::Scope(scope)) = selection.source else {
            panic!("missing scope")
        };
        assert_eq!(scope.modes, vec![w::OrderMode::Executed as i32]);
        query.period = Some(Period {
            from: Some(until),
            until: Some(from),
        });
        assert!(compile(&query).is_err());
    }
}
