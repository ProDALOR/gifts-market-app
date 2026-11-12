use market_query::{BookCursor, BookFilter, BookId, BookQuery, BookSide, MarketState};
use market_view::*;
use std::sync::Arc;

fn now() -> MarketTimestamp {
    MarketTimestamp::from_millis(10_000).unwrap()
}
fn initial() -> MarketState {
    let definition: CatalogDefinition = serde_json::from_value(serde_json::json!({
        "venues": [
            {
                "id": "venue",
                "name": "Venue"
            }
        ],
        "feeds": [
            {
                "id": "a",
                "venue_id": "venue"
            },
            {
                "id": "b",
                "venue_id": "venue"
            }
        ],
        "currencies": [
            {
                "id": "TON",
                "name": "Toncoin",
                "decimals": 9
            },
            {
                "id": "USD",
                "name": "Dollar",
                "decimals": 2
            }
        ],
        "item_types": [
            {
                "id": "bear",
                "name": "Bear",
                "attributes": {
                    "model": {
                        "allowed_values": [
                            "gold",
                            "silver"
                        ]
                    },
                    "backdrop": {
                        "allowed_values": [
                            "blue",
                            "red"
                        ]
                    }
                }
            },
            {
                "id": "cat",
                "name": "Cat"
            }
        ],
        "attributes": [
            {
                "id": "backdrop",
                "name": "Backdrop",
                "value_type": "enum"
            },
            {
                "id": "model",
                "name": "Model",
                "value_type": "enum"
            }
        ],
        "attribute_values": [
            {
                "id": "gold",
                "name": "Gold",
                "attribute_id": "model"
            },
            {
                "id": "silver",
                "name": "Silver",
                "attribute_id": "model"
            },
            {
                "id": "blue",
                "name": "Blue",
                "attribute_id": "backdrop"
            },
            {
                "id": "red",
                "name": "Red",
                "attribute_id": "backdrop"
            }
        ]
    }))
    .unwrap();
    let mut state = MarketState::new(Arc::new(
        Catalog::from_entries(definition.into_entries()).unwrap(),
    ));
    state
        .apply(&[
            Mutation::Upsert(feed("a", FeedState::Ready)),
            Mutation::Upsert(feed("b", FeedState::Ready)),
        ])
        .unwrap();
    state
}
fn feed(id: &str, state: FeedState) -> Record {
    Record::Feed(FeedStatus {
        feed_id: id.parse().unwrap(),
        venue_id: "venue".parse().unwrap(),
        state,
        last_sync_at: Some(now()),
    })
}
fn ask(feed: &str, id: &str, item: &str, currency: &str, amount: &str) -> Record {
    Record::Order(market_view::Order {
        feed_id: feed.parse().unwrap(),
        item_type_id: "bear".parse().unwrap(),
        observed_at: now(),
        data: market_view::OrderData {
            amounts: market_view::PriceQuantity {
                price: Money::parse(currency.parse().unwrap(), amount).unwrap(),
                quantity: market_view::Quantity::ONE,
            },
            order_id: id.parse().unwrap(),
            meta: [("url".into(), String::new())].into(),
            expires_at: None,
            kind: market_view::OrderKind::AskItem {
                item_id: item.parse().unwrap(),
            },
        },
    })
}
fn bid(feed: &str, id: &str, amount: &str, target: OrderKind, expires: Option<i64>) -> Record {
    let quantity = if matches!(target, OrderKind::BidItem { .. }) {
        1
    } else {
        3
    };
    Record::Order(market_view::Order {
        feed_id: feed.parse().unwrap(),
        item_type_id: "bear".parse().unwrap(),
        observed_at: MarketTimestamp::from_millis(1_000).unwrap(),
        data: market_view::OrderData {
            amounts: market_view::PriceQuantity {
                price: Money::parse("TON".parse().unwrap(), amount).unwrap(),
                quantity: quantity
                    .to_string()
                    .parse::<market_view::Quantity>()
                    .unwrap(),
            },
            order_id: id.parse().unwrap(),
            expires_at: expires.map(|t| MarketTimestamp::from_millis(t).unwrap()),
            meta: Default::default(),
            kind: target,
        },
    })
}
fn all_target() -> OrderKind {
    OrderKind::BidAny
}
fn book(currency: &str) -> BookId {
    BookId {
        item_type_id: "bear".parse().unwrap(),
        currency_id: currency.parse().unwrap(),
    }
}
fn page(state: &MarketState, side: BookSide, currency: &str, filter: &BookFilter) -> Vec<Order> {
    let page = state
        .book_page(BookQuery {
            book: &book(currency),
            side,
            filter,
            after: None,
            now: now(),
            limit: 200,
            max_scan: 100_000,
        })
        .unwrap();
    assert!(page.next_after.is_none());
    page.orders
}
fn amounts(records: &[Order]) -> Vec<String> {
    records
        .iter()
        .map(|order| order.amounts.price.amount.to_string())
        .collect()
}
#[test]
fn pooled_bid_matches_alternatives_without_expanding_quantity_and_updates_atomically() {
    let mut state = initial();
    let filter: AttributeFilter = [
        (
            "model".parse().unwrap(),
            [
                market_view::AttributeValue::Enum("gold".parse().unwrap()),
                market_view::AttributeValue::Enum("silver".parse().unwrap()),
            ]
            .into(),
        ),
        (
            "backdrop".parse().unwrap(),
            [market_view::AttributeValue::Enum("blue".parse().unwrap())].into(),
        ),
    ]
    .into();
    let pooled = bid(
        "b",
        "pooled",
        "12",
        OrderKind::BidAttributes {
            filter: filter.clone(),
        },
        None,
    );
    let candidates = [
        ("gold-blue", Some("gold"), Some("blue"), true),
        ("silver-blue", Some("silver"), Some("blue"), true),
        ("gold-red", Some("gold"), Some("red"), false),
        ("silver-unknown", Some("silver"), None, false),
        ("unknown-blue", None, Some("blue"), false),
    ];
    let mut mutations = vec![Mutation::Upsert(pooled.clone())];
    for (id, model, backdrop, _) in candidates {
        let attributes = [
            ("model".parse().unwrap(), model),
            ("backdrop".parse().unwrap(), backdrop),
        ]
        .into_iter()
        .filter_map(|(kind, value)| {
            value.map(|v| (kind, market_view::AttributeValue::Enum(v.parse().unwrap())))
        })
        .collect();
        mutations.push(Mutation::Upsert(Record::Item(market_view::Item {
            feed_id: "a".parse().unwrap(),
            item_type_id: "bear".parse().unwrap(),
            observed_at: now(),
            data: market_view::ItemData {
                item_id: id.parse().unwrap(),
                attributes,
            },
        })));
    }
    state.apply(&mutations).unwrap();
    let rows = page(&state, BookSide::Bid, "TON", &BookFilter::default());
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].amounts.quantity.to_string(), "3");
    for (id, _, _, matches) in candidates {
        let query = BookFilter {
            item_id: Some(id.parse().unwrap()),
            ..Default::default()
        };
        assert_eq!(
            !page(&state, BookSide::Bid, "TON", &query).is_empty(),
            matches,
            "{id}"
        );
        let attributes = state
            .item_observation(query.item_id.as_ref().unwrap())
            .unwrap()
            .attributes
            .clone();
        assert_eq!(
            !page(
                &state,
                BookSide::Bid,
                "TON",
                &BookFilter {
                    attributes,
                    ..Default::default()
                }
            )
            .is_empty(),
            matches,
            "hypothetical {id}"
        );
    }
    let before = state.clone();
    let mut updated = pooled.clone();
    let Record::Order(order) = &mut updated else {
        unreachable!()
    };
    order.amounts.price = Money::parse("TON".parse().unwrap(), "15").unwrap();
    order.amounts.quantity = "2".parse::<market_view::Quantity>().unwrap();
    order.kind = OrderKind::BidAttributes {
        filter: [
            (
                "model".parse().unwrap(),
                [market_view::AttributeValue::Enum("silver".parse().unwrap())].into(),
            ),
            (
                "backdrop".parse().unwrap(),
                [market_view::AttributeValue::Enum("blue".parse().unwrap())].into(),
            ),
        ]
        .into(),
    };
    // Reject the entire batch, including the otherwise valid price/selector update.
    let mut invalid = filter;
    let market_view::AttributePredicate::OneOf { values } = invalid.get_mut("model").unwrap()
    else {
        unreachable!()
    };
    values.insert(market_view::AttributeValue::Enum(
        "unknown".parse().unwrap(),
    ));
    assert!(
        state
            .apply(&[
                Mutation::Upsert(updated.clone()),
                Mutation::Upsert(bid(
                    "b",
                    "invalid",
                    "1",
                    OrderKind::BidAttributes { filter: invalid },
                    None
                ))
            ])
            .is_err()
    );
    assert_eq!(
        page(&state, BookSide::Bid, "TON", &BookFilter::default()),
        rows
    );
    state.apply(&[Mutation::Upsert(updated)]).unwrap();
    let gold = BookFilter {
        item_id: Some("gold-blue".parse().unwrap()),
        ..Default::default()
    };
    let silver = BookFilter {
        item_id: Some("silver-blue".parse().unwrap()),
        ..Default::default()
    };
    assert!(page(&state, BookSide::Bid, "TON", &gold).is_empty());
    assert_eq!(
        page(&state, BookSide::Bid, "TON", &silver)[0]
            .amounts
            .quantity
            .to_string(),
        "2"
    );
    assert_eq!(
        amounts(&page(&state, BookSide::Bid, "TON", &silver)),
        ["15"]
    );
    assert_eq!(page(&before, BookSide::Bid, "TON", &gold).len(), 1);
    state.apply(&[Mutation::Delete(pooled.key())]).unwrap();
    assert!(page(&state, BookSide::Bid, "TON", &silver).is_empty());
}
#[test]
fn price_updates_currency_moves_deletes_and_invalid_batches_are_atomic() {
    let mut state = initial();
    let filter = BookFilter::default();
    let high = ask("a", "a-high", "item-1", "TON", "100");
    let low = ask("b", "z-low", "item-2", "TON", "1");
    state
        .apply(&[
            Mutation::Upsert(high.clone()),
            Mutation::Upsert(low.clone()),
            Mutation::Upsert(ask("a", "usd", "item-3", "USD", "0.01")),
            Mutation::Upsert(bid("a", "b-low", "10", all_target(), None)),
            Mutation::Upsert(bid("a", "b-high", "50", all_target(), None)),
        ])
        .unwrap();
    assert_eq!(
        amounts(&page(&state, BookSide::Ask, "TON", &filter)),
        ["1", "100"]
    );
    assert_eq!(
        amounts(&page(&state, BookSide::Bid, "TON", &filter)),
        ["50", "10"]
    );
    assert_eq!(
        amounts(&page(&state, BookSide::Ask, "USD", &filter)),
        ["0.01"]
    );
    let snapshot = state.clone();
    let changed = ask("b", "z-low", "item-2", "USD", "2");
    state
        .apply(&[
            Mutation::Delete(high.key()),
            Mutation::Upsert(changed.clone()),
        ])
        .unwrap();
    assert!(page(&state, BookSide::Ask, "TON", &filter).is_empty());
    assert_eq!(
        amounts(&page(&snapshot, BookSide::Ask, "TON", &filter)),
        ["1", "100"]
    );
    let mut invalid = changed.clone();
    let Record::Order(a) = &mut invalid else {
        unreachable!()
    };
    a.item_type_id = "cat".parse().unwrap();
    // The unchanged bid below still references this item's original type.
    state
        .apply(&[Mutation::Upsert(bid(
            "a",
            "specific",
            "3",
            OrderKind::BidItem {
                item_id: "item-2".parse().unwrap(),
            },
            None,
        ))])
        .unwrap();
    let before = page(&state, BookSide::Ask, "USD", &filter);
    assert!(
        state
            .apply(&[
                Mutation::Delete(ask("a", "usd", "item-3", "USD", "1").key()),
                Mutation::Upsert(invalid)
            ])
            .is_err()
    );
    assert_eq!(page(&state, BookSide::Ask, "USD", &filter), before);
    state.apply(&[Mutation::Delete(changed.key())]).unwrap();
    assert_eq!(
        amounts(&page(&state, BookSide::Ask, "USD", &filter)),
        ["0.01"]
    );
}
#[test]
fn expiry_and_every_non_ready_feed_state_are_excluded_without_deleting_observations() {
    let mut state = initial();
    let filter = BookFilter::default();
    let mut expired_ask = ask("a", "expired-ask", "item", "TON", "0.1");
    let Record::Order(order) = &mut expired_ask else {
        unreachable!()
    };
    order.observed_at = MarketTimestamp::from_millis(1_000).unwrap();
    order.expires_at = Some(now());
    state
        .apply(&[
            Mutation::Upsert(expired_ask),
            Mutation::Upsert(ask("a", "ask", "item", "TON", "1")),
            Mutation::Upsert(bid("a", "expired", "100", all_target(), Some(9_999))),
            Mutation::Upsert(bid("a", "at-boundary", "90", all_target(), Some(10_000))),
            Mutation::Upsert(bid("a", "live", "10", all_target(), Some(10_001))),
        ])
        .unwrap();
    assert_eq!(
        amounts(&page(&state, BookSide::Bid, "TON", &filter)),
        ["10"]
    );
    assert_eq!(amounts(&page(&state, BookSide::Ask, "TON", &filter)), ["1"]);
    let count = state.len();
    for status in [FeedState::Syncing, FeedState::Stale, FeedState::Unavailable] {
        state.apply(&[Mutation::Upsert(feed("a", status))]).unwrap();
        assert!(page(&state, BookSide::Ask, "TON", &filter).is_empty());
        assert!(page(&state, BookSide::Bid, "TON", &filter).is_empty());
        assert_eq!(state.len(), count);
    }
    state
        .apply(&[Mutation::Upsert(feed("a", FeedState::Ready))])
        .unwrap();
    assert_eq!(page(&state, BookSide::Ask, "TON", &filter).len(), 1);
    let expired = state
        .book_page(BookQuery {
            book: &book("TON"),
            side: BookSide::Bid,
            filter: &filter,
            after: None,
            now: MarketTimestamp::from_millis(10_001).unwrap(),
            limit: 200,
            max_scan: 100,
        })
        .unwrap();
    assert!(expired.orders.is_empty());
}
#[test]
fn item_and_attribute_targets_use_one_ready_metadata_observation() {
    let mut state = initial();
    let gold: Attributes = [(
        "model".parse().unwrap(),
        market_view::AttributeValue::Enum("gold".parse().unwrap()),
    )]
    .into();
    let item = market_view::Item {
        feed_id: "a".parse().unwrap(),
        item_type_id: "bear".parse().unwrap(),
        observed_at: now(),
        data: market_view::ItemData {
            item_id: "item".parse().unwrap(),
            attributes: gold.clone(),
        },
    };
    state
        .apply(&[
            Mutation::Upsert(Record::Item(item.clone())),
            Mutation::Upsert(ask("b", "ask", "item", "TON", "1")),
            Mutation::Upsert(bid("b", "all", "1", all_target(), None)),
            Mutation::Upsert(bid(
                "b",
                "specific",
                "2",
                OrderKind::BidItem {
                    item_id: item.item_id.clone(),
                },
                None,
            )),
            Mutation::Upsert(bid(
                "b",
                "attr",
                "3",
                OrderKind::BidAttributes {
                    filter: {
                        let attrs: Attributes = gold.clone();
                        attrs
                            .into_iter()
                            .map(|(kind, value)| (kind, [value].into()))
                            .collect()
                    },
                },
                None,
            )),
            Mutation::Upsert(bid(
                "b",
                "other",
                "100",
                OrderKind::BidItem {
                    item_id: "other".parse().unwrap(),
                },
                None,
            )),
        ])
        .unwrap();
    let by_item = BookFilter {
        item_id: Some(item.item_id.clone()),
        ..BookFilter::default()
    };
    assert_eq!(
        amounts(&page(&state, BookSide::Bid, "TON", &by_item)),
        ["3", "2", "1"]
    );
    let attrs = BookFilter {
        attributes: gold,
        ..BookFilter::default()
    };
    assert_eq!(page(&state, BookSide::Ask, "TON", &attrs).len(), 1);
    assert_eq!(
        amounts(&page(&state, BookSide::Bid, "TON", &attrs)),
        ["3", "1"]
    );
    let mut newer = item.clone();
    newer.feed_id = "b".parse().unwrap();
    newer.observed_at = MarketTimestamp::from_millis(10_001).unwrap();
    newer.attributes.clear();
    state
        .apply(&[Mutation::Upsert(Record::Item(newer))])
        .unwrap();
    assert_eq!(
        amounts(&page(&state, BookSide::Bid, "TON", &by_item)),
        ["2", "1"]
    );
    assert!(page(&state, BookSide::Ask, "TON", &attrs).is_empty());
    state
        .apply(&[Mutation::Upsert(feed("b", FeedState::Unavailable))])
        .unwrap();
    assert_eq!(
        state.item_observation(&item.item_id).unwrap().feed_id,
        item.feed_id
    );
    state
        .apply(&[Mutation::Delete(Record::Item(item.clone()).key())])
        .unwrap();
    assert!(state.item_observation(&item.item_id).is_none());
}
#[test]
fn bounded_pages_have_stable_ties_and_reject_cross_book_cursors() {
    let mut state = initial();
    let filter = BookFilter::default();
    state
        .apply(&[
            Mutation::Upsert(ask("a", "first", "g1", "TON", "1")),
            Mutation::Upsert(feed("a", FeedState::Stale)),
            Mutation::Upsert(ask("b", "a", "g2", "TON", "1.0")),
            Mutation::Upsert(ask("b", "z", "g3", "TON", "1.00")),
        ])
        .unwrap();
    let id = book("TON");
    let mut after = None;
    let mut keys = Vec::new();
    let mut empty_with_progress = false;
    loop {
        let page = state
            .book_page(BookQuery {
                book: &id,
                side: BookSide::Ask,
                filter: &filter,
                after: after.as_ref(),
                now: now(),
                limit: 1,
                max_scan: 1,
            })
            .unwrap();
        empty_with_progress |= page.orders.is_empty() && page.next_after.is_some();
        keys.extend(page.orders.iter().map(Order::key));
        let Some(next) = page.next_after else {
            break;
        };
        after = Some(next);
    }
    assert!(empty_with_progress);
    assert_eq!(
        keys,
        vec![
            ask("b", "a", "g2", "TON", "1").key(),
            ask("b", "z", "g3", "TON", "1").key()
        ]
    );
    let invalid = BookCursor {
        book: id,
        side: BookSide::Ask,
        price: "1".parse().unwrap(),
        key: keys[0].clone(),
    };
    assert!(
        state
            .book_page(BookQuery {
                book: &book("USD"),
                side: BookSide::Ask,
                filter: &filter,
                after: Some(&invalid),
                now: now(),
                limit: 1,
                max_scan: 1
            })
            .is_err()
    );
}
#[test]
fn indexes_match_an_independent_sorted_scan_after_many_deltas_and_snapshot_rebuild() {
    let mut state = initial();
    let filter = BookFilter::default();
    for step in 0..240 {
        let feed_id = if step % 2 == 0 { "a" } else { "b" };
        let id = format!("order-{}", step % 37);
        let record = if step % 3 == 0 {
            bid(
                feed_id,
                &id,
                &format!("{}", step % 23 + 1),
                all_target(),
                Some(9_990 + step % 20),
            )
        } else {
            ask(
                feed_id,
                &id,
                &format!("item-{}", step % 7),
                if step % 5 == 0 { "USD" } else { "TON" },
                &format!("{}", step % 23 + 1),
            )
        };
        let mutation = if step % 11 == 0 {
            Mutation::Delete(record.key())
        } else {
            Mutation::Upsert(record)
        };
        state
            .apply(&[
                mutation,
                Mutation::Upsert(feed(
                    feed_id,
                    if step % 13 == 0 {
                        FeedState::Stale
                    } else {
                        FeedState::Ready
                    },
                )),
            ])
            .unwrap();
        for currency in ["TON", "USD"] {
            for side in [BookSide::Ask, BookSide::Bid] {
                let mut expected: Vec<_> = state
                    .iter()
                    .filter_map(|(_, r)| {
                        let Record::Order(order) = r else {return None;};
                        if order.side() != side || order.expires_at.is_some_and(|expiry| expiry <= now()) {return None;}
                        let price = &order.amounts.price;
                        let feed_id = r.key().feed_id;
                        let ready = matches!(state.get(&RecordKey {feed_id:feed_id.clone(),id:RecordId::Feed(feed_id)}),Some(Record::Feed(f)) if f.state == FeedState::Ready);
                        if price.currency_id.as_str() != currency || !ready {
                            return None;
                        }
                        Some((
                            if side == BookSide::Ask {
                                price.amount
                            } else {
                                -price.amount
                            },
                            r.key(),
                        ))
                    })
                    .collect();
                expected.sort();
                assert_eq!(
                    page(&state, side, currency, &filter)
                        .iter()
                        .map(Order::key)
                        .collect::<Vec<_>>(),
                    expected.into_iter().map(|(_, k)| k).collect::<Vec<_>>()
                );
            }
        }
    }
    let mut rebuilt = MarketState::new(state.catalog().clone());
    rebuilt
        .apply(
            &state
                .iter()
                .map(|(_, r)| Mutation::Upsert(r.clone()))
                .collect::<Vec<_>>(),
        )
        .unwrap();
    for side in [BookSide::Ask, BookSide::Bid] {
        assert_eq!(
            page(&state, side, "TON", &filter),
            page(&rebuilt, side, "TON", &filter)
        );
    }
}

