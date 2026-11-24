use crate::{source::ValuationSource, *};
use std::sync::Arc;
use tokio::sync::watch;
pub(crate) type GiftResolverState = Arc<std::sync::OnceLock<Arc<dyn market_gifts::GiftResolver>>>;
pub(crate) struct Work {
    pub request: crate::market::PreparedQuery,
    pub previous: Option<Arc<crate::market::ValuationRead>>,
}
#[derive(Clone)]
pub struct ValuationHandle {
    pub(crate) source: ValuationSource,
    pub(crate) books: Arc<crate::floors::BookCache>,
    native: Handle<Work, Arc<crate::market::ValuationRead>, ValuationError>,
    status: watch::Receiver<market_replica::SyncStatus>,
    watch_slots: Arc<tokio::sync::Semaphore>,
    batch_slots: Arc<tokio::sync::Semaphore>,
    gift_resolver: GiftResolverState,
}
impl ValuationHandle {
    pub(crate) async fn closed(&self) {
        self.native.closed().await;
    }
    /// Configure once during application composition, before starting API/bot consumers.
    pub fn set_gift_resolver(
        &self,
        resolver: Arc<dyn market_gifts::GiftResolver>,
    ) -> Result<(), ValuationError> {
        self.gift_resolver
            .set(resolver)
            .map_err(|_| ValuationError::invalid("Gift resolver is already configured"))
    }
    pub async fn query(
        &self,
        request: ValuationQuery,
    ) -> Result<Arc<ValuationReport>, ValuationError> {
        Ok(self.read(request.prepare()?, None).await?.report.clone())
    }
    pub async fn batch(
        &self,
        query: ValuationBatchQuery,
    ) -> Result<Arc<ValuationBatchReport>, ValuationError> {
        query.validate()?;
        let _permit = self
            .batch_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| ValuationError::capacity("Valuation batch capacity reached"))?;
        let resolver = self.gift_resolver.get().cloned();
        let source = self.source.clone();
        let books = self.books.clone();
        let work = async move {
            // Normalize before coalescing. Repeated queries retain their output slots,
            // but share lookup, calculation and the immutable completed report.
            let mut unique: Vec<crate::market::PreparedQuery> = Vec::new();
            let mut positions = Vec::with_capacity(query.queries.len());
            for query in query.queries {
                positions.push(
                    query
                        .prepare()
                        .map(|input| {
                            if let Some(index) =
                                unique.iter().position(|old| old.same_query(&input))
                            {
                                index
                            } else {
                                let index = unique.len();
                                unique.push(input);
                                index
                            }
                        })
                        .map_err(StrategyFailure::from),
                );
            }
            if unique.is_empty() {
                return Ok(Arc::new(ValuationBatchReport {
                    results: positions
                        .into_iter()
                        .map(|item| ValuationBatchItem::Error {
                            error: item.expect_err("no valid batch queries"),
                        })
                        .collect(),
                }));
            }
            let mut prepared = Vec::with_capacity(unique.len());
            let mut pending = unique.into_iter();
            while let Some(first) = pending.next() {
                if let Some(second) = pending.next() {
                    let (first, second) = tokio::join!(
                        batch_resolve(first, resolver.clone()),
                        batch_resolve(second, resolver.clone())
                    );
                    prepared.extend([first, second]);
                } else {
                    prepared.push(batch_resolve(first, resolver.clone()).await);
                }
            }
            source
                .prepare(move |snapshot| -> Result<_, ValuationError> {
                    let mut inputs = Vec::with_capacity(prepared.len());
                    for item in prepared {
                        snapshot.check_cancelled()?;
                        inputs.push(item.and_then(|(query, failure)| {
                            crate::market::BatchInput::resolve(&snapshot, query, failure)
                                .map_err(Into::into)
                        }));
                    }
                    let evaluated_at = jiff::Timestamp::now();
                    let results = crate::market::evaluate_batch(
                        &snapshot,
                        &books,
                        inputs,
                        |input, results| {
                            Arc::new(ValuationReport {
                                target: input.target.as_ref().clone(),
                                currency_id: input.currency.clone(),
                                evaluated_at,
                                stale: false,
                                results,
                            })
                        },
                        |_| {},
                    )?;
                    let stale = snapshot.sync()?.state != market_gifts::SyncState::Live;
                    let results: Vec<_> = results
                        .into_iter()
                        .map(|item| match item {
                            Ok(mut data) => {
                                Arc::make_mut(&mut data).stale = stale;
                                ValuationBatchItem::Value { data }
                            }
                            Err(error) => ValuationBatchItem::Error { error },
                        })
                        .collect();
                    let results = positions
                        .into_iter()
                        .map(|position| match position {
                            Ok(index) => results[index].clone(),
                            Err(error) => ValuationBatchItem::Error { error },
                        })
                        .collect();
                    Ok(Arc::new(ValuationBatchReport { results }))
                })
                .await?
        };
        tokio::select! { biased;
            _ = self.native.closed() => Err(ValuationError::unavailable("Valuation service stopped")),
            value = tokio::time::timeout(std::time::Duration::from_secs(30), work) => value.map_err(|_| ValuationError::timed_out("Valuation batch timed out"))?,
        }
    }
    async fn read(
        &self,
        request: crate::market::PreparedQuery,
        previous: Option<Arc<crate::market::ValuationRead>>,
    ) -> Result<Arc<crate::market::ValuationRead>, ValuationError> {
        let mark = self
            .native
            .evaluate(Work { request, previous })
            .await
            .map_err(crate::client::actor_error)?;
        Ok(mark.data.clone())
    }

    pub async fn watch(&self, request: ValuationQuery) -> Result<ValuationWatch, ValuationError> {
        let request = request.prepare()?;
        let permit = self
            .watch_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| ValuationError::capacity("Valuation watch capacity reached"))?;
        let mut status = self.status.clone();
        status.borrow_and_update();
        let current = self.read(request.clone(), None).await?;
        Ok(ValuationWatch {
            handle: self.clone(),
            request,
            current,
            status,
            _permit: permit,
        })
    }
    pub async fn start(
        arbiter: &actix::ArbiterHandle,
        reads: market_replica::ReadExecutor,
    ) -> Result<(Self, ValuationTask), ServiceError> {
        let status = reads.reader().status_updates();
        let gift_resolver: GiftResolverState = Default::default();
        let actor_resolver = gift_resolver.clone();
        let source = ValuationSource::new(reads);
        let books = Arc::new(crate::floors::BookCache::default());
        let actor_source = source.clone();
        let actor_books = books.clone();
        let service = Service::start_on(
            arbiter,
            "valuation",
            move |life| {
                crate::market::MarketValuationActor::new(
                    actor_source,
                    actor_books,
                    life,
                    actor_resolver,
                )
            },
            Config {
                max_requests: 32,
                ..Default::default()
            },
        )
        .await?;
        Ok((
            Self {
                source,
                books,
                native: service.handle,
                status,
                watch_slots: Arc::new(tokio::sync::Semaphore::new(32)),
                batch_slots: Arc::new(tokio::sync::Semaphore::new(32)),
                gift_resolver,
            },
            service.task,
        ))
    }
}
async fn batch_resolve(
    mut query: crate::market::PreparedQuery,
    resolver: Option<Arc<dyn market_gifts::GiftResolver>>,
) -> Result<(crate::market::PreparedQuery, Option<ValuationError>), StrategyFailure> {
    let failure = crate::market::resolution::external(&mut query, resolver)
        .await
        .map_err(StrategyFailure::from)?;
    Ok((query, failure))
}
/// Pull-based watch: dropping it or its pending next() releases all work through normal cancellation.
pub struct ValuationWatch {
    handle: ValuationHandle,
    request: crate::market::PreparedQuery,
    current: Arc<crate::market::ValuationRead>,
    status: watch::Receiver<market_replica::SyncStatus>,
    _permit: tokio::sync::OwnedSemaphorePermit,
}
impl ValuationWatch {
    pub fn current(&self) -> Arc<ValuationReport> {
        self.current.report.clone()
    }
    pub async fn changed(&mut self) -> Result<Arc<ValuationReport>, ValuationError> {
        loop {
            let stale = self.status.borrow_and_update().state != market_replica::SyncState::Live;
            if stale == self.current.report.stale
                && tokio::time::Instant::now() < self.current.deadline()
            {
                tokio::select! {
                    _ = self.handle.native.closed() => return Err(ValuationError::unavailable("Valuation service stopped")),
                    _ = tokio::time::sleep_until(self.current.deadline()) => {},
                    changed = self.status.changed() => changed.map_err(|_|ValuationError::unavailable("Valuation source closed"))?,
                }
                continue;
            }
            let next = self
                .handle
                .read(self.request.clone(), Some(self.current.clone()))
                .await?;
            let changed = !Arc::ptr_eq(&next.report, &self.current.report);
            // Always retain the new deadline, including when the report is unchanged.
            self.current = next;
            if changed {
                return Ok(self.current());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        gift_floor::tests::{ask, book},
        test_support::MarketDataService,
    };
    use market_hub_protocol::wire as w;
    use std::time::Duration;
    #[tokio::test]
    async fn watch_initialization_keeps_cached_prices_and_observes_availability_during_setup() {
        let (_dir, mut writer) = book();
        let data = MarketDataService::start(writer.reader(), 2, 1)
            .await
            .unwrap();
        let (handle, task) = ValuationHandle::start(&data.arbiter(), data.reader.executor())
            .await
            .unwrap();
        let query = ValuationQuery {
            target: ValuationTarget::Selection {
                collection_id: "gift".into(),
                criteria: Default::default(),
            },
            currency_id: "GRAM".into(),
            strategies: vec![StrategyId::new("ask_floor").unwrap()],
            period: None,
        };
        writer
            .apply(w::Batch {
                cursor: "before-watch".into(),
                changes: vec![ask("before", "m", "b", "2", 999, None)],
            })
            .unwrap();
        let watch = handle.watch(query.clone()).await.unwrap();
        assert!(
            !watch.status.has_changed().unwrap(),
            "The baseline acknowledges previous status notifications"
        );
        drop(watch);
        let executor = data.reader.executor();
        let held = executor.budget().wait().await.unwrap();
        let held2 = executor.budget().wait().await.unwrap();
        let mut setup = Box::pin(handle.watch(query));
        assert!(
            tokio::time::timeout(Duration::from_millis(30), &mut setup)
                .await
                .is_err()
        );
        writer
            .apply(w::Batch {
                cursor: "during-watch".into(),
                changes: vec![ask("during", "m", "b", "1", 998, None)],
            })
            .unwrap();
        drop(held);
        drop(held2);
        let watch = setup.await.unwrap();
        assert!(
            watch.status.has_changed().unwrap(),
            "Status updates during initialization remain observable"
        );
        let StrategyOutcome::Value(StrategyValuation::AskFloor(value)) =
            &watch.current.report.results[0]
        else {
            panic!()
        };
        assert_eq!(value.price.value().unwrap().as_str(), "2");
        drop(watch);
        task.shutdown().await.unwrap();
        let stop = tokio_util::sync::CancellationToken::new();
        stop.cancel();
        data.task.run(stop).await.unwrap();
    }
    #[tokio::test]
    async fn busy_collection_watches_and_concurrent_queries_share_one_minute_refresh() {
        let (_dir, mut writer) = book();
        let data = MarketDataService::start(writer.reader(), 4, 1)
            .await
            .unwrap();
        let (handle, task) = ValuationHandle::start(&data.arbiter(), data.reader.executor())
            .await
            .unwrap();
        let query = ValuationQuery {
            target: ValuationTarget::Selection {
                collection_id: "gift".into(),
                criteria: Default::default(),
            },
            currency_id: "GRAM".into(),
            strategies: vec![StrategyId::new("ask_floor").unwrap()],
            period: None,
        };
        let mut watch = handle.watch(query.clone()).await.unwrap();
        let baseline = watch.current();
        let initial_deadline = watch.current.deadline();
        for i in 1..=20 {
            writer
                .apply(w::Batch {
                    cursor: format!("burst-{i}"),
                    changes: vec![ask("busy", "m", "b", "1", i, None)],
                })
                .unwrap();
            assert!(Arc::ptr_eq(
                &baseline,
                &handle.query(query.clone()).await.unwrap()
            ));
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(20), watch.changed())
                .await
                .is_err()
        );
        assert_eq!(
            watch.current.deadline(),
            initial_deadline,
            "Change bursts cannot postpone the deadline"
        );
        // Availability is independent of the cached price interval.
        writer.mark(market_replica::SyncState::Stale);
        let stale = tokio::time::timeout(Duration::from_secs(2), watch.changed())
            .await
            .unwrap()
            .unwrap();
        assert!(stale.stale);
        assert_eq!(stale.evaluated_at, baseline.evaluated_at);
        assert_eq!(watch.current.deadline(), initial_deadline);
        writer.mark(market_replica::SyncState::Live);
        let _ = watch.changed().await.unwrap();
        crate::test_support::refresh_interval().await;
        let (a, b, updated) = tokio::join!(
            handle.query(query.clone()),
            handle.query(query.clone()),
            watch.changed()
        );
        let (a, b, updated) = (a.unwrap(), b.unwrap(), updated.unwrap());
        assert!(Arc::ptr_eq(&a, &b));
        assert!(Arc::ptr_eq(&a, &updated));
        assert!(!Arc::ptr_eq(&a, &baseline));
        assert!(
            matches!(&a.results[0], StrategyOutcome::Value(StrategyValuation::AskFloor(value)) if value.price.value().unwrap().as_str() == "1")
        );
        // A no-change interval advances the timer but neither rebuilds nor emits a report.
        let statistics = watch.current.statistics()[0].clone();
        crate::test_support::refresh_interval().await;
        assert!(
            tokio::time::timeout(Duration::from_millis(30), watch.changed())
                .await
                .is_err()
        );
        assert!(Arc::ptr_eq(&a, &watch.current()));
        assert!(Arc::ptr_eq(&statistics, &watch.current.statistics()[0]));
        assert!(watch.current.deadline() > tokio::time::Instant::now());
        // Dropping a pending next() leaves the due refresh observable on its next call.
        writer
            .apply(w::Batch {
                cursor: "after-cancel".into(),
                changes: vec![ask("busy", "m", "b", "0.5", 999, None)],
            })
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(20), watch.changed())
                .await
                .is_err()
        );
        crate::test_support::refresh_interval().await;
        let budget = data.reader.executor().budget();
        let mut held = Vec::new();
        for _ in 0..4 {
            held.push(budget.wait().await.unwrap());
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(20), watch.changed())
                .await
                .is_err()
        );
        drop(held);
        let refreshed = tokio::time::timeout(Duration::from_secs(2), watch.changed())
            .await
            .unwrap()
            .unwrap();
        assert!(
            matches!(&refreshed.results[0], StrategyOutcome::Value(StrategyValuation::AskFloor(value)) if value.price.value().unwrap().as_str() == "0.5")
        );
        task.shutdown().await.unwrap();
        assert!(watch.changed().await.is_err());
        drop(watch);
        let stop = tokio_util::sync::CancellationToken::new();
        stop.cancel();
        data.task.run(stop).await.unwrap();
    }
}
