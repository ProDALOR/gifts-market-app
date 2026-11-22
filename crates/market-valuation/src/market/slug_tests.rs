use super::*;
use crate::{gift_floor::tests::book, integration_tests::entity, test_support::MarketDataService};
use market_gifts::*;
use market_hub_protocol::wire as w;
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

struct Resolver(Mutex<Result<TelegramGiftDetails, GiftResolutionError>>);
impl GiftResolver for Resolver {
    fn resolve<'a>(
        &'a self,
        slug: &'a GiftSlug,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<TelegramGiftDetails, GiftResolutionError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            let mut value = self.0.lock().unwrap().clone()?;
            value.slug = slug.clone();
            value.number = slug.number();
            Ok(value)
        })
    }
}
fn facts() -> TelegramGiftDetails {
    TelegramGiftDetails {
        id: "telegram-identity".into(),
        slug: GiftSlug::new("Gift-123").unwrap(),
        number: GiftNumber::new(123).unwrap(),
        collection: TelegramGiftId::new(99).unwrap(),
        model: TelegramDocumentId::new(100).unwrap(),
        backdrop: Some(200),
        pattern: None,
    }
}
fn query(number: u32) -> ValuationQuery {
    ValuationQuery {
        target: ValuationTarget::Gift {
            gift: GiftLookup::Slug(GiftSlug::new(format!("Gift-{number}")).unwrap()),
        },
        currency_id: "GRAM".into(),
        strategies: vec![StrategyId::new("gift_floor").unwrap()],
        period: None,
    }
}
fn aliases(writer: &mut market_replica::ReplicaWriter, with_slug: bool) {
    let alias = |id: i64| w::ExternalId {
        namespace: "tg".into(),
        id: id.to_string(),
    };
    let mut item = w::ItemType {
        id: "gift".into(),
        name: "Gift".into(),
        quantity_step: "1".into(),
        aliases: vec![alias(99)],
        attributes: [
            (
                "tg.model".into(),
                w::AttributeRule {
                    allowed_values: Some(w::AllowedAttributeValues {
                        ids: vec!["m".into(), "otherm".into()],
                    }),
                    required: false,
                },
            ),
            (
                "tg.backdrop".into(),
                w::AttributeRule {
                    allowed_values: Some(w::AllowedAttributeValues {
                        ids: vec!["b".into(), "otherb".into()],
                    }),
                    required: false,
                },
            ),
            ("tg.pattern".into(), w::AttributeRule::default()),
            ("tg.number".into(), w::AttributeRule::default()),
        ]
        .into(),
    };
    if with_slug {
        item.aliases.push(w::ExternalId {
            namespace: "tg.slug".into(),
            id: "Gift".into(),
        });
    }
    let mut changes = vec![entity(w::entity::Entity::ItemType(item))];
    for (id, kind, tg) in [
        ("m", "tg.model", 100),
        ("otherm", "tg.model", 101),
        ("b", "tg.backdrop", 200),
    ] {
        changes.push(entity(w::entity::Entity::AttributeOption(
            w::AttributeOption {
                id: id.into(),
                attribute_id: kind.into(),
                name: id.into(),
                aliases: vec![alias(tg)],
                ..Default::default()
            },
        )));
    }
    writer
        .apply(w::Batch {
            cursor: "aliases".into(),
            changes,
        })
        .unwrap();
}
async fn close(data: MarketDataService, task: crate::ValuationTask) {
    task.shutdown().await.unwrap();
    let stop = CancellationToken::new();
    stop.cancel();
    data.task.run(stop).await.unwrap();
}
#[tokio::test]
async fn user_session_facts_resolve_unlisted_gifts_without_canonical_slugs_and_preserve_market_identity()
 {
    let (_dir, mut writer) = book();
    aliases(&mut writer, false);
    let data = MarketDataService::start(writer.reader(), 2, 1)
        .await
        .unwrap();
    let (handle, task) = ValuationHandle::start(&data.arbiter(), data.reader.executor())
        .await
        .unwrap();
    assert_eq!(
        handle.query(query(123)).await.unwrap_err().code(),
        ValuationErrorCode::NotFound
    );
    let resolver = Arc::new(Resolver(Mutex::new(Ok(facts()))));
    handle.set_gift_resolver(resolver.clone()).unwrap();
    assert!(handle.set_gift_resolver(resolver.clone()).is_err());
    let listed = handle.query(query(123)).await.unwrap();
    assert_eq!(listed.target.gift().unwrap().id.as_str(), "gift-pair0");
    let unlisted = handle.query(query(9999)).await.unwrap();
    let gift = unlisted.target.gift().unwrap();
    assert_eq!(gift.id.as_str(), "telegram-identity");
    assert_eq!(gift.collection_id.as_str(), "gift");
    assert_eq!(gift.traits.model.as_ref().unwrap().as_str(), "m");
    let StrategyOutcome::Value(StrategyValuation::GiftFloor(value)) = &unlisted.results[0] else {
        panic!()
    };
    assert_eq!(value.final_mark.value().unwrap().as_str(), "18");
    // Changes in resolved facts must bypass completed reports, even within the minute.
    let mut updated = facts();
    updated.model = TelegramDocumentId::new(101).unwrap();
    *resolver.0.lock().unwrap() = Ok(updated);
    let updated = handle.query(query(9999)).await.unwrap();
    assert_eq!(
        updated
            .target
            .gift()
            .unwrap()
            .traits
            .model
            .as_ref()
            .unwrap()
            .as_str(),
        "otherm"
    );
    assert!(!Arc::ptr_eq(&unlisted, &updated));
    close(data, task).await;
}
#[tokio::test]
async fn temporary_user_session_failure_falls_back_to_observed_market_gifts() {
    let (_dir, mut writer) = book();
    aliases(&mut writer, true);
    let data = MarketDataService::start(writer.reader(), 2, 1)
        .await
        .unwrap();
    let (handle, task) = ValuationHandle::start(&data.arbiter(), data.reader.executor())
        .await
        .unwrap();
    let resolver = Arc::new(Resolver(Mutex::new(Err(GiftResolutionError::Unavailable))));
    handle.set_gift_resolver(resolver.clone()).unwrap();
    assert_eq!(
        handle
            .query(query(123))
            .await
            .unwrap()
            .target
            .gift()
            .unwrap()
            .id
            .as_str(),
        "gift-pair0"
    );
    assert_eq!(
        handle.query(query(9999)).await.unwrap_err().code(),
        ValuationErrorCode::Unavailable
    );
    *resolver.0.lock().unwrap() = Err(GiftResolutionError::NotFound);
    assert_eq!(
        handle.query(query(123)).await.unwrap_err().code(),
        ValuationErrorCode::NotFound
    );
    close(data, task).await;
}
struct Pending {
    started: CancellationToken,
    cancelled: CancellationToken,
}
impl GiftResolver for Pending {
    fn resolve<'a>(
        &'a self,
        _: &'a GiftSlug,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<TelegramGiftDetails, GiftResolutionError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            let _guard = self.cancelled.clone().drop_guard();
            self.started.cancel();
            std::future::pending().await
        })
    }
}
#[tokio::test]
async fn cancelling_slug_lookup_releases_work_without_reserving_a_replica_read_worker() {
    let (_dir, writer) = book();
    let data = MarketDataService::start(writer.reader(), 1, 1)
        .await
        .unwrap();
    let (handle, task) = ValuationHandle::start(&data.arbiter(), data.reader.executor())
        .await
        .unwrap();
    let pending = Arc::new(Pending {
        started: CancellationToken::new(),
        cancelled: CancellationToken::new(),
    });
    handle.set_gift_resolver(pending.clone()).unwrap();
    let clone = handle.clone();
    let request = tokio::spawn(async move { clone.query(query(9999)).await });
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        pending.started.cancelled(),
    )
    .await
    .unwrap();
    // The only read worker stays available while Telegram is pending.
    let mut selection = query(9999);
    selection.target = ValuationTarget::Selection {
        collection_id: "gift".into(),
        criteria: Default::default(),
    };
    selection.strategies = vec![StrategyId::new("ask_floor").unwrap()];
    tokio::time::timeout(std::time::Duration::from_secs(2), handle.query(selection))
        .await
        .unwrap()
        .unwrap();
    request.abort();
    let _ = request.await;
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        pending.cancelled.cancelled(),
    )
    .await
    .unwrap();
    close(data, task).await;
}
