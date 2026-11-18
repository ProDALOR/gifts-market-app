//! Portfolio floor calculation over one immutable market snapshot.
use crate::error::{Result, ValuationError};
use market_actors::{BookScope, CatalogTrait, MarketSnapshot, ReadDependencies};
use market_gifts::{valuation::*, *};
use rust_decimal::Decimal;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy)]
pub(crate) enum FloorKind {
    Model,
    Collection,
}
impl FloorKind {
    pub fn id(self) -> &'static str {
        match self {
            Self::Model => "model_floor",
            Self::Collection => "collection_floor",
        }
    }
}
pub(crate) fn normalize(input: PortfolioPositions) -> Result<PortfolioPositions> {
    if input.positions.len() > 1000 {
        return Err(ValuationError::invalid(
            "at most 1000 model positions are allowed",
        ));
    }
    let mut total = 0u32;
    let mut grouped = BTreeMap::new();
    for p in input.positions {
        p.validate()
            .map_err(|e| ValuationError::invalid(e.to_string()))?;
        total = total
            .checked_add(p.quantity)
            .filter(|n| *n <= 10_000)
            .ok_or_else(|| ValuationError::invalid("portfolio quantity must be 1..10000"))?;
        *grouped.entry(p.model).or_insert(0u32) += p.quantity;
    }
    Ok(PortfolioPositions {
        positions: grouped
            .into_iter()
            .map(|(model, quantity)| ModelPosition { model, quantity })
            .collect(),
        currency_id: input.currency_id,
    })
}
pub(crate) fn portfolio(
    snapshot: MarketSnapshot,
    input: PortfolioPositions,
    kind: FloorKind,
) -> Result<(crate::Prepared<Valuation>, ReadDependencies)> {
    snapshot.require_currency(&input.currency_id)?;
    let lookups = input
        .positions
        .iter()
        .map(|p| p.model.clone())
        .collect::<Vec<_>>();
    let quantities = input
        .positions
        .iter()
        .map(|p| p.quantity)
        .collect::<Vec<_>>();
    let mut collections: BTreeSet<CollectionId> = input
        .positions
        .iter()
        .filter_map(|p| match &p.model {
            ModelLookup::Catalog { collection, .. } | ModelLookup::Collection { collection } => {
                Some(collection.clone())
            }
            _ => None,
        })
        .collect();
    let positions = snapshot.resolve_models(lookups.clone())?;
    collections.extend(positions.iter().filter_map(|p| p.collection.clone()));
    let dependencies = snapshot.dependencies(
        collections.iter().cloned().map(|collection| BookScope {
            collection,
            currency: input.currency_id.clone(),
        }),
        &input.currency_id,
        CatalogTrait::Model,
        &lookups,
    )?;
    let mut missing: BTreeMap<CollectionId, BTreeSet<ModelId>> = BTreeMap::new();
    for p in &positions {
        match kind {
            FloorKind::Model => {
                if let Some(id) = &p.identity {
                    missing
                        .entry(id.collection_id.clone())
                        .or_default()
                        .insert(id.model_id.clone());
                }
            }
            FloorKind::Collection => {
                if let Some(id) = &p.collection {
                    missing.entry(id.clone()).or_default();
                }
            }
        }
    }
    let now = jiff::Timestamp::now().as_millisecond();
    let (models, broad) = scan(missing, &input.currency_id, kind, |scope, token| {
        Ok(snapshot.asks(scope, now, token)?)
    })?;
    let mut rows = Vec::with_capacity(positions.len());
    let mut total = Decimal::ZERO;
    let mut priced = 0;
    let mut unpriced = 0;
    for (position, quantity) in positions.into_iter().zip(quantities) {
        let quote = match kind {
            FloorKind::Model => position.identity.as_ref().and_then(|id| models.get(id)),
            FloorKind::Collection => position.collection.as_ref().and_then(|id| broad.get(id)),
        };
        let mut row = ModelValue {
            identity: position.identity,
            collection: position.collection_label,
            model: position.model_label,
            quantity,
            floor: None,
            value: None,
            venue_id: None,
            unpriced: position.unresolved,
        };
        if let Some((price, venue)) = quote {
            let value = price
                .decimal()
                .checked_mul(Decimal::from(row.quantity))
                .ok_or_else(|| ValuationError::unavailable("portfolio value overflow"))?;
            total = total
                .checked_add(value)
                .ok_or_else(|| ValuationError::unavailable("portfolio value overflow"))?;
            row.floor = Some(price.clone());
            row.value = Some(
                value
                    .try_into()
                    .map_err(|e| ValuationError::unavailable(format!("{e}")))?,
            );
            row.venue_id = Some(venue.clone());
            row.unpriced = None;
            priced += row.quantity;
        } else {
            if matches!(kind, FloorKind::Collection) && position.collection.is_some() {
                row.unpriced = None;
            }
            if row.unpriced.is_none() {
                row.unpriced = Some(UnpricedReason::NoListing);
            }
            unpriced += row.quantity;
        }
        rows.push(row);
    }
    Ok((
        crate::Prepared::new(
            Valuation {
                rows,
                total: total
                    .try_into()
                    .map_err(|e| ValuationError::unavailable(format!("{e}")))?,
                priced,
                unpriced,
                currency_id: input.currency_id,
                sync: snapshot.sync()?,
            },
            snapshot.revision()?,
        ),
        dependencies,
    ))
}

