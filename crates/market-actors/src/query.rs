//! Compile the finite gift query language into private generic replica requests.
use crate::{
    error::{DataError, Result},
    protocol::{BACKDROP, MODEL, NUMBER, PATTERN},
};
use market_gifts::*;
use market_hub_protocol::wire as w;
use std::collections::BTreeMap;
fn limit(value: u32) -> Result<u32> {
    if (1..=200).contains(&value) {
        Ok(value)
    } else {
        Err(DataError::invalid("limit must be 1..200"))
    }
}
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
fn number(value: u32) -> w::AttributeValue {
    w::AttributeValue {
        value: Some(w::attribute_value::Value::Integer(i64::from(value))),
    }
}
fn scope(filters: GiftFilters, mode: w::OrderMode) -> Result<w::Scope> {
    if filters.models.len() + filters.backdrops.len() + filters.patterns.len() > 500 {
        return Err(DataError::invalid("at most 500 gift trait selections"));
    }
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
        if range.min == Some(0)
            || range.max == Some(0)
            || range.min.zip(range.max).is_some_and(|(min, max)| min > max)
        {
            return Err(DataError::invalid("invalid gift number range"));
        }
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
    if filters.price.as_ref().is_some_and(|range| {
        range
            .min
            .as_ref()
            .zip(range.max.as_ref())
            .is_some_and(|(min, max)| min.decimal() > max.decimal())
    }) {
        return Err(DataError::invalid("invalid gift price range"));
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
pub(crate) fn orders(query: GiftOrdersQuery, mode: w::ReadMode) -> Result<w::ReadRequest> {
    Ok(w::ReadRequest {
        selection: Some(w::Selection {
            source: Some(w::selection::Source::Scope(scope(
                query.filters,
                match query.side {
                    GiftSide::Ask => w::OrderMode::Ask,
                    GiftSide::Bid => w::OrderMode::Bid,
                },
            )?)),
            ..Default::default()
        }),
        limit: limit(query.limit)?,
        page_token: query.page_token,
        mode: mode as i32,
        sort: sort(query.sort),
    })
}
pub(crate) fn history(query: GiftHistoryQuery) -> Result<w::ReadRequest> {
    if query.period.as_ref().is_some_and(|p| {
        p.from
            .zip(p.until)
            .is_some_and(|(from, until)| from > until)
    }) {
        return Err(DataError::invalid("invalid execution period"));
    }
    Ok(w::ReadRequest {
        selection: Some(w::Selection {
            source: Some(w::selection::Source::Scope(scope(
                query.filters,
                w::OrderMode::Executed,
            )?)),
            executed_at: query.period.map(|p| w::Period {
                from_ms: p.from.map(|t| t.as_millisecond()),
                until_ms: p.until.map(|t| t.as_millisecond()),
            }),
            ..Default::default()
        }),
        limit: limit(query.limit)?,
        page_token: query.page_token,
        sort: sort(query.sort),
        ..Default::default()
    })
}
