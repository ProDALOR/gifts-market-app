//! One internal owner for observed data, metadata and expiry.
use super::*;
use crate::{
    floors::{MAX_AGE, Statistics},
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
    pub(super) deadline: Instant,
}
impl ReadSet {
    pub fn new() -> Self {
        Self {
            metadata: None,
            statistics: vec![],
            observations: vec![],
            deadline: Instant::now() + MAX_AGE,
        }
    }
    pub fn statistics(&mut self, stats: Arc<Statistics>) -> Result<(), ValuationError> {
        self.deadline = self.deadline.min(stats.deadline());
        if let Some(previous) = self
            .statistics
            .iter_mut()
            .find(|entry| entry.scope == stats.scope)
        {
            *previous = stats;
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
    pub fn current(&self, snapshot: &ValuationSnapshot) -> Result<bool, ValuationError> {
        if Instant::now() >= self.deadline {
            return Ok(false);
        }
        if let Some(metadata) = &self.metadata
            && !snapshot.dependencies_data_current(metadata)?
        {
            return Ok(false);
        }
        for stats in &self.statistics {
            if !stats.current(snapshot)? {
                return Ok(false);
            }
        }
        for read in &self.observations {
            if !read.current(snapshot)? {
                return Ok(false);
            }
        }
        // Metadata validation can cross a listing's expiry while the view is pinned.
        Ok(Instant::now() < self.deadline)
    }
}
