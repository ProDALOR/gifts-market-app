use super::*;
use market_gifts_hub::{BACKDROP, MODEL, PATTERN};
use market_hub_protocol::wire as w;
use w::entity::Entity as E;

fn upsert(entity: E) -> w::Change {
    w::Change {
        change: Some(w::change::Change::Upsert(w::Entity {
            entity: Some(entity),
        })),
    }
}
fn remove(kind: w::EntityKind, id: &str) -> w::Change {
    w::Change {
        change: Some(w::change::Change::Remove(w::Removal {
            key: Some(w::Key {
                kind: kind as i32,
                id: id.into(),
                venue_id: if kind == w::EntityKind::Order {
                    "venue".into()
                } else {
                    String::new()
                },
            }),
            ..Default::default()
        })),
    }
}
fn item(name: &str, model: Option<&str>) -> E {
    E::ItemType(w::ItemType {
        id: "gift".into(),
        name: name.into(),
        quantity_step: "1".into(),
        attributes: [
            (
                MODEL.into(),
                w::AttributeRule {
                    required: true,
                    allowed_values: model.map(|id| w::AllowedAttributeValues {
                        ids: vec![id.into()],
                    }),
                },
            ),
            (
                BACKDROP.into(),
                w::AttributeRule {
                    required: false,
                    allowed_values: Some(w::AllowedAttributeValues {
                        ids: vec!["backdrop".into()],
                    }),
                },
            ),
            (
                PATTERN.into(),
                w::AttributeRule {
                    required: false,
                    allowed_values: Some(w::AllowedAttributeValues {
                        ids: vec!["pattern".into()],
                    }),
                },
            ),
        ]
        .into(),
        ..Default::default()
    })
}
fn model(name: &str) -> E {
    E::AttributeOption(w::AttributeOption {
        id: "model".into(),
        name: name.into(),
        attribute_id: MODEL.into(),
        aliases: vec![w::ExternalId {
            namespace: "tg".into(),
            id: "123".into(),
        }],
        ..Default::default()
    })
}
fn venue(name: &str) -> E {
    E::Venue(w::Venue {
        id: "venue".into(),
        name: name.into(),
    })
}
fn currency(name: &str) -> E {
    E::Currency(w::Currency {
        id: "GRAM".into(),
        name: name.into(),
        decimals: 9,
    })
}
fn order(id: &str, currency: &str, price: &str) -> E {
    E::Order(w::Order {
        id: id.into(),
        book: Some(w::BookKey {
            item_type_id: "gift".into(),
            currency_id: currency.into(),
            venue_id: "venue".into(),
        }),
        mode: w::OrderMode::Ask as i32,
        price: price.into(),
        quantity: "1".into(),
        ..Default::default()
    })
}
fn seed() -> Vec<w::Change> {
    vec![
        item("Gift", Some("model")),
        model("Original"),
        venue("Venue"),
        currency("Gram"),
        order("order", "GRAM", "1"),
    ]
    .into_iter()
    .map(upsert)
    .collect()
}
struct Fixture {
    directory: tempfile::TempDir,
    writer: market_replica::ReplicaWriter,
    cache: ProjectionCache,
    sequence: usize,
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let mut writer = market_replica::ReplicaWriter::open(directory.path(), "cache").unwrap();
        let mut snapshot = writer.begin_snapshot().unwrap();
        snapshot.append(&seed()).unwrap();
        writer
            .install_snapshot(snapshot.finish("seed").unwrap())
            .unwrap();
        Self {
            directory,
            writer,
            cache: Default::default(),
            sequence: 0,
        }
    }
    fn apply(&mut self, changes: Vec<w::Change>) {
        self.sequence += 1;
        self.writer
            .apply(w::Batch {
                changes,
                cursor: self.sequence.to_string(),
            })
            .unwrap();
    }
    fn catalog(&self) -> GiftCatalog {
        self.cache.bootstrap(self.writer.reader()).unwrap()
    }
    fn choices(&self) -> GiftCollectionCatalog {
        self.cache
            .collection(self.writer.reader(), "gift".into())
            .unwrap()
    }
}
fn same_catalog(before: &GiftCatalog, after: &GiftCatalog) {
    assert_eq!(before, after);
    assert!(Arc::ptr_eq(&before.collections, &after.collections));
    assert!(Arc::ptr_eq(&before.venues, &after.venues));
    assert!(Arc::ptr_eq(&before.currencies, &after.currencies));
    assert!(Arc::ptr_eq(&before.books, &after.books));
}
fn same_choices(before: &GiftCollectionCatalog, after: &GiftCollectionCatalog) {
    assert_eq!(before, after);
    assert!(Arc::ptr_eq(&before.models, &after.models));
    assert!(Arc::ptr_eq(&before.backdrops, &after.backdrops));
    assert!(Arc::ptr_eq(&before.patterns, &after.patterns));
}
#[test]
fn prices_freshness_and_unused_attribute_definitions_reuse_every_projection() {
    let mut f = Fixture::new();
    let before = f.catalog();
    let choices = f.choices();
    f.writer.mark(market_replica::SyncState::Stale);
    same_catalog(&before, &f.catalog());
    same_choices(&choices, &f.choices());
    f.apply(vec![upsert(order("order", "GRAM", "2"))]);
    f.apply(vec![]); // A coalesced heartbeat cannot invalidate metadata.
    same_catalog(&before, &f.catalog());
    same_choices(&choices, &f.choices());
    f.apply(vec![upsert(E::Attribute(w::Attribute {
        id: "extra".into(),
        name: "Extra".into(),
        value_type: 5,
        ..Default::default()
    }))]);
    same_catalog(&before, &f.catalog());
    same_choices(&choices, &f.choices());
}
#[test]
fn venue_and_currency_changes_invalidate_only_their_own_arrays() {
    let mut f = Fixture::new();
    let before = f.catalog();
    let choices = f.choices();
    f.apply(vec![upsert(venue("Renamed venue"))]);
    let after = f.catalog();
    assert_eq!(after.venues[0].name, "Renamed venue");
    assert!(!Arc::ptr_eq(&before.venues, &after.venues));
    assert!(Arc::ptr_eq(&before.collections, &after.collections));
    assert!(Arc::ptr_eq(&before.currencies, &after.currencies));
    assert!(Arc::ptr_eq(&before.books, &after.books));
    same_choices(&choices, &f.choices());
    f.apply(vec![upsert(currency("Renamed currency"))]);
    let next = f.catalog();
    assert_eq!(next.currencies[0].name, "Renamed currency");
    assert!(!Arc::ptr_eq(&after.currencies, &next.currencies));
    assert!(Arc::ptr_eq(&after.collections, &next.collections));
    assert!(Arc::ptr_eq(&after.venues, &next.venues));
    assert!(Arc::ptr_eq(&after.books, &next.books));
    same_choices(&choices, &f.choices());
    f.apply(vec![
        remove(w::EntityKind::Venue, "venue"),
        remove(w::EntityKind::Currency, "GRAM"),
    ]);
    let removed = f.catalog();
    assert!(removed.venues.is_empty());
    assert!(removed.currencies.is_empty());
    assert!(Arc::ptr_eq(&next.collections, &removed.collections));
    assert!(Arc::ptr_eq(&next.books, &removed.books));
    same_choices(&choices, &f.choices());
}
#[test]
fn book_additions_moves_and_last_order_removal_preserve_gift_projections() {
    let mut f = Fixture::new();
    let before = f.catalog();
    let choices = f.choices();
    f.apply(vec![upsert(order("other", "XTR", "3"))]);
    let added = f.catalog();
    assert_eq!(added.books.len(), 2);
    assert!(!Arc::ptr_eq(&before.books, &added.books));
    assert!(Arc::ptr_eq(&before.collections, &added.collections));
    assert!(Arc::ptr_eq(&before.venues, &added.venues));
    assert!(Arc::ptr_eq(&before.currencies, &added.currencies));
    same_choices(&choices, &f.choices());
    f.apply(vec![upsert(order("other", "GRAM", "3"))]);
    let moved = f.catalog();
    assert_eq!(moved.books.len(), 1);
    assert!(!Arc::ptr_eq(&added.books, &moved.books));
    same_choices(&choices, &f.choices());
    f.apply(vec![remove(w::EntityKind::Order, "other")]);
    same_catalog(&moved, &f.catalog()); // The book still has its first order.
    f.apply(vec![remove(w::EntityKind::Order, "order")]);
    let empty = f.catalog();
    assert!(empty.books.is_empty());
    assert!(!Arc::ptr_eq(&moved.books, &empty.books));
    assert!(Arc::ptr_eq(&before.collections, &empty.collections));
    same_choices(&choices, &f.choices());
}
#[test]
fn model_updates_removals_and_recreation_invalidate_gifts_but_reuse_market_arrays() {
    let mut f = Fixture::new();
    let before = f.catalog();
    let choices = f.choices();
    f.apply(vec![upsert(model("Renamed"))]);
    f.apply(vec![]);
    let renamed = f.catalog();
    let renamed_choices = f.choices();
    assert_eq!(renamed_choices.models[0].name, "Renamed");
    assert!(!Arc::ptr_eq(&choices.models, &renamed_choices.models));
    assert!(!Arc::ptr_eq(&before.collections, &renamed.collections));
    assert!(Arc::ptr_eq(&before.venues, &renamed.venues));
    assert!(Arc::ptr_eq(&before.currencies, &renamed.currencies));
    assert!(Arc::ptr_eq(&before.books, &renamed.books));
    f.apply(vec![upsert(model("Renamed"))]); // Identical observations retain the same revision.
    same_catalog(&renamed, &f.catalog());
    same_choices(&renamed_choices, &f.choices());
    f.apply(vec![remove(w::EntityKind::AttributeOption, "model")]);
    assert!(f.choices().models.is_empty());
    assert!(f.catalog().collections[0].preview.is_none());
    f.apply(vec![upsert(model("Restored"))]);
    assert_eq!(f.choices().models[0].name, "Restored");
    assert!(f.catalog().collections[0].preview.is_some());
}
#[test]
fn collection_membership_and_gift_eligibility_rebuild_the_dependent_projections() {
    let mut f = Fixture::new();
    let before = f.catalog();
    let choices = f.choices();
    let second = E::AttributeOption(w::AttributeOption {
        id: "second".into(),
        name: "Second".into(),
        attribute_id: MODEL.into(),
        ..Default::default()
    });
    f.apply(vec![
        upsert(item("Renamed", Some("second"))),
        upsert(second),
    ]);
    let after = f.catalog();
    let changed = f.choices();
    assert_eq!(after.collections[0].name, "Renamed");
    assert_eq!(changed.collection.name, "Renamed");
    assert_eq!(changed.models[0].id.as_str(), "second");
    assert!(!Arc::ptr_eq(&choices.models, &changed.models));
    assert!(Arc::ptr_eq(&before.venues, &after.venues));
    assert!(Arc::ptr_eq(&before.currencies, &after.currencies));
    f.apply(vec![upsert(E::ItemType(w::ItemType {
        id: "gift".into(),
        name: "Commodity".into(),
        quantity_step: "1".into(),
        ..Default::default()
    }))]);
    assert!(f.catalog().collections.is_empty());
    assert!(f.catalog().books.is_empty()); // A surviving generic book is no longer a gift book.
    assert!(
        f.cache
            .collection(f.writer.reader(), "gift".into())
            .is_err()
    );
    f.apply(vec![upsert(item("Gift", Some("model")))]);
    assert_eq!(f.catalog().books.len(), 1);
    assert_eq!(f.choices().models[0].id.as_str(), "model");
    f.apply(vec![remove(w::EntityKind::ItemType, "gift")]);
    assert!(f.catalog().collections.is_empty());
    assert!(
        f.cache
            .collection(f.writer.reader(), "gift".into())
            .is_err()
    );
    assert!(f.cache.lock().choices.is_empty()); // Failed misses are never retained.
}
#[test]
fn unrestricted_choices_observe_new_options_and_backdrop_pattern_changes() {
    let mut f = Fixture::new();
    f.apply(vec![upsert(item("Gift", None))]);
    let before = f.choices();
    f.apply(vec![
        upsert(E::AttributeOption(w::AttributeOption {
            id: "second".into(),
            attribute_id: MODEL.into(),
            name: "Second".into(),
            ..Default::default()
        })),
        upsert(E::AttributeOption(w::AttributeOption {
            id: "backdrop".into(),
            attribute_id: BACKDROP.into(),
            name: "Blue".into(),
            meta: [
                ("colors.center_color".into(), "#112233".into()),
                ("colors.edge_color".into(), "#223344".into()),
                ("colors.pattern_color".into(), "#334455".into()),
            ]
            .into(),
            ..Default::default()
        })),
        upsert(E::AttributeOption(w::AttributeOption {
            id: "pattern".into(),
            attribute_id: PATTERN.into(),
            name: "Stars".into(),
            aliases: vec![w::ExternalId {
                namespace: "tg".into(),
                id: "456".into(),
            }],
            ..Default::default()
        })),
    ]);
    let after = f.choices();
    assert_eq!(after.models.len(), 2);
    assert!(!Arc::ptr_eq(&before.models, &after.models));
    assert_eq!(
        after.backdrops[0]
            .palette
            .as_ref()
            .unwrap()
            .center
            .to_string(),
        "#112233"
    );
    assert_eq!(
        after.patterns[0].media.as_ref().unwrap().target,
        market_gifts::MediaTarget::Pattern("pattern".into())
    );
    f.apply(vec![
        remove(w::EntityKind::AttributeOption, "backdrop"),
        remove(w::EntityKind::AttributeOption, "pattern"),
    ]);
    let removed = f.choices();
    assert!(removed.backdrops.is_empty());
    assert!(removed.patterns.is_empty());
}
#[test]
fn pinned_reads_never_mix_cache_components_from_different_views() {
    let mut f = Fixture::new();
    let old = f.writer.reader().snapshot().unwrap();
    let before = f.catalog();
    let choices = f.choices();
    f.apply(vec![
        upsert(item("New gift", Some("model"))),
        upsert(model("New model")),
        upsert(venue("New venue")),
        upsert(currency("New currency")),
        upsert(order("order", "XTR", "2")),
    ]);
    let current = f.catalog();
    let current_choices = f.choices();
    assert_eq!(current.collections[0].name, "New gift");
    assert_eq!(current.venues[0].name, "New venue");
    assert_eq!(current.currencies[0].name, "New currency");
    assert_eq!(current.books[0].currency_id.as_str(), "XTR");
    assert_eq!(f.cache.bootstrap(old.clone()).unwrap(), before);
    assert_eq!(f.cache.collection(old, "gift".into()).unwrap(), choices);
    assert_eq!(f.catalog(), current);
    assert_eq!(f.choices(), current_choices);
}
#[test]
fn replacement_and_process_restart_invalidate_every_projection_even_with_identical_data() {
    let mut f = Fixture::new();
    let before = f.catalog();
    let choices = f.choices();
    let mut replacement = f.writer.begin_snapshot().unwrap();
    replacement.append(&seed()).unwrap();
    f.writer
        .install_snapshot(replacement.finish("replaced").unwrap())
        .unwrap();
    let after = f.catalog();
    assert_eq!(before, after);
    assert!(!Arc::ptr_eq(&before.collections, &after.collections));
    assert!(!Arc::ptr_eq(&before.books, &after.books));
    assert!(!Arc::ptr_eq(&before.venues, &after.venues));
    assert!(!Arc::ptr_eq(&before.currencies, &after.currencies));
    assert!(!Arc::ptr_eq(&choices.models, &f.choices().models));
    let Fixture {
        directory,
        writer,
        cache,
        ..
    } = f;
    let before_restart = cache.collection(writer.reader(), "gift".into()).unwrap();
    drop(writer);
    let reopened = market_replica::ReplicaWriter::open(directory.path(), "cache").unwrap();
    let restarted = cache.bootstrap(reopened.reader()).unwrap();
    assert_eq!(after, restarted);
    assert!(!Arc::ptr_eq(&after.collections, &restarted.collections));
    assert!(!Arc::ptr_eq(&after.books, &restarted.books));
    assert!(!Arc::ptr_eq(&after.venues, &restarted.venues));
    assert!(!Arc::ptr_eq(&after.currencies, &restarted.currencies));
    assert!(!Arc::ptr_eq(
        &before_restart.models,
        &cache
            .collection(reopened.reader(), "gift".into())
            .unwrap()
            .models
    ));
}
#[test]
fn failed_component_builds_are_retried_without_poisoning_the_cache() {
    let f = Fixture::new();
    let revision = f
        .writer
        .reader()
        .metadata_revision(&[EntityKind::Venue])
        .unwrap();
    let failed = f.cache.cached(
        |i| &mut i.venues,
        revision,
        || Err(Status::unavailable("temporary failure")),
    );
    assert!(failed.is_err());
    assert_eq!(f.catalog().venues[0].name, "Venue");
    same_catalog(&f.catalog(), &f.catalog());
}
#[test]
fn collection_cache_remains_bounded_and_recently_used_entries_survive() {
    let mut f = Fixture::new();
    let items = (0..9)
        .map(|i| {
            let E::ItemType(mut value) = item("Gift", Some("model")) else {
                unreachable!()
            };
            value.id = format!("gift.{i}");
            upsert(E::ItemType(value))
        })
        .collect();
    f.apply(items);
    let first = f
        .cache
        .collection(f.writer.reader(), "gift.0".into())
        .unwrap();
    for i in 1..8 {
        f.cache
            .collection(f.writer.reader(), format!("gift.{i}").try_into().unwrap())
            .unwrap();
    }
    same_choices(
        &first,
        &f.cache
            .collection(f.writer.reader(), "gift.0".into())
            .unwrap(),
    );
    f.cache
        .collection(f.writer.reader(), "gift.8".into())
        .unwrap();
    let index = f.cache.lock();
    assert_eq!(index.choices.len(), 8);
    assert!(
        index
            .choices
            .iter()
            .any(|entry| entry.id.as_str() == "gift.0")
    );
    assert!(
        !index
            .choices
            .iter()
            .any(|entry| entry.id.as_str() == "gift.1")
    );
}

