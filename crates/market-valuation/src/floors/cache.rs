//! One owner per collection/currency: summary, optional traits, and one refresh deadline.
use super::{Preparation, REFRESH_INTERVAL, Statistics};
use crate::{
    ValuationError,
    source::{BookScope, ValuationSnapshot},
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
};
use tokio::time::Instant;

const TRAIT_CACHE_LIMIT: usize = 8;
const COLLECTION_CACHE_LIMIT: usize = 512;

pub(crate) struct StatisticsRead {
    pub statistics: Arc<Statistics>,
    /// Capture the deadline when reading the cache; later hits cannot move a report's timer.
    pub refresh_after: Instant,
}
#[derive(Clone)]
struct CachedTraits {
    statistics: Arc<Statistics>,
    used: u64,
}
#[derive(Clone)]
struct CachedBook {
    generation: u64,
    summary: Arc<Statistics>,
    traits: Option<CachedTraits>,
    refresh_after: Instant,
    used: u64,
}
impl CachedBook {
    fn read(&self, preparation: Preparation) -> Option<StatisticsRead> {
        let statistics = match preparation {
            Preparation::Collection => &self.summary,
            Preparation::Traits => &self.traits.as_ref()?.statistics,
        };
        Some(StatisticsRead {
            statistics: statistics.clone(),
            refresh_after: self.refresh_after,
        })
    }
    fn prepared(&self) -> &Statistics {
        self.traits
            .as_ref()
            .map_or(&self.summary, |traits| &traits.statistics)
    }
}
#[derive(Default)]
struct Index {
    books: HashMap<BookScope, CachedBook>,
    clock: u64,
    gates: HashMap<BookScope, Weak<Mutex<()>>>,
}
impl Index {
    fn active(&self, scope: &BookScope) -> bool {
        self.gates
            .get(scope)
            .is_some_and(|gate| gate.strong_count() > 0)
    }
    fn gate(&mut self, scope: &BookScope) -> Arc<Mutex<()>> {
        if self.gates.len() > COLLECTION_CACHE_LIMIT {
            self.gates.retain(|_, gate| gate.strong_count() > 0);
        }
        if let Some(gate) = self.gates.get(scope).and_then(Weak::upgrade) {
            return gate;
        }
        let gate = Arc::new(Mutex::new(()));
        self.gates.insert(scope.clone(), Arc::downgrade(&gate));
        gate
    }

    fn touch(&mut self) -> u64 {
        self.clock += 1;
        self.clock
    }
    fn insert(&mut self, scope: BookScope, entry: CachedBook) {
        let has_traits = entry.traits.is_some();
        self.books.insert(scope, entry);
        while has_traits
            && self
                .books
                .values()
                .filter(|book| book.traits.is_some())
                .count()
                > TRAIT_CACHE_LIMIT
        {
            let oldest = self
                .books
                .iter()
                .filter(|(scope, _)| !self.active(scope))
                .filter_map(|(scope, book)| book.traits.as_ref().map(|traits| (scope, traits.used)))
                .min_by_key(|(_, used)| *used)
                .map(|(scope, _)| scope.clone())
                .expect("cached traits");
            self.books.get_mut(&oldest).expect("cached book").traits = None;
        }
        // Trait-bearing entries are protected from summary sweeps. Dropping a trait
        // projection leaves its small summary available in this same entry.
        while self.books.len() > COLLECTION_CACHE_LIMIT {
            let oldest = self
                .books
                .iter()
                .filter(|(scope, book)| book.traits.is_none() && !self.active(scope))
                .min_by_key(|(_, book)| book.used)
                .map(|(scope, _)| scope.clone())
                .expect("evictable summary");
            self.books.remove(&oldest);
        }
    }
}
#[derive(Default)]
pub(crate) struct BookCache(Mutex<Index>);
impl BookCache {
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Index>, ValuationError> {
        self.0
            .lock()
            .map_err(|_| ValuationError::unavailable("Valuation cache is unavailable"))
    }
    pub fn get(
        &self,
        snapshot: &ValuationSnapshot,
        scope: &BookScope,
    ) -> Result<StatisticsRead, ValuationError> {
        self.prepare(snapshot, scope, Preparation::Traits)
    }
    pub fn collection(
        &self,
        snapshot: &ValuationSnapshot,
        scope: &BookScope,
    ) -> Result<StatisticsRead, ValuationError> {
        self.prepare(snapshot, scope, Preparation::Collection)
    }
    fn prepare(
        &self,
        snapshot: &ValuationSnapshot,
        scope: &BookScope,
        requested: Preparation,
    ) -> Result<StatisticsRead, ValuationError> {
        let gate = self.lock()?.gate(scope);
        let _preparation = gate
            .lock()
            .map_err(|_| ValuationError::unavailable("Valuation preparation failed"))?;
        snapshot.check_cancelled()?;
        let generation = snapshot.generation()?;
        let previous = {
            let mut index = self.lock()?;
            let used = index.touch();
            index.books.get_mut(scope).map(|entry| {
                entry.used = used;
                if requested == Preparation::Traits
                    && let Some(traits) = &mut entry.traits
                {
                    traits.used = used;
                }
                entry.clone()
            })
        };
        // Admission pins before this per-book gate. An older queued view may still
        // finish, but must never replace or revalidate a newer cached generation.
        let publish = previous
            .as_ref()
            .is_none_or(|entry| entry.generation <= generation);
        let previous = previous.filter(|_| publish);
        if let Some(entry) = &previous
            && let Some(mut read) = entry.read(requested)
        {
            let due = Instant::now() >= entry.refresh_after;
            if !due || entry.prepared().data_current(snapshot)? {
                if due {
                    snapshot.check_cancelled()?;
                    read.refresh_after = Instant::now() + REFRESH_INTERVAL;
                    let mut index = self.lock()?;
                    let entry = index.books.get_mut(scope).expect("validated book");
                    entry.refresh_after = read.refresh_after;
                    entry.generation = generation;
                }
                return Ok(read);
            }
        }
        let preparation = if requested == Preparation::Traits
            || previous
                .as_ref()
                .is_some_and(|entry| entry.traits.is_some())
        {
            Preparation::Traits
        } else {
            Preparation::Collection
        };
        // Per-book coalescing allows independent collections to prepare in parallel.
        // Scans stay outside the global index lock; active books cannot be evicted.
        let statistics = Arc::new(Statistics::read(snapshot, scope, preparation)?);
        let summary = if preparation == Preparation::Traits {
            match &previous {
                Some(entry) if entry.summary.data_current(snapshot)? => entry.summary.clone(),
                _ => Arc::new(statistics.collection_only(snapshot)?),
            }
        } else {
            statistics.clone()
        };
        snapshot.check_cancelled()?;
        let mut index = self.lock()?;
        let used = index.touch();
        let traits = (preparation == Preparation::Traits).then(|| CachedTraits {
            statistics,
            used: if requested == Preparation::Traits {
                used
            } else {
                previous
                    .as_ref()
                    .and_then(|entry| entry.traits.as_ref())
                    .expect("prepared traits")
                    .used
            },
        });
        let entry = CachedBook {
            generation,
            summary,
            traits,
            refresh_after: Instant::now() + REFRESH_INTERVAL,
            used,
        };
        let read = entry.read(requested).expect("prepared projection");
        if publish {
            index.insert(scope.clone(), entry);
        }
        Ok(read)
    }
}

#[cfg(test)]
#[path = "cache_tests.rs"]
mod tests;