type Floors = (
    BTreeMap<ModelIdentity, (Amount, VenueId)>,
    BTreeMap<CollectionId, (Amount, VenueId)>,
);
fn scan(
    missing: BTreeMap<CollectionId, BTreeSet<ModelId>>,
    currency: &CurrencyId,
    kind: FloorKind,
    mut read: impl FnMut(&BookScope, String) -> Result<Page<market_actors::AskQuote>>,
) -> Result<Floors> {
    let mut models = BTreeMap::new();
    let mut broad = BTreeMap::new();
    let mut budget = 2560;
    for (collection, mut remaining) in missing {
        let scope = BookScope {
            collection: collection.clone(),
            currency: currency.clone(),
        };
        let mut token = String::new();
        'pages: loop {
            if budget == 0 {
                return Err(ValuationError::capacity("floor scan limit exceeded"));
            }
            budget -= 1;
            let page = read(&scope, token)?;
            for quote in page.data {
                match kind {
                    FloorKind::Collection => {
                        broad.insert(collection.clone(), (quote.price, quote.venue_id));
                        break 'pages;
                    }
                    FloorKind::Model => {
                        if let Some(id) = quote.model_id
                            && remaining.remove(&id)
                        {
                            models.insert(
                                ModelIdentity {
                                    collection_id: collection.clone(),
                                    model_id: id,
                                },
                                (quote.price, quote.venue_id),
                            );
                            if remaining.is_empty() {
                                break 'pages;
                            }
                        }
                    }
                }
            }
            if page.next_page_token.is_empty() {
                break;
            }
            token = page.next_page_token;
        }
    }
    Ok((models, broad))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integration_tests::entity as upsert;
    use market_hub_protocol::wire as w;
    #[tokio::test]
    async fn many_models_use_one_book_pass_instead_of_one_scan_per_model() {
        let directory = tempfile::tempdir().unwrap();
        let mut writer =
            market_replica::ReplicaWriter::open(directory.path(), "batch-floors").unwrap();
        let mut seed = writer.begin_snapshot().unwrap();
        seed.append(&[
            upsert(w::entity::Entity::Venue(w::Venue {
                id: "venue".into(),
                name: "Venue".into(),
            })),
            upsert(w::entity::Entity::Currency(w::Currency {
                id: "GRAM".into(),
                name: "Gram".into(),
                decimals: 9,
            })),
            upsert(w::entity::Entity::ItemType(w::ItemType {
                id: "gift".into(),
                name: "Gift".into(),
                quantity_step: "1".into(),
                ..Default::default()
            })),
        ])
        .unwrap();
        let rows: Vec<_> = (0..10_000)
            .map(|i| {
                upsert(w::entity::Entity::Order(w::Order {
                    id: format!("order.{i:05}"),
                    book: Some(w::BookKey {
                        item_type_id: "gift".into(),
                        currency_id: "GRAM".into(),
                        venue_id: "venue".into(),
                    }),
                    mode: w::OrderMode::Ask as i32,
                    price: (i + 1).to_string(),
                    quantity: "1".into(),
                    attributes: [(
                        market_gifts_hub::MODEL.into(),
                        w::AttributePredicate {
                            predicate: Some(w::attribute_predicate::Predicate::Eq(
                                w::AttributeValue {
                                    value: Some(w::attribute_value::Value::OptionId(format!(
                                        "model.{}",
                                        i / 100
                                    ))),
                                },
                            )),
                        },
                    )]
                    .into(),
                    ..Default::default()
                }))
            })
            .collect();
        seed.append(&rows).unwrap();
        writer
            .install_snapshot(seed.finish("seed").unwrap())
            .unwrap();
        let service = market_actors::MarketDataService::start(writer.reader(), 2, 1)
            .await
            .unwrap();
        let snapshot = service.reader.snapshot().unwrap();
        let missing = [(
            "gift".into(),
            (0..100)
                .map(|i| format!("model.{i}").try_into().unwrap())
                .collect(),
        )]
        .into();
        let mut calls = 0;
        let mut scanned = 0;
        let (result, _) = scan(missing, &"GRAM".into(), FloorKind::Model, |scope, token| {
            calls += 1;
            let page = snapshot.asks(scope, 0, token)?;
            scanned += page.data.len();
            Ok(page)
        })
        .unwrap();
        assert_eq!(result.len(), 100);
        assert_eq!(scanned, 10_000);
        assert_eq!(calls, 20);
        for i in 0..100 {
            let key = ModelIdentity {
                collection_id: "gift".into(),
                model_id: format!("model.{i}").try_into().unwrap(),
            };
            assert_eq!(result[&key].0.decimal(), Decimal::from(i * 100 + 1));
        }
        let stop = tokio_util::sync::CancellationToken::new();
        stop.cancel();
        service.task.run(stop).await.unwrap();
    }
}