#[test]
fn feed_availability_changes_rebuild_books_without_evicting_collection_choices() {
    let mut f = Fixture::new();
    let before = f.catalog();
    let choices = f.choices();
    let E::Order(extra) = order("unused", "XTR", "1") else {
        unreachable!()
    };
    let mut feed = w::Feed {
        id: "feed".into(),
        books: vec![extra.book.unwrap()],
        ..Default::default()
    };
    f.apply(vec![upsert(E::Feed(feed.clone()))]);
    let added = f.catalog();
    assert_eq!(added.books.len(), 2);
    assert!(!Arc::ptr_eq(&before.books, &added.books));
    assert!(Arc::ptr_eq(&before.collections, &added.collections));
    same_choices(&choices, &f.choices());
    feed.last_verified_at_ms = Some(1000);
    f.apply(vec![upsert(E::Feed(feed))]);
    same_catalog(&added, &f.catalog());
    f.apply(vec![remove(w::EntityKind::Feed, "feed")]);
    let removed = f.catalog();
    assert_eq!(removed.books, before.books);
    assert!(!Arc::ptr_eq(&added.books, &removed.books));
    same_choices(&choices, &f.choices());
}

#[test]
fn failure_from_an_older_view_does_not_clear_a_newer_cached_projection() {
    use std::{sync::mpsc, time::Duration};
    let mut f = Fixture::new();
    let old_revision = f
        .writer
        .reader()
        .metadata_revision(&[EntityKind::Venue])
        .unwrap();
    let cache = Arc::new(std::mem::take(&mut f.cache));
    let (started, ready) = mpsc::sync_channel(1);
    let (release, resume) = mpsc::sync_channel(1);
    let old_cache = cache.clone();
    let pending = std::thread::spawn(move || {
        old_cache.cached(
            |i| &mut i.venues,
            old_revision,
            || {
                started.send(()).unwrap();
                resume.recv_timeout(Duration::from_secs(5)).unwrap();
                Err(Status::unavailable("older projection failed"))
            },
        )
    });
    ready.recv_timeout(Duration::from_secs(5)).unwrap();
    f.apply(vec![upsert(venue("New venue"))]);
    let current = cache.bootstrap(f.writer.reader()).unwrap();
    release.send(()).unwrap();
    assert!(pending.join().unwrap().is_err());
    same_catalog(&current, &cache.bootstrap(f.writer.reader()).unwrap());
}