fn units(id: &str, amount: &str, quantity: u32, attributes: Attributes) -> Record {
    let Record::Order(mut order) = ask("b", id, "unused", "TON", amount) else {
        unreachable!()
    };
    order.amounts.quantity = quantity
        .to_string()
        .parse::<market_view::Quantity>()
        .unwrap();
    order.kind = OrderKind::AskUnits { attributes };
    Record::Order(order)
}
#[test]
fn unit_asks_sort_by_unit_price_and_use_only_their_own_known_traits() {
    let mut state = initial();
    let attributes: Attributes = [
        (
            "model".parse().unwrap(),
            AttributeValue::Enum("gold".parse().unwrap()),
        ),
        (
            "backdrop".parse().unwrap(),
            AttributeValue::Enum("blue".parse().unwrap()),
        ),
    ]
    .into();
    let mut item = market_view::Item {
        feed_id: "a".parse().unwrap(),
        item_type_id: "bear".parse().unwrap(),
        observed_at: now(),
        data: market_view::ItemData {
            item_id: "identified".parse().unwrap(),
            attributes: attributes.clone(),
        },
    };
    let full = units("full", "1", 100, attributes.clone());
    let partial = units(
        "partial",
        "0.5",
        5,
        [(
            "model".parse().unwrap(),
            AttributeValue::Enum("gold".parse().unwrap()),
        )]
        .into(),
    );
    let mut expired = units("expired", "0.01", 1000, attributes);
    let Record::Order(order) = &mut expired else {
        unreachable!()
    };
    order.observed_at = MarketTimestamp::from_millis(1000).unwrap();
    order.expires_at = Some(now());
    state
        .apply(&[
            Mutation::Upsert(Record::Item(item.clone())),
            Mutation::Upsert(ask("a", "specific", "identified", "TON", "2")),
            Mutation::Upsert(full.clone()),
            Mutation::Upsert(partial),
            Mutation::Upsert(expired),
        ])
        .unwrap();
    let all = page(&state, BookSide::Ask, "TON", &BookFilter::default());
    assert_eq!(amounts(&all), ["0.5", "1", "2"]);
    assert_eq!(
        all.iter()
            .map(|o| o.amounts.quantity.to_string())
            .collect::<Vec<_>>(),
        ["5", "100", "1"]
    );
    let blue = BookFilter {
        attributes: [(
            "backdrop".parse().unwrap(),
            AttributeValue::Enum("blue".parse().unwrap()),
        )]
        .into(),
        ..Default::default()
    };
    assert_eq!(
        amounts(&page(&state, BookSide::Ask, "TON", &blue)),
        ["1", "2"]
    );
    let specific = BookFilter {
        item_id: Some(item.item_id.clone()),
        ..Default::default()
    };
    assert_eq!(
        amounts(&page(&state, BookSide::Ask, "TON", &specific)),
        ["2"]
    );
    let first = state
        .book_page(BookQuery {
            book: &book("TON"),
            side: BookSide::Ask,
            filter: &blue,
            after: None,
            now: now(),
            limit: 1,
            max_scan: 100,
        })
        .unwrap();
    assert_eq!(first.orders[0].amounts.quantity.to_string(), "100");
    let second = state
        .book_page(BookQuery {
            book: &book("TON"),
            side: BookSide::Ask,
            filter: &blue,
            after: first.next_after.as_ref(),
            now: now(),
            limit: 1,
            max_scan: 100,
        })
        .unwrap();
    assert_eq!(amounts(&second.orders), ["2"]);
    // Changing an identified observation must not change a unit offer's source-supplied traits.
    item.attributes.insert(
        "backdrop".parse().unwrap(),
        AttributeValue::Enum("red".parse().unwrap()),
    );
    state
        .apply(&[Mutation::Upsert(Record::Item(item))])
        .unwrap();
    assert_eq!(amounts(&page(&state, BookSide::Ask, "TON", &blue)), ["1"]);
    state
        .apply(&[Mutation::Upsert(feed("b", FeedState::Stale))])
        .unwrap();
    assert!(page(&state, BookSide::Ask, "TON", &blue).is_empty());
    assert_eq!(
        amounts(&page(&state, BookSide::Ask, "TON", &specific)),
        ["2"]
    );
}
#[test]
fn quantity_updates_offer_changes_and_removals_keep_one_atomic_index_entry() {
    let mut state = initial();
    let original = units("stock", "5", 100, Default::default());
    state.apply(&[Mutation::Upsert(original.clone())]).unwrap();
    let snapshot = state.clone();
    let mut changed = original.clone();
    let Record::Order(order) = &mut changed else {
        unreachable!()
    };
    order.amounts.quantity = "50".parse::<market_view::Quantity>().unwrap();
    state.apply(&[Mutation::Upsert(changed.clone())]).unwrap();
    assert_eq!(
        page(&state, BookSide::Ask, "TON", &BookFilter::default()).len(),
        1
    );
    assert_eq!(
        page(&state, BookSide::Ask, "TON", &BookFilter::default())[0]
            .amounts
            .quantity
            .to_string(),
        "50"
    );
    assert_eq!(
        page(&snapshot, BookSide::Ask, "TON", &BookFilter::default())[0]
            .amounts
            .quantity
            .to_string(),
        "100"
    );
    let mut invalid = changed.clone();
    let Record::Order(order) = &mut invalid else {
        unreachable!()
    };
    order.kind = OrderKind::AskItem {
        item_id: "identified".parse().unwrap(),
    };
    assert!(state.apply(&[Mutation::Upsert(invalid.clone())]).is_err());
    let Record::Order(order) = &mut invalid else {
        unreachable!()
    };
    order.amounts.quantity = market_view::Quantity::ONE;
    state.apply(&[Mutation::Upsert(invalid)]).unwrap();
    let specific = BookFilter {
        item_id: Some("identified".parse().unwrap()),
        ..Default::default()
    };
    assert_eq!(page(&state, BookSide::Ask, "TON", &specific).len(), 1);
    state.apply(&[Mutation::Upsert(changed.clone())]).unwrap();
    assert!(page(&state, BookSide::Ask, "TON", &specific).is_empty());
    let invalid = units(
        "invalid",
        "1",
        2,
        [("model".parse().unwrap(), AttributeValue::Integer(1))].into(),
    );
    assert!(
        state
            .apply(&[Mutation::Delete(changed.key()), Mutation::Upsert(invalid)])
            .is_err()
    );
    assert_eq!(
        page(&state, BookSide::Ask, "TON", &BookFilter::default())[0]
            .amounts
            .quantity
            .to_string(),
        "50"
    );
    let Record::Order(order) = &mut changed else {
        unreachable!()
    };
    order.amounts.price = Money::parse("USD".parse().unwrap(), "0.1").unwrap();
    state.apply(&[Mutation::Upsert(changed.clone())]).unwrap();
    assert!(page(&state, BookSide::Ask, "TON", &BookFilter::default()).is_empty());
    assert_eq!(
        page(&state, BookSide::Ask, "USD", &BookFilter::default())[0]
            .amounts
            .quantity
            .to_string(),
        "50"
    );
    state.apply(&[Mutation::Delete(changed.key())]).unwrap();
    assert!(page(&state, BookSide::Ask, "USD", &BookFilter::default()).is_empty());
}

