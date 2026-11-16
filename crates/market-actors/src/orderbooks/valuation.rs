use crate::valuation::{ModelKey, ResolvedPosition, UnpricedReason, Valuation};
use market_hub_protocol::wire as w;
use market_replica::ReplicaReader;
use rust_decimal::Decimal;
use std::collections::BTreeMap;
use tonic::Status;

fn decimal(value: &str) -> Result<Decimal, Status> {
    Decimal::from_str_exact(value).map_err(|_| Status::data_loss("invalid order amount"))
}
fn floor(
    reader: &ReplicaReader,
    key: &ModelKey,
    currency: &str,
    now: i64,
) -> Result<Option<(Decimal, String)>, Status> {
    let mut request = w::ReadRequest {
        selection: Some(w::Selection {
            source: Some(w::selection::Source::Scope(w::Scope {
                item_type_id: Some(key.collection.clone()),
                currency_id: Some(currency.to_owned()),
                modes: vec![w::OrderMode::Ask as i32],
                attributes: [(
                    key.attribute.clone(),
                    w::AttributePredicate {
                        predicate: Some(w::attribute_predicate::Predicate::Eq(w::AttributeValue {
                            value: Some(w::attribute_value::Value::OptionId(key.option.clone())),
                        })),
                    },
                )]
                .into(),
                ..Default::default()
            })),
            ..Default::default()
        }),
        mode: w::ReadMode::Live as i32,
        sort: w::ReadSort::PriceAsc as i32,
        limit: 1,
        ..Default::default()
    };
    for _ in 0..256 {
        let page = reader.read(request.clone())?.page;
        for entity in page.entities {
            if let Some(w::entity::Entity::Order(order)) = entity.entity
                && order.expires_at_ms.is_none_or(|expires| expires > now)
                && decimal(&order.quantity)? > Decimal::ZERO
            {
                let price = decimal(&order.price)?;
                if price < Decimal::ZERO {
                    return Err(Status::data_loss("negative order price"));
                }
                return Ok(Some((
                    price,
                    order
                        .book
                        .ok_or_else(|| Status::data_loss("missing book"))?
                        .venue_id,
                )));
            }
        }
        if page.next_page_token.is_empty() {
            return Ok(None);
        }
        request.page_token = page.next_page_token;
    }
    Err(Status::resource_exhausted("floor scan limit exceeded"))
}
pub(super) fn value(
    reader: ReplicaReader,
    positions: Vec<ResolvedPosition>,
    currency: String,
) -> Result<Valuation, Status> {
    let reader = reader.snapshot()?;
    let currency_page = reader.read(w::ReadRequest {
        selection: Some(w::Selection {
            kinds: vec![w::EntityKind::Currency as i32],
            keys: vec![w::Key {
                kind: w::EntityKind::Currency as i32,
                id: currency.clone(),
                ..Default::default()
            }],
            ..Default::default()
        }),
        mode: w::ReadMode::Live as i32,
        limit: 1,
        ..Default::default()
    })?;
    if currency_page.page.entities.is_empty() {
        return Err(Status::invalid_argument("unknown quote currency"));
    }
    let mut total = Decimal::ZERO;
    let mut priced = 0;
    let mut unpriced = 0;
    let mut rows = Vec::new();
    let mut quotes = BTreeMap::new();
    let now = jiff::Timestamp::now().as_millisecond();
    for position in positions {
        let mut row = position.row;
        if let Some(key) = position.key {
            if !quotes.contains_key(&key) {
                quotes.insert(key.clone(), floor(&reader, &key, &currency, now)?);
            }
            if let Some((price, venue)) = &quotes[&key] {
                let value = price
                    .checked_mul(Decimal::from(row.quantity))
                    .ok_or_else(|| Status::data_loss("portfolio value overflow"))?;
                total = total
                    .checked_add(value)
                    .ok_or_else(|| Status::data_loss("portfolio value overflow"))?;
                row.floor = Some(price.normalize().to_string());
                row.value = Some(value.normalize().to_string());
                row.venue_id = Some(venue.clone());
            } else {
                row.unpriced = Some(UnpricedReason::NoListing);
            }
        }
        if row.floor.is_some() {
            priced += row.quantity;
        } else {
            unpriced += row.quantity;
        }
        rows.push(row);
    }
    Ok(Valuation {
        rows,
        total: total.normalize().to_string(),
        priced,
        unpriced,
        currency_id: currency,
        cursor: currency_page.page.cursor,
        sync: reader.status().into(),
    })
}