fn named_item(id: &str, name: &str, model: Option<&str>) -> E {
    let E::ItemType(mut value) = item(name, model) else {
        unreachable!()
    };
    value.id = id.into();
    E::ItemType(value)
}
fn option(id: &str, attribute: &str, name: &str) -> E {
    E::AttributeOption(w::AttributeOption {
        id: id.into(),
        attribute_id: attribute.into(),
        name: name.into(),
        ..Default::default()
    })
}
fn collection_choices(f: &Fixture, id: &str) -> GiftCollectionCatalog {
    f.cache
        .collection(f.writer.reader(), CollectionId::new(id).unwrap())
        .unwrap()
}
#[test]
fn collection_changes_preserve_unrelated_choices_and_shared_options_refresh_all_dependents() {
    let mut f = Fixture::new();
    f.apply(vec![
        upsert(named_item("other", "Other", Some("other.model"))),
        upsert(option("other.model", MODEL, "Other model")),
        upsert(named_item("shared", "Shared", Some("model"))),
    ]);
    let first = f.choices();
    let shared = collection_choices(&f, "shared");
    let other = collection_choices(&f, "other");
    f.apply(vec![upsert(named_item(
        "other",
        "Renamed other",
        Some("other.model"),
    ))]);
    let renamed = collection_choices(&f, "other");
    assert_eq!(renamed.collection.name, "Renamed other");
    assert!(!Arc::ptr_eq(&other.models, &renamed.models));
    same_choices(&first, &f.choices());
    same_choices(&shared, &collection_choices(&f, "shared"));
    f.apply(vec![upsert(option(
        "other.model",
        MODEL,
        "Updated other model",
    ))]);
    let updated_other = collection_choices(&f, "other");
    assert_eq!(updated_other.models[0].name, "Updated other model");
    assert!(!Arc::ptr_eq(&renamed.models, &updated_other.models));
    same_choices(&first, &f.choices());
    same_choices(&shared, &collection_choices(&f, "shared"));
    f.apply(vec![upsert(model("Shared update"))]);
    f.apply(vec![]); // Revalidation must survive coalesced metadata notifications.
    assert_eq!(f.choices().models[0].name, "Shared update");
    assert!(!Arc::ptr_eq(&first.models, &f.choices().models));
    let updated_shared = collection_choices(&f, "shared");
    assert_eq!(updated_shared.models[0].name, "Shared update");
    assert!(!Arc::ptr_eq(&shared.models, &updated_shared.models));
    same_choices(&updated_other, &collection_choices(&f, "other"));
    let first = f.choices();
    f.apply(vec![remove(w::EntityKind::ItemType, "other")]);
    assert!(
        f.cache
            .collection(f.writer.reader(), "other".into())
            .is_err()
    );
    same_choices(&first, &f.choices());
    same_choices(&updated_shared, &collection_choices(&f, "shared"));
    f.apply(vec![upsert(named_item(
        "other",
        "Restored",
        Some("other.model"),
    ))]);
    assert_eq!(collection_choices(&f, "other").collection.name, "Restored");
    same_choices(&first, &f.choices());
}
#[test]
fn missing_allowed_options_are_dependencies_and_empty_restrictions_stay_empty() {
    let mut f = Fixture::new();
    let E::ItemType(mut empty) = named_item("empty", "Empty", Some("unused")) else {
        unreachable!()
    };
    for rule in empty.attributes.values_mut() {
        rule.allowed_values.as_mut().unwrap().ids.clear();
    }
    f.apply(vec![
        upsert(named_item("pending", "Pending", Some("later"))),
        upsert(E::ItemType(empty)),
    ]);
    let first = f.choices();
    let empty = collection_choices(&f, "empty");
    let missing = collection_choices(&f, "pending");
    assert!(missing.models.is_empty());
    assert!(empty.models.is_empty());
    f.apply(vec![upsert(option("later", MODEL, "Arrived"))]);
    let added = collection_choices(&f, "pending");
    assert_eq!(added.models[0].name, "Arrived");
    assert!(!Arc::ptr_eq(&missing.models, &added.models));
    same_choices(&first, &f.choices());
    same_choices(&empty, &collection_choices(&f, "empty"));
    f.apply(vec![remove(w::EntityKind::AttributeOption, "later")]);
    assert!(collection_choices(&f, "pending").models.is_empty());
    same_choices(&first, &f.choices());
    same_choices(&empty, &collection_choices(&f, "empty"));
    // A referenced option of the wrong kind can later become a valid model.
    f.apply(vec![upsert(option("later", PATTERN, "Wrong kind"))]);
    assert!(collection_choices(&f, "pending").models.is_empty());
    f.apply(vec![upsert(option("later", MODEL, "Restored"))]);
    assert_eq!(collection_choices(&f, "pending").models[0].name, "Restored");
    same_choices(&first, &f.choices());
}
#[test]
fn unrestricted_traits_track_only_their_attribute_group_including_membership_moves() {
    let mut f = Fixture::new();
    f.apply(vec![upsert(named_item("open", "Open", None))]);
    let first = f.choices();
    let open = collection_choices(&f, "open");
    f.apply(vec![upsert(option(
        "unrelated",
        BACKDROP,
        "Unrelated backdrop",
    ))]);
    same_choices(&first, &f.choices());
    same_choices(&open, &collection_choices(&f, "open"));
    f.apply(vec![upsert(option("new", MODEL, "New model"))]);
    let added = collection_choices(&f, "open");
    assert_eq!(added.models.len(), 2);
    assert!(!Arc::ptr_eq(&open.models, &added.models));
    same_choices(&first, &f.choices());
    f.apply(vec![upsert(option("new", BACKDROP, "Moved to backdrop"))]);
    let moved = collection_choices(&f, "open");
    assert_eq!(moved.models.len(), 1);
    assert!(moved.backdrops.is_empty());
    same_choices(&first, &f.choices());
    // A move must invalidate the destination group too.
    f.apply(vec![upsert(option(
        "unrelated",
        MODEL,
        "Moved into models",
    ))]);
    let moved_in = collection_choices(&f, "open");
    assert_eq!(moved_in.models.len(), 2);
    assert!(
        moved_in
            .models
            .iter()
            .any(|m| m.name == "Moved into models")
    );
    same_choices(&first, &f.choices());
    f.apply(vec![remove(w::EntityKind::AttributeOption, "unrelated")]);
    assert_eq!(collection_choices(&f, "open").models.len(), 1);
    same_choices(&first, &f.choices());
}
#[test]
fn first_and_last_unrestricted_option_invalidate_only_the_dependent_collection() {
    let mut f = Fixture::new();
    let E::ItemType(mut patterns) = named_item("open.patterns", "Patterns", Some("model")) else {
        unreachable!()
    };
    patterns.attributes.get_mut(PATTERN).unwrap().allowed_values = None;
    f.apply(vec![upsert(E::ItemType(patterns))]);
    let first = f.choices();
    let empty = collection_choices(&f, "open.patterns");
    assert!(empty.patterns.is_empty());
    f.apply(vec![upsert(option(
        "new.pattern",
        PATTERN,
        "First pattern",
    ))]);
    let added = collection_choices(&f, "open.patterns");
    assert_eq!(added.patterns.len(), 1);
    assert!(!Arc::ptr_eq(&empty.patterns, &added.patterns));
    same_choices(&first, &f.choices());
    f.apply(vec![remove(w::EntityKind::AttributeOption, "new.pattern")]);
    let removed = collection_choices(&f, "open.patterns");
    assert!(removed.patterns.is_empty());
    assert!(!Arc::ptr_eq(&added.patterns, &removed.patterns));
    same_choices(&first, &f.choices());
    f.apply(vec![upsert(option(
        "new.pattern",
        PATTERN,
        "Restored pattern",
    ))]);
    assert_eq!(
        collection_choices(&f, "open.patterns").patterns[0].name,
        "Restored pattern"
    );
    same_choices(&first, &f.choices());
}