#[test]
fn fractional_orders_updates_and_invalid_grid_batches_share_the_same_indexes() {
    let original = initial();
    let entries = original.catalog().entries().map(|mut entry| {
        if let CatalogEntry::ItemType(kind) = &mut entry
            && kind.id.as_str() == "bear"
        {
            kind.quantity_step = "0.01".parse().unwrap();
        }
        entry
    });
    let mut state = MarketState::new(Arc::new(Catalog::from_entries(entries).unwrap()));
    let mut sell = ask("a", "fraction-sell", "identified", "TON", "100");
    let Record::Order(order) = &mut sell else {
        unreachable!()
    };
    order.amounts.quantity = "0.75".parse().unwrap();
    let mut buy = bid(
        "b",
        "fraction-buy",
        "90",
        OrderKind::BidItem {
            item_id: "identified".parse().unwrap(),
        },
        None,
    );
    let Record::Order(order) = &mut buy else {
        unreachable!()
    };
    order.amounts.quantity = "0.25".parse().unwrap();
    state
        .apply(&[
            Mutation::Upsert(feed("a", FeedState::Ready)),
            Mutation::Upsert(feed("b", FeedState::Ready)),
            Mutation::Upsert(sell.clone()),
            Mutation::Upsert(buy.clone()),
        ])
        .unwrap();
    let snapshot = state.clone();
    let specific = BookFilter {
        item_id: Some("identified".parse().unwrap()),
        ..Default::default()
    };
    assert_eq!(
        page(&state, BookSide::Ask, "TON", &specific)[0]
            .amounts
            .quantity
            .to_string(),
        "0.75"
    );
    assert_eq!(
        page(&state, BookSide::Bid, "TON", &specific)[0]
            .amounts
            .quantity
            .to_string(),
        "0.25"
    );
    let Record::Order(order) = &mut sell else {
        unreachable!()
    };
    order.amounts.quantity = "0.5".parse().unwrap();
    let mut invalid = buy.clone();
    let Record::Order(order) = &mut invalid else {
        unreachable!()
    };
    order.amounts.quantity = "0.251".parse().unwrap();
    assert!(
        state
            .apply(&[Mutation::Upsert(sell.clone()), Mutation::Upsert(invalid)])
            .is_err()
    );
    assert_eq!(
        page(&state, BookSide::Ask, "TON", &specific)[0]
            .amounts
            .quantity
            .to_string(),
        "0.75"
    );
    let Record::Order(order) = &mut buy else {
        unreachable!()
    };
    order.amounts.quantity = "0.2".parse().unwrap();
    state
        .apply(&[Mutation::Upsert(sell.clone()), Mutation::Upsert(buy)])
        .unwrap();
    assert_eq!(
        page(&state, BookSide::Ask, "TON", &specific)[0]
            .amounts
            .quantity
            .to_string(),
        "0.5"
    );
    assert_eq!(
        page(&state, BookSide::Bid, "TON", &specific)[0]
            .amounts
            .quantity
            .to_string(),
        "0.2"
    );
    assert_eq!(
        page(&snapshot, BookSide::Ask, "TON", &specific)[0]
            .amounts
            .quantity
            .to_string(),
        "0.75"
    );
    state.apply(&[Mutation::Delete(sell.key())]).unwrap();
    assert!(page(&state, BookSide::Ask, "TON", &specific).is_empty());
}

