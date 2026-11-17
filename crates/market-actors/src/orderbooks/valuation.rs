use market_gifts::valuation::{
    ModelIdentity, ModelPosition, ModelValue, UnpricedReason, Valuation, ValuationRequest,
};
use market_gifts::{CurrencyId, VenueId};
use market_hub_protocol::wire as w;
use market_replica::ReplicaReader;
use rust_decimal::Decimal;
use std::collections::{BTreeMap, BTreeSet};
use tonic::Status;

/// A single ascending-price scan per collection supplies every requested model.
/// The reader is pinned by the caller, so catalog identity and all quotes agree.
fn floors(
    reader: &ReplicaReader,
    keys: impl Iterator<Item = ModelIdentity>,
    currency: &CurrencyId,
    now: i64,
) -> Result<BTreeMap<ModelIdentity, (Decimal, VenueId)>, Status> {
    scan_floors(keys, currency, now, |request| {
        Ok(reader.read(request)?.page)
    })
}
fn scan_floors(
    keys: impl Iterator<Item = ModelIdentity>,
    currency: &CurrencyId,
    now: i64,
    mut read: impl FnMut(w::ReadRequest) -> Result<w::Page, Status>,
) -> Result<BTreeMap<ModelIdentity, (Decimal, VenueId)>, Status> {
    let mut groups: BTreeMap<market_gifts::CollectionId, BTreeSet<market_gifts::ModelId>> =
        BTreeMap::new();
    for key in keys {
        groups
            .entry(key.collection_id)
            .or_default()
            .insert(key.model_id);
    }
    let mut quotes = BTreeMap::new();
    let mut budget = 2560; // bounded across the whole portfolio, not per model
    for (collection_id, mut missing) in groups {
        let mut request = w::ReadRequest {
            selection: Some(w::Selection {
                source: Some(w::selection::Source::Scope(w::Scope {
                    item_type_id: Some(collection_id.to_string()),
                    currency_id: Some(currency.to_string()),
                    modes: vec![w::OrderMode::Ask as i32],
                    ..Default::default()
                })),
                ..Default::default()
            }),
            mode: w::ReadMode::Live as i32,
            sort: w::ReadSort::PriceAsc as i32,
            limit: 500,
            ..Default::default()
        };
        loop {
            if budget == 0 {
                return Err(Status::resource_exhausted("floor scan limit exceeded"));
            }
            budget -= 1;
            let page = read(request.clone())?;
            for entity in page.entities {
                let Some(w::entity::Entity::Order(order)) = entity.entity else {
                    continue;
                };
                let Some(model) = market_gifts_hub::concrete_model(&order) else {
                    continue;
                };
                if !missing.contains(model) {
                    continue;
                }
                let Some(quote) = market_gifts_hub::floor_quote(order, now)
                    .map_err(|error| Status::data_loss(error.to_string()))?
                else {
                    continue;
                };
                if !missing.remove(quote.model_id.as_str()) {
                    continue;
                }
                quotes.insert(
                    ModelIdentity {
                        collection_id: collection_id.clone(),
                        model_id: quote.model_id,
                    },
                    (quote.price.decimal(), quote.venue_id),
                );
            }
            if missing.is_empty() || page.next_page_token.is_empty() {
                break;
            }
            request.page_token = page.next_page_token;
        }
    }
    Ok(quotes)
}
pub(super) fn value(reader: ReplicaReader, request: ValuationRequest) -> Result<Valuation, Status> {
    let reader = reader.snapshot()?;
    let mut grouped = BTreeMap::new();
    for position in request.positions {
        *grouped.entry(position.model).or_insert(0u32) += position.quantity;
    }
    let positions = grouped
        .into_iter()
        .map(|(model, quantity)| ModelPosition { model, quantity })
        .collect();
    let rows = crate::catalog::resolve_models(&reader, positions)?;
    value_resolved(&reader, rows, request.currency_id)
}
fn value_resolved(
    reader: &ReplicaReader,
    positions: Vec<ModelValue>,
    currency: CurrencyId,
) -> Result<Valuation, Status> {
    let currency_page = reader.read(w::ReadRequest {
        selection: Some(w::Selection {
            kinds: vec![w::EntityKind::Currency as i32],
            keys: vec![w::Key {
                kind: w::EntityKind::Currency as i32,
                id: currency.to_string(),
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
    let now = jiff::Timestamp::now().as_millisecond();
    let quotes = floors(
        reader,
        positions.iter().filter_map(|row| row.identity.clone()),
        &currency,
        now,
    )?;
    for mut row in positions {
        if let Some(key) = &row.identity {
            if let Some((price, venue)) = quotes.get(key) {
                let value = price
                    .checked_mul(Decimal::from(row.quantity))
                    .ok_or_else(|| Status::data_loss("portfolio value overflow"))?;
                total = total
                    .checked_add(value)
                    .ok_or_else(|| Status::data_loss("portfolio value overflow"))?;
                row.floor = Some(
                    (*price)
                        .try_into()
                        .map_err(|error| Status::data_loss(format!("{error}")))?,
                );
                row.value = Some(
                    value
                        .try_into()
                        .map_err(|error| Status::data_loss(format!("{error}")))?,
                );
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
        total: total
            .try_into()
            .map_err(|error| Status::data_loss(format!("{error}")))?,
        priced,
        unpriced,
        currency_id: currency,
        cursor: currency_page.page.cursor,
        sync: crate::protocol::sync(reader.status())
            .map_err(|error| Status::data_loss(error.to_string()))?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use market_gifts::valuation::{ModelLookup, ModelPosition};
    use market_gifts::{TelegramDocumentId, TelegramGiftId};
    fn upsert(entity: w::entity::Entity) -> w::Change {
        w::Change {
            change: Some(w::change::Change::Upsert(w::Entity {
                entity: Some(entity),
            })),
        }
    }
    #[test]
    fn floor_scan_skips_unsupported_asks_and_prices_singleton_model_selections() {
        let ask = |price: &str| w::Order {
            id: format!("order.{price}"),
            book: Some(w::BookKey {
                venue_id: "venue".into(),
                item_type_id: "collection".into(),
                currency_id: "GRAM".into(),
            }),
            mode: w::OrderMode::Ask as i32,
            item_id: Some("gift".into()),
            price: price.into(),
            quantity: "1".into(),
            attributes: [(
                market_gifts_hub::MODEL.into(),
                w::AttributePredicate {
                    predicate: Some(w::attribute_predicate::Predicate::Eq(w::AttributeValue {
                        value: Some(w::attribute_value::Value::OptionId("model".into())),
                    })),
                },
            )]
            .into(),
            ..Default::default()
        };
        let mut unsupported = ask("1");
        unsupported
            .attributes
            .insert("unsupported".into(), w::AttributePredicate::default());
        let mut expired = ask("2");
        expired.expires_at_ms = Some(10);
        let mut singleton = ask("3");
        singleton.attributes.insert(
            market_gifts_hub::MODEL.into(),
            w::AttributePredicate {
                predicate: Some(w::attribute_predicate::Predicate::OneOf(
                    w::AttributeValues {
                        values: vec![w::AttributeValue {
                            value: Some(w::attribute_value::Value::OptionId("model".into())),
                        }],
                    },
                )),
            },
        );
        let key = ModelIdentity {
            collection_id: "collection".into(),
            model_id: "model".into(),
        };
        let mut calls = 0;
        let quotes = scan_floors(std::iter::once(key.clone()), &"GRAM".into(), 10, |_| {
            calls += 1;
            Ok(w::Page {
                entities: [
                    unsupported.clone(),
                    expired.clone(),
                    singleton.clone(),
                    ask("4"),
                ]
                .into_iter()
                .map(|order| w::Entity {
                    entity: Some(w::entity::Entity::Order(order)),
                })
                .collect(),
                ..Default::default()
            })
        })
        .unwrap();
        assert_eq!(calls, 1);
        assert_eq!(quotes[&key].0, Decimal::from(3));
        let mut fractional = ask("1");
        fractional.quantity = "1.5".into();
        let error = scan_floors(std::iter::once(key), &"GRAM".into(), 10, |_| {
            Ok(w::Page {
                entities: vec![w::Entity {
                    entity: Some(w::entity::Entity::Order(fractional.clone())),
                }],
                ..Default::default()
            })
        })
        .unwrap_err();
        assert_eq!(error.code(), tonic::Code::DataLoss);
    }
    #[test]
    fn many_models_use_one_book_pass_instead_of_one_scan_per_model() {
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
        let reader = writer.reader().snapshot().unwrap();
        let keys = (0..100).map(|i| ModelIdentity {
            collection_id: "gift".into(),
            model_id: format!("model.{i}").try_into().unwrap(),
        });
        let mut calls = 0;
        let mut scanned = 0;
        let result = scan_floors(keys, &"GRAM".into(), 0, |request| {
            calls += 1;
            let page = reader.read(request)?.page;
            scanned += page.entities.len();
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
            assert_eq!(result[&key].0, Decimal::from(i * 100 + 1));
        }
    }
    #[test]
    fn catalog_identity_and_floor_share_a_view_during_replica_replacement() {
        let directory = tempfile::tempdir().unwrap();
        let mut writer =
            market_replica::ReplicaWriter::open(directory.path(), "valuation").unwrap();
        let option = |name: &str, document: &str| {
            upsert(w::entity::Entity::AttributeOption(w::AttributeOption {
                id: "model".into(),
                name: name.into(),
                attribute_id: market_gifts_hub::MODEL.into(),
                aliases: vec![w::ExternalId {
                    namespace: "tg".into(),
                    id: document.into(),
                }],
                ..Default::default()
            }))
        };
        let ask = |price: &str| {
            upsert(w::entity::Entity::Order(w::Order {
                id: "order".into(),
                book: Some(w::BookKey {
                    venue_id: "venue".into(),
                    item_type_id: "gift".into(),
                    currency_id: "GRAM".into(),
                }),
                mode: w::OrderMode::Ask as i32,
                price: price.into(),
                quantity: "1".into(),
                attributes: [(
                    market_gifts_hub::MODEL.into(),
                    w::AttributePredicate {
                        predicate: Some(w::attribute_predicate::Predicate::Eq(w::AttributeValue {
                            value: Some(w::attribute_value::Value::OptionId("model".into())),
                        })),
                    },
                )]
                .into(),
                ..Default::default()
            }))
        };
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
            upsert(w::entity::Entity::Attribute(w::Attribute {
                id: market_gifts_hub::MODEL.into(),
                name: "Model".into(),
                value_type: w::AttributeType::Enum as i32,
                ..Default::default()
            })),
            upsert(w::entity::Entity::ItemType(w::ItemType {
                id: "gift".into(),
                name: "Gift".into(),
                quantity_step: "1".into(),
                aliases: vec![w::ExternalId {
                    namespace: "tg".into(),
                    id: "99".into(),
                }],
                attributes: [(
                    market_gifts_hub::MODEL.into(),
                    w::AttributeRule {
                        required: true,
                        allowed_values: Some(w::AllowedAttributeValues {
                            ids: vec!["model".into()],
                        }),
                    },
                )]
                .into(),
            })),
            option("Original", "10"),
            ask("3"),
        ])
        .unwrap();
        writer
            .install_snapshot(seed.finish("original").unwrap())
            .unwrap();
        let position = ModelPosition {
            model: ModelLookup::Telegram {
                gift_id: TelegramGiftId::new(99).unwrap(),
                document_id: TelegramDocumentId::new(10).unwrap(),
            },
            quantity: 2,
        };
        let view = writer.reader().snapshot().unwrap();
        let resolved = crate::catalog::resolve_models(&view, vec![position.clone()]).unwrap();
        writer
            .apply(w::Batch {
                cursor: "replacement".into(),
                changes: vec![option("Replacement", "20"), ask("9")],
            })
            .unwrap();
        let result = value_resolved(&view, resolved, "GRAM".into()).unwrap();
        assert_eq!(result.cursor, "original");
        assert_eq!(result.rows[0].model, "Original");
        assert_eq!(
            result.rows[0].identity.as_ref().unwrap().model_id.as_str(),
            "model"
        );
        assert_eq!(result.total.as_str(), "6");
        let current = value(
            writer.reader(),
            ValuationRequest {
                positions: vec![position],
                currency_id: "GRAM".into(),
            },
        )
        .unwrap();
        assert_eq!(current.cursor, "replacement");
        assert_eq!(current.total.as_str(), "0");
        assert_eq!(current.rows[0].unpriced, Some(UnpricedReason::UnknownModel));
    }
}
