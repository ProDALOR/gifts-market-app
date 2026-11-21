//! One internal owner for observed dependencies and the minimum refresh deadline.
use super::*;
use crate::{
    floors::{REFRESH_INTERVAL, Statistics, StatisticsRead},
    source::{BookScope, OrderDependency, ReadDependencies, ValuationSnapshot},
};
use market_gifts::{CollectionId, CurrencyId, GiftLookup, GiftOrderQuery};
use std::sync::Arc;
use tokio::time::Instant;
#[derive(Clone)]
pub(super) struct ReadSet {
    metadata: Option<ReadDependencies>,
    pub(super) statistics: Vec<Arc<Statistics>>,
    observations: Vec<OrderDependency>,
    expires_at: Option<jiff::Timestamp>,
    pub(super) deadline: Instant,
}
impl ReadSet {
    pub fn new() -> Self {
        Self {
            metadata: None,
            statistics: vec![],
            observations: vec![],
            expires_at: None,
            deadline: Instant::now() + REFRESH_INTERVAL,
        }
    }
    pub fn statistics(&mut self, read: StatisticsRead) -> Result<(), ValuationError> {
        self.deadline = self.deadline.min(read.refresh_after);
        let stats = read.statistics;
        if let Some(previous) = self
            .statistics
            .iter_mut()
            .find(|entry| entry.scope == stats.scope)
        {
            if stats.traits_prepared() || !previous.traits_prepared() {
                *previous = stats;
            }
        } else {
            self.statistics.push(stats);
        }
        self.check_metadata_budget()
    }
    pub fn observe(
        &mut self,
        snapshot: &ValuationSnapshot,
        query: &GiftOrderQuery,
    ) -> Result<(), ValuationError> {
        let dependency = snapshot.order_dependency(query)?;
        if !self
            .observations
            .iter()
            .any(|previous| previous.same_query(&dependency))
        {
            if self.observations.len() >= 128 {
                return Err(ValuationError::capacity(
                    "valuation observation dependency budget exceeded",
                ));
            }
            self.observations.push(dependency);
        }
        Ok(())
    }
    pub fn finish(
        &mut self,
        snapshot: &ValuationSnapshot,
        collections: Vec<CollectionId>,
        currency: &CurrencyId,
        lookup: Option<&GiftLookup>,
    ) -> Result<(), ValuationError> {
        // Successful statistics already cover these books, their currency and catalog choices.
        let uncovered = collections
            .into_iter()
            .filter(|collection| {
                !self.statistics.iter().any(|stats| {
                    &stats.scope.collection == collection && &stats.scope.currency == currency
                })
            })
            .collect::<Vec<_>>();
        if !uncovered.is_empty() || matches!(lookup, Some(GiftLookup::Slug(_))) {
            self.metadata = Some(snapshot.target_dependencies(
                uncovered.into_iter().map(|collection| BookScope {
                    collection,
                    currency: currency.clone(),
                }),
                currency,
                lookup,
            )?);
        }
        self.check_metadata_budget()
    }
    fn check_metadata_budget(&self) -> Result<(), ValuationError> {
        // Keep the total metadata work limit, without rereading covered choices.
        let keys = self
            .metadata
            .as_ref()
            .map_or(1, ReadDependencies::key_count)
            + self
                .statistics
                .iter()
                .map(|stats| stats.metadata_keys().saturating_sub(1))
                .sum::<usize>();
        if keys > crate::source::METADATA_KEY_LIMIT {
            return Err(ValuationError::capacity("read dependency budget exceeded"));
        }
        Ok(())
    }
    pub fn renew(&mut self) {
        self.deadline = Instant::now() + REFRESH_INTERVAL;
    }
    pub fn expires_at(&mut self, at: jiff::Timestamp) {
        self.expires_at = Some(self.expires_at.map_or(at, |previous| previous.min(at)));
    }
    pub fn current(&self, snapshot: &ValuationSnapshot) -> Result<bool, ValuationError> {
        snapshot.check_cancelled()?;
        // Changes and expirations are intentionally sampled at most once a minute.
        if Instant::now() < self.deadline {
            return Ok(true);
        }
        if self
            .expires_at
            .is_some_and(|at| at <= jiff::Timestamp::now())
        {
            return Ok(false);
        }
        if let Some(metadata) = &self.metadata
            && !snapshot.dependencies_data_current(metadata)?
        {
            return Ok(false);
        }
        for stats in &self.statistics {
            if !stats.data_current(snapshot)? {
                return Ok(false);
            }
        }
        for read in &self.observations {
            if !read.current(snapshot)? {
                return Ok(false);
            }
        }
        snapshot.check_cancelled()?;
        Ok(true)
    }
}