#[test]
fn item_observations_and_order_references_share_one_atomic_item_lifecycle() {
    let mut state = initial();
    let item = market_view::Item {
        feed_id: "a".parse().unwrap(),
        item_type_id: "bear".parse().unwrap(),
        observed_at: now(),
        data: market_view::ItemData {
            item_id: "shared".parse().unwrap(),
            attributes: Default::default(),
        },
    };
    let mut second = item.clone();
    second.feed_id = "b".parse().unwrap();
    let sell = ask("a", "sell", "shared", "TON", "1");
    let buy = bid(
        "b",
        "buy",
        "2",
        OrderKind::BidItem {
            item_id: item.item_id.clone(),
        },
        None,
    );
    state
        .apply(&[
            Mutation::Upsert(Record::Item(item.clone())),
            Mutation::Upsert(Record::Item(second.clone())),
            Mutation::Upsert(sell.clone()),
            Mutation::Upsert(buy.clone()),
        ])
        .unwrap();
    let snapshot = state.clone();
    state
        .apply(&[
            Mutation::Delete(Record::Item(item.clone()).key()),
            Mutation::Delete(Record::Item(second).key()),
        ])
        .unwrap();
    assert!(state.item_observation(&item.item_id).is_none());
    let mut changed = item.clone();
    changed.item_type_id = "cat".parse().unwrap();
    assert!(
        state
            .apply(&[Mutation::Upsert(Record::Item(changed.clone()))])
            .is_err()
    );
    assert_eq!(
        page(&state, BookSide::Ask, "TON", &BookFilter::default()).len(),
        1
    );
    // Removing all old references permits a complete atomic type correction.
    state
        .apply(&[
            Mutation::Delete(sell.key()),
            Mutation::Upsert(Record::Item(changed.clone())),
            Mutation::Delete(buy.key()),
        ])
        .unwrap();
    assert_eq!(
        state.item_observation(&item.item_id).unwrap().item_type_id,
        changed.item_type_id
    );
    assert!(page(&state, BookSide::Ask, "TON", &BookFilter::default()).is_empty());
    assert_eq!(
        snapshot
            .item_observation(&item.item_id)
            .unwrap()
            .item_type_id,
        item.item_type_id
    );
    assert_eq!(
        page(&snapshot, BookSide::Ask, "TON", &BookFilter::default()).len(),
        1
    );
    // The last metadata removal releases the item ID for another complete observation.
    state
        .apply(&[
            Mutation::Delete(Record::Item(changed).key()),
            Mutation::Upsert(sell),
        ])
        .unwrap();
    assert_eq!(
        page(&state, BookSide::Ask, "TON", &BookFilter::default()).len(),
        1
    );
}