#[test]
fn oversized_missing_dependencies_are_not_retained_and_preserve_other_cached_collections() {
    let mut f = Fixture::new();
    let E::ItemType(mut large) = named_item("large", "Large", Some("unused")) else {
        unreachable!()
    };
    large
        .attributes
        .get_mut(MODEL)
        .unwrap()
        .allowed_values
        .as_mut()
        .unwrap()
        .ids = (0..10_001).map(|i| format!("missing.{i}")).collect();
    f.apply(vec![upsert(E::ItemType(large))]);
    let first = f.choices();
    let large = collection_choices(&f, "large");
    assert!(large.models.is_empty());
    assert!(
        f.cache
            .lock()
            .choices
            .iter()
            .all(|entry| entry.id.as_str() != "large")
    );
    same_choices(&first, &f.choices());
}

#[test]
fn catalog_content_revisions_survive_cache_eviction_and_identical_replica_replacement() {
    let mut f = Fixture::new();
    let directory = f
        .cache
        .bootstrap_snapshot(f.writer.reader())
        .unwrap()
        .revision;
    let choices = f
        .cache
        .collection_snapshot(f.writer.reader(), "gift".into())
        .unwrap()
        .revision;
    f.cache.lock().choices.clear();
    assert_eq!(
        choices,
        f.cache
            .collection_snapshot(f.writer.reader(), "gift".into())
            .unwrap()
            .revision
    );
    let mut replacement = f.writer.begin_snapshot().unwrap();
    replacement.append(&seed()).unwrap();
    f.writer
        .install_snapshot(replacement.finish("same-content").unwrap())
        .unwrap();
    assert_eq!(
        directory,
        f.cache
            .bootstrap_snapshot(f.writer.reader())
            .unwrap()
            .revision
    );
    assert_eq!(
        choices,
        f.cache
            .collection_snapshot(f.writer.reader(), "gift".into())
            .unwrap()
            .revision
    );
}
