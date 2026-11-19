use super::*;
use crate::{
    MarketDataService,
    tests::{fixture, order},
};
use market_gifts::{GiftFilters, GiftOrderQuery, OrderRange, OrderWatchQuery};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

fn query(collection: &str) -> OrderWatchQuery {
    OrderWatchQuery {
        query: GiftOrderQuery::new(GiftFilters::new(
            market_gifts::CollectionId::new(collection).unwrap(),
            "GRAM".into(),
        )),
        range: Some(OrderRange {
            start: None,
            end: None,
        }),
    }
}
async fn terminal(updates: &mut watch::Receiver<DataUpdate>) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if matches!(*updates.borrow_and_update(), DataUpdate::Error(_)) {
                return;
            }
            updates.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
}
async fn price(updates: &mut watch::Receiver<DataUpdate>, expected: &str) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let DataUpdate::Orders { result, .. } = updates.borrow_and_update().clone()
                && result
                    .data
                    .first()
                    .is_some_and(|row| row.order.price.as_str() == expected)
            {
                return;
            }
            updates.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
}
struct Harness {
    service: MarketDataService,
    reads: market_replica::ReadRuntime,
}
impl Harness {
    async fn new(reader: market_replica::ReplicaReader, queries: usize) -> Self {
        let reads =
            market_replica::ReadRuntime::new(reader, queries, CancellationToken::new()).unwrap();
        let service = MarketDataService::start(reads.handle(), 1).await.unwrap();
        Self { service, reads }
    }
    async fn shutdown(self) {
        let stop = CancellationToken::new();
        stop.cancel();
        self.service.task.run(stop).await.unwrap();
        self.reads.shutdown().await;
    }
}

#[tokio::test]
async fn shared_leases_and_clones_cancel_only_after_the_final_lease_is_dropped() {
    let (_directory, writer) = fixture();
    let h = Harness::new(writer.reader(), 2).await;
    let (a, mut first) = h.service.handle.watch_orders(query("gift")).await.unwrap();
    let clone = a.clone();
    let (b, mut second) = h.service.handle.watch_orders(query("gift")).await.unwrap();
    assert!(first.same_channel(&second));
    let stopped = a._feed.stop.clone();
    price(&mut first, "3").await;
    drop(a);
    drop(b);
    assert!(
        !stopped.is_cancelled(),
        "a cloned lease still owns the shared producer"
    );
    drop(clone);
    assert!(
        stopped.is_cancelled(),
        "cleanup must not wait for an actor mailbox message"
    );
    terminal(&mut first).await;
    terminal(&mut second).await;
    h.shutdown().await;
}

#[tokio::test]
async fn terminal_updates_release_capacity_before_immediate_retries_even_with_old_leases() {
    let (_directory, writer) = fixture();
    let h = Harness::new(writer.reader(), 2).await;
    let mut old = Vec::new();
    for _ in 0..16 {
        let (lease, mut updates) = h
            .service
            .handle
            .watch_orders(query("missing"))
            .await
            .unwrap();
        for (_, previous) in &old {
            assert!(
                !updates.same_channel(previous),
                "a terminal feed cannot accept new subscribers"
            );
        }
        terminal(&mut updates).await;
        old.push((lease, updates));
    }
    let (active, mut updates) = h.service.handle.watch_orders(query("gift")).await.unwrap();
    drop(old);
    price(&mut updates, "3").await;
    assert!(
        !active._feed.stop.is_cancelled(),
        "old leases cannot cancel a replacement producer"
    );
    drop(active);
    terminal(&mut updates).await;
    h.shutdown().await;
}

#[tokio::test]
async fn losing_all_receivers_stops_a_feed_and_old_leases_cannot_cancel_its_replacement() {
    let (_directory, mut writer) = fixture();
    let h = Harness::new(writer.reader(), 2).await;
    let (old, mut updates) = h.service.handle.watch_orders(query("gift")).await.unwrap();
    price(&mut updates, "3").await;
    drop(updates);
    tokio::time::timeout(Duration::from_secs(2), old._feed.stop.cancelled())
        .await
        .unwrap();
    // Observe completion even though the caller retained only its lease.
    terminal(&mut old._feed.sender.subscribe()).await;
    let (new, mut updates) = h.service.handle.watch_orders(query("gift")).await.unwrap();
    assert!(!Arc::ptr_eq(&old._feed, &new._feed));
    drop(old);
    writer
        .apply(market_hub_protocol::wire::Batch {
            cursor: "replacement".into(),
            changes: vec![order("2")],
        })
        .unwrap();
    price(&mut updates, "2").await;
    assert!(!new._feed.stop.is_cancelled());
    drop(new);
    terminal(&mut updates).await;
    h.shutdown().await;
}

#[tokio::test]
async fn dropping_the_final_lease_cancels_a_producer_waiting_for_read_capacity() {
    let (_directory, writer) = fixture();
    let h = Harness::new(writer.reader(), 1).await;
    let busy = h.reads.handle().budget().try_query().unwrap();
    let (lease, mut updates) = h.service.handle.watch_orders(query("gift")).await.unwrap();
    let stopped = lease._feed.stop.clone();
    drop(lease);
    assert!(stopped.is_cancelled());
    terminal(&mut updates).await;
    // The producer released its slot without needing the occupied read permit.
    let (replacement, mut updates) = h.service.handle.watch_orders(query("gift")).await.unwrap();
    drop(replacement);
    terminal(&mut updates).await;
    drop(busy);
    h.shutdown().await;
}

#[tokio::test]
async fn equivalent_catalog_watches_share_a_feed_and_keep_independent_leases() {
    let (_directory, mut writer) = fixture();
    let h = Harness::new(writer.reader(), 2).await;
    let (a, mut first) = h
        .service
        .handle
        .subscribe_catalog(vec!["gift".into(), "gift".into()])
        .await
        .unwrap();
    let (b, second) = h
        .service
        .handle
        .subscribe_catalog(vec!["gift".into()])
        .await
        .unwrap();
    assert!(first.same_channel(&second));
    async fn catalog(updates: &mut watch::Receiver<DataUpdate>) -> market_gifts::CatalogVersions {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let DataUpdate::Catalog(value) = updates.borrow_and_update().clone() {
                    return value;
                }
                updates.changed().await.unwrap();
            }
        })
        .await
        .unwrap()
    }
    let before = catalog(&mut first).await;
    drop(a);
    writer
        .apply(market_hub_protocol::wire::Batch {
            cursor: "catalog".into(),
            changes: vec![crate::tests::entity(
                market_hub_protocol::wire::entity::Entity::Currency(
                    market_hub_protocol::wire::Currency {
                        id: "GRAM".into(),
                        name: "Updated currency".into(),
                        decimals: 9,
                    },
                ),
            )],
        })
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            first.changed().await.unwrap();
            if let DataUpdate::Catalog(value) = first.borrow_and_update().clone()
                && value != before
            {
                break;
            }
        }
    })
    .await
    .unwrap();
    drop(b);
    terminal(&mut first).await;
    h.shutdown().await;
}
