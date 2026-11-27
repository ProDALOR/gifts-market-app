//! Bounded, cached selection indexes over an immutable valued inventory.
use crate::{PortfolioError, PortfolioQuery, PortfolioState, UserId};
use market_gifts::{
    Amount, BackdropId, BackdropRef, CollectionId, CollectionRef, CurrencyId, GiftNumber, ModelId,
    ModelRef, PatternId, PatternRef,
};
use market_valuation::{InventoryValuation, InventoryValue};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, VecDeque},
    sync::{Arc, Mutex, Weak},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schema", schema(value_type = String, pattern = "^[0-9a-f]{32}$"))]
pub struct PortfolioVersion(String);
impl PortfolioVersion {
    pub(crate) fn new() -> Self {
        Self(uuid::Uuid::new_v4().simple().to_string())
    }
}
impl TryFrom<String> for PortfolioVersion {
    type Error = market_gifts::InvalidValue;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.len() != 32
            || !value
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            return Err(market_gifts::InvalidValue("invalid portfolio version"));
        }
        Ok(Self(value))
    }
}
impl From<PortfolioVersion> for String {
    fn from(value: PortfolioVersion) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[serde(bound(
    deserialize = "C: Deserialize<'de>, M: Deserialize<'de>, B: Deserialize<'de>, P: Deserialize<'de>"
))]
pub struct PortfolioFilters<C = CollectionRef, M = ModelRef, B = BackdropRef, P = PatternRef> {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schema(required = false))]
    pub collection_id: Option<C>,
    #[serde(default)]
    #[cfg_attr(feature = "schema", schema(max_items = 4096))]
    pub models: Vec<M>,
    #[serde(default)]
    #[cfg_attr(feature = "schema", schema(max_items = 4096))]
    pub backdrops: Vec<B>,
    #[serde(default)]
    #[cfg_attr(feature = "schema", schema(max_items = 4096))]
    pub patterns: Vec<P>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schema(required = false))]
    pub number: Option<GiftNumber>,
}
pub type ResolvedPortfolioFilters = PortfolioFilters<CollectionId, ModelId, BackdropId, PatternId>;
impl<C, M, B, P> Default for PortfolioFilters<C, M, B, P> {
    fn default() -> Self {
        Self {
            collection_id: None,
            models: vec![],
            backdrops: vec![],
            patterns: vec![],
            number: None,
        }
    }
}
impl<C, M: Ord, B: Ord, P: Ord> PortfolioFilters<C, M, B, P> {
    fn validate(&self) -> Result<(), PortfolioError> {
        if self.collection_id.is_none() && !self.models.is_empty() {
            return Err(PortfolioError::InvalidData(
                "model filters require a collection".into(),
            ));
        }
        if [self.models.len(), self.backdrops.len(), self.patterns.len()]
            .into_iter()
            .any(|n| n > 4096)
        {
            return Err(PortfolioError::InvalidData(
                "select at most 4096 resolved values per trait".into(),
            ));
        }
        Ok(())
    }
    fn normalize(&mut self) -> Result<(), PortfolioError> {
        self.validate()?;
        self.models.sort();
        self.models.dedup();
        self.backdrops.sort();
        self.backdrops.dedup();
        self.patterns.sort();
        self.patterns.dedup();
        Ok(())
    }
}
impl ResolvedPortfolioFilters {
    fn matches(&self, item: &InventoryValue) -> bool {
        let gift = item.resolved.as_ref();
        let traits = gift.map(|gift| &gift.traits);
        self.collection_id
            .as_ref()
            .is_none_or(|id| gift.is_some_and(|gift| &gift.collection_id == id))
            && self
                .number
                .is_none_or(|number| item.gift.gift.number == number)
            && (self.models.is_empty()
                || traits
                    .and_then(|t| t.model.as_ref())
                    .is_some_and(|id| self.models.binary_search(id).is_ok()))
            && (self.backdrops.is_empty()
                || traits
                    .and_then(|t| t.backdrop.as_ref())
                    .is_some_and(|id| self.backdrops.binary_search(id).is_ok()))
            && (self.patterns.is_empty()
                || traits
                    .and_then(|t| t.pattern.as_ref())
                    .is_some_and(|id| self.patterns.binary_search(id).is_ok()))
    }
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub enum PortfolioSort {
    PositionAsc,
    PositionDesc,
    NumberAsc,
    NumberDesc,
    ValueAsc,
    #[default]
    ValueDesc,
}
fn page_size() -> usize {
    100
}
/// A reusable portfolio selection, independent of page coverage and inventory versions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[serde(bound(
    deserialize = "C: Deserialize<'de>, M: Deserialize<'de>, B: Deserialize<'de>, P: Deserialize<'de>"
))]
pub struct PortfolioTarget<C = CollectionRef, M = ModelRef, B = BackdropRef, P = PatternRef> {
    pub user_id: UserId,
    pub currency_id: CurrencyId,
    #[serde(default)]
    pub filters: PortfolioFilters<C, M, B, P>,
    #[serde(default)]
    pub sort: PortfolioSort,
}
pub type ResolvedPortfolioTarget = PortfolioTarget<CollectionId, ModelId, BackdropId, PatternId>;
impl<C, M: Ord, B: Ord, P: Ord> PortfolioTarget<C, M, B, P> {
    pub fn validate(&self) -> Result<(), PortfolioError> {
        self.filters.validate()
    }
    pub fn into_query(self) -> PortfolioReadQuery<C, M, B, P> {
        PortfolioReadQuery {
            user_id: self.user_id,
            currency_id: self.currency_id,
            filters: self.filters,
            sort: self.sort,
            offset: 0,
            limit: 30,
            version: None,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[serde(bound(
    deserialize = "C: Deserialize<'de>, M: Deserialize<'de>, B: Deserialize<'de>, P: Deserialize<'de>"
))]
pub struct PortfolioReadQuery<C = CollectionRef, M = ModelRef, B = BackdropRef, P = PatternRef> {
    pub user_id: UserId,
    pub currency_id: CurrencyId,
    #[serde(default)]
    pub filters: PortfolioFilters<C, M, B, P>,
    #[serde(default)]
    pub sort: PortfolioSort,
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "page_size")]
    #[cfg_attr(feature = "schema", schema(minimum = 1, maximum = 100, default = 100))]
    pub limit: usize,
    /// Reuse the returned version for subsequent pages. Changes require a fresh read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<PortfolioVersion>,
}
pub type ResolvedPortfolioReadQuery =
    PortfolioReadQuery<CollectionId, ModelId, BackdropId, PatternId>;
impl<C, M: Ord, B: Ord, P: Ord> PortfolioReadQuery<C, M, B, P> {
    pub fn subject(&self) -> PortfolioQuery {
        PortfolioQuery {
            user_id: self.user_id,
            currency_id: self.currency_id.clone(),
        }
    }
    pub fn validate(&self) -> Result<(), PortfolioError> {
        if self.limit == 0 || self.limit > 100 || self.offset > 10_000 {
            return Err(PortfolioError::InvalidData(
                "portfolio limit must be 1..100 and offset at most 10000".into(),
            ));
        }
        if self.offset > 0 && self.version.is_none() {
            return Err(PortfolioError::InvalidData(
                "continuation requires a portfolio version".into(),
            ));
        }
        self.filters.validate()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct PortfolioSelection {
    pub count: usize,
    pub priced: usize,
    pub unpriced: usize,
    pub unverified: usize,
    pub collections: Vec<CollectionId>,
    pub total: Option<Amount>,
}
impl PortfolioSelection {
    pub fn full(report: &InventoryValuation) -> Self {
        Self {
            count: report.items.len(),
            priced: report.priced,
            unpriced: report.unpriced,
            unverified: report
                .items
                .iter()
                .filter(|item| {
                    item.gift.relation == market_gifts::inventory::GiftRelation::Unverified
                })
                .count(),
            collections: report
                .items
                .iter()
                .filter_map(|item| item.resolved.as_ref().map(|gift| &gift.collection_id))
                .collect::<BTreeSet<_>>()
                .into_iter()
                .cloned()
                .collect(),
            total: report.total.clone(),
        }
    }
    pub fn summarize<'a>(
        items: impl IntoIterator<Item = &'a InventoryValue>,
    ) -> Result<Self, PortfolioError> {
        let mut total = Decimal::ZERO;
        let (mut count, mut priced, mut unverified) = (0, 0, 0);
        let mut collections = BTreeSet::new();
        for item in items {
            count += 1;
            if let Some(gift) = &item.resolved {
                collections.insert(&gift.collection_id);
            }
            if let Some(value) = &item.estimate {
                total = total.checked_add(value.decimal()).ok_or_else(|| {
                    PortfolioError::InvalidData("portfolio total overflow".into())
                })?;
                priced += 1;
            }
            if item.gift.relation == market_gifts::inventory::GiftRelation::Unverified {
                unverified += 1;
            }
        }
        Ok(Self {
            count,
            priced,
            unpriced: count - priced,
            unverified,
            collections: collections.into_iter().cloned().collect(),
            total: (priced > 0 || count == 0)
                .then(|| Amount::try_from(total).expect("nonnegative checked total")),
        })
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct PortfolioPage {
    pub version: PortfolioVersion,
    pub offset: usize,
    #[cfg_attr(feature = "schema", schema(max_items = 100))]
    pub items: Vec<InventoryValue>,
    pub has_more: bool,
    pub selection: Arc<PortfolioSelection>,
}
#[derive(Clone, PartialEq, Eq)]
struct Key {
    user: UserId,
    currency: CurrencyId,
    version: PortfolioVersion,
    filters: ResolvedPortfolioFilters,
    sort: PortfolioSort,
}
struct Index {
    positions: Vec<usize>,
    selection: Arc<PortfolioSelection>,
}
#[derive(Default)]
struct ReadCache {
    entries: VecDeque<(Key, Arc<Index>)>,
    builds: Vec<(Key, Weak<Mutex<()>>)>,
}
impl ReadCache {
    fn get(&mut self, key: &Key) -> Option<Arc<Index>> {
        let position = self.entries.iter().position(|(old, _)| old == key)?;
        let entry = self.entries.remove(position).expect("located read index");
        let index = entry.1.clone();
        self.entries.push_back(entry);
        Some(index)
    }
}
/// At most 32 compact indexes; this cache retains no inventory/report snapshots.
#[derive(Default)]
pub struct PortfolioReadCache(Mutex<ReadCache>);
impl PortfolioReadCache {
    fn index(
        &self,
        report: &InventoryValuation,
        key: Key,
        check: &mut impl FnMut() -> Result<(), PortfolioError>,
    ) -> Result<Arc<Index>, PortfolioError> {
        let build = {
            let mut cache = self.0.lock().map_err(|_| PortfolioError::Stopped)?;
            if let Some(index) = cache.get(&key) {
                return Ok(index);
            }
            cache.builds.retain(|(_, build)| build.strong_count() > 0);
            if let Some(build) = cache
                .builds
                .iter()
                .find_map(|(old, build)| (old == &key).then(|| build.upgrade()).flatten())
            {
                build
            } else {
                let build = Arc::new(Mutex::new(()));
                cache.builds.push((key.clone(), Arc::downgrade(&build)));
                build
            }
        };
        // Coalesce identical selections; unrelated reads never wait on a scan or sort.
        let _building = build.lock().map_err(|_| PortfolioError::Stopped)?;
        check()?;
        if let Some(index) = self
            .0
            .lock()
            .map_err(|_| PortfolioError::Stopped)?
            .get(&key)
        {
            return Ok(index);
        }
        let index = Arc::new(build_index(report, &key, check)?);
        check()?;
        let mut cache = self.0.lock().map_err(|_| PortfolioError::Stopped)?;
        cache.entries.push_back((key, index.clone()));
        if cache.entries.len() > 32 {
            cache.entries.pop_front();
        }
        Ok(index)
    }
    pub fn read(
        &self,
        state: &PortfolioState,
        mut query: ResolvedPortfolioReadQuery,
        mut check: impl FnMut() -> Result<(), PortfolioError>,
    ) -> Result<PortfolioPage, PortfolioError> {
        query.validate()?;
        query.filters.normalize()?;
        check()?;
        let report = state.valuation.as_ref().ok_or_else(|| {
            PortfolioError::InvalidData("Portfolio is still being checked".into())
        })?;
        if report.items.len() > 10_000 {
            return Err(PortfolioError::Busy);
        }
        let version = state.version.as_ref().ok_or(PortfolioError::RestartRead)?;
        if state.user_id != query.user_id || report.currency_id != query.currency_id {
            return Err(PortfolioError::InvalidData(
                "portfolio subject or currency mismatch".into(),
            ));
        }
        if query
            .version
            .as_ref()
            .is_some_and(|wanted| wanted != version)
        {
            return Err(PortfolioError::RestartRead);
        }
        let key = Key {
            user: query.user_id,
            currency: query.currency_id,
            version: version.clone(),
            filters: query.filters,
            sort: query.sort,
        };
        let index = self.index(report, key, &mut check)?;
        check()?;
        let end = query
            .offset
            .saturating_add(query.limit)
            .min(index.positions.len());
        let items = index
            .positions
            .get(query.offset..end)
            .unwrap_or_default()
            .iter()
            .map(|index| report.items[*index].clone())
            .collect();
        Ok(PortfolioPage {
            version: version.clone(),
            offset: query.offset,
            has_more: end < index.positions.len(),
            items,
            selection: index.selection.clone(),
        })
    }
}
fn build_index(
    report: &InventoryValuation,
    key: &Key,
    check: &mut impl FnMut() -> Result<(), PortfolioError>,
) -> Result<Index, PortfolioError> {
    let mut positions = Vec::new();
    for (index, item) in report.items.iter().enumerate() {
        check()?;
        if key.filters.matches(item) {
            positions.push(index);
        }
    }
    let selection =
        PortfolioSelection::summarize(positions.iter().map(|index| &report.items[*index]))?;
    match key.sort {
        PortfolioSort::PositionAsc => {}
        PortfolioSort::PositionDesc => positions.reverse(),
        PortfolioSort::NumberAsc | PortfolioSort::NumberDesc => positions.sort_by_key(|index| {
            let gift = &report.items[*index].gift.gift;
            (
                if key.sort == PortfolioSort::NumberDesc {
                    -i64::from(gift.number.get())
                } else {
                    i64::from(gift.number.get())
                },
                &gift.id,
            )
        }),
        PortfolioSort::ValueAsc | PortfolioSort::ValueDesc => {
            positions.sort_by_cached_key(|index| {
                let item = &report.items[*index];
                let value = item.estimate.as_ref().map(Amount::decimal);
                let price = value.unwrap_or_default();
                (
                    value.is_none(),
                    if key.sort == PortfolioSort::ValueDesc {
                        -price
                    } else {
                        price
                    },
                    item.gift.gift.number,
                    &item.gift.gift.id,
                )
            })
        }
    }
    check()?;
    Ok(Index {
        positions,
        selection: Arc::new(selection),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn state(count: u32) -> PortfolioState {
        let items: Vec<_> = (1..=count)
            .rev()
            .map(|number| {
                let mut gift = crate::tests::gift(&number.to_string());
                if number % 7 == 0 {
                    gift.relation = crate::GiftRelation::Unverified;
                }
                InventoryValue {
                    resolved: Some(market_gifts::CollectibleGift {
                        id: gift.gift.id.clone(),
                        number: Some(gift.gift.number),
                        collection_id: if number % 2 == 0 {
                            "gift".into()
                        } else {
                            "other".into()
                        },
                        traits: market_gifts::GiftTraits {
                            model: Some(if number % 2 == 0 {
                                "m".into()
                            } else {
                                "otherm".into()
                            }),
                            backdrop: Some("b".into()),
                            pattern: None,
                        },
                    }),
                    gift,
                    estimate: (number % 10 != 0)
                        .then(|| Amount::new(format!("{number}.01")).unwrap()),
                    error: None,
                }
            })
            .collect();
        let summary = PortfolioSelection::summarize(&items).unwrap();
        PortfolioState {
            user_id: UserId::new(7).unwrap(),
            refresh_id: 1,
            progress: crate::PortfolioProgress::default(),
            inventory: None,
            version: Some(PortfolioVersion::new()),
            refresh_after: None,
            error: None,
            valuation: Some(Arc::new(InventoryValuation {
                currency_id: "GRAM".into(),
                evaluated_at: jiff::Timestamp::now(),
                stale: false,
                total: summary.total,
                priced: summary.priced,
                unpriced: summary.unpriced,
                items,
            })),
        }
    }
    fn query() -> ResolvedPortfolioReadQuery {
        PortfolioReadQuery {
            user_id: UserId::new(7).unwrap(),
            currency_id: "GRAM".into(),
            filters: Default::default(),
            sort: PortfolioSort::PositionAsc,
            offset: 0,
            limit: 100,
            version: None,
        }
    }
    #[test]
    fn six_thousand_gifts_page_without_cloning_or_rescanning_the_full_selection() {
        let state = state(6000);
        let cache = PortfolioReadCache::default();
        let first = cache.read(&state, query(), || Ok(())).unwrap();
        assert_eq!(first.items.len(), 100);
        assert_eq!(first.items[0].gift.gift.number.get(), 6000);
        assert_eq!(first.selection.count, 6000);
        assert_eq!(first.selection.priced, 5400);
        let index = cache.0.lock().unwrap().entries.back().unwrap().1.clone();
        let mut next = query();
        next.offset = 100;
        next.version = Some(first.version.clone());
        let page = cache.read(&state, next, || Ok(())).unwrap();
        assert_eq!(page.items[0].gift.gift.number.get(), 5900);
        assert!(Arc::ptr_eq(&first.selection, &page.selection));
        assert!(
            Arc::ptr_eq(&index, &cache.0.lock().unwrap().entries.back().unwrap().1),
            "adjacent pages share their filtered/sorted index"
        );
        let mut filtered = query();
        filtered.filters.collection_id = Some("gift".into());
        filtered.filters.models = vec!["m".into()];
        filtered.filters.backdrops = vec!["b".into()];
        let page = cache.read(&state, filtered, || Ok(())).unwrap();
        assert_eq!(page.selection.count, 3000);
        assert_eq!(page.items.len(), 100);
        let mut exact = query();
        exact.filters.number = Some(GiftNumber::new(1).unwrap());
        let page = cache.read(&state, exact, || Ok(())).unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.selection.total.as_ref().unwrap().as_str(), "1.01");
        let mut reverse = query();
        reverse.sort = PortfolioSort::PositionDesc;
        assert_eq!(
            cache.read(&state, reverse, || Ok(())).unwrap().items[0]
                .gift
                .gift
                .number
                .get(),
            1
        );
        let mut price = query();
        price.sort = PortfolioSort::ValueDesc;
        assert_eq!(
            cache.read(&state, price, || Ok(())).unwrap().items[0]
                .gift
                .gift
                .number
                .get(),
            5999
        );
        assert_eq!(
            state.valuation.as_ref().unwrap().items[0]
                .gift
                .gift
                .number
                .get(),
            6000,
            "reads preserve Telegram inventory order"
        );
    }
    #[test]
    fn exact_decimal_sorts_keep_unknown_values_last_and_unknown_totals_distinct() {
        let mut state = state(3);
        let report = Arc::make_mut(state.valuation.as_mut().unwrap());
        report.items[0].estimate = Some(Amount::new("9999999999999999.01").unwrap());
        report.items[1].estimate = Some(Amount::new("9999999999999999.02").unwrap());
        report.items[2].estimate = None;
        report.priced = 2;
        report.unpriced = 1;
        report.total = Some(Amount::new("19999999999999998.03").unwrap());
        let cache = PortfolioReadCache::default();
        for (sort, first) in [(PortfolioSort::ValueAsc, 3), (PortfolioSort::ValueDesc, 2)] {
            let mut q = query();
            q.sort = sort;
            let page = cache.read(&state, q, || Ok(())).unwrap();
            assert_eq!(page.items[0].gift.gift.number.get(), first);
            assert_eq!(page.items[2].gift.gift.number.get(), 1);
            assert_eq!(
                page.selection.total.as_ref().unwrap().as_str(),
                "19999999999999998.03"
            );
        }
        let mut unknown = query();
        unknown.filters.number = Some(GiftNumber::new(1).unwrap());
        assert!(
            cache
                .read(&state, unknown, || Ok(()))
                .unwrap()
                .selection
                .total
                .is_none()
        );
        let mut empty = query();
        empty.filters.number = Some(GiftNumber::new(99).unwrap());
        assert_eq!(
            cache
                .read(&state, empty, || Ok(()))
                .unwrap()
                .selection
                .total
                .as_ref()
                .unwrap()
                .as_str(),
            "0"
        );
    }

    #[test]
    fn building_one_selection_does_not_block_other_reads_and_coalesces_identical_reads() {
        use std::{sync::mpsc, thread, time::Duration};
        let state = Arc::new(state(6000));
        let cache = Arc::new(PortfolioReadCache::default());
        let (started, start) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let first = thread::spawn({
            let state = state.clone();
            let cache = cache.clone();
            move || {
                let mut checks = 0;
                cache.read(&state, query(), || {
                    checks += 1;
                    // Initial validation and build admission precede the first scanned row.
                    if checks == 3 {
                        started.send(()).unwrap();
                        released.recv().unwrap();
                    }
                    Ok(())
                })
            }
        });
        start.recv_timeout(Duration::from_secs(5)).unwrap();
        let (finished, finish) = mpsc::channel();
        let independent = thread::spawn({
            let state = state.clone();
            let cache = cache.clone();
            move || {
                let mut other = query();
                other.sort = PortfolioSort::NumberAsc;
                let page = cache.read(&state, other, || Ok(()));
                finished.send(()).unwrap();
                page
            }
        });
        let identical = thread::spawn({
            let state = state.clone();
            let cache = cache.clone();
            move || cache.read(&state, query(), || Ok(()))
        });
        let progressed = finish.recv_timeout(Duration::from_secs(5)).is_ok();
        release.send(()).unwrap();
        let first = first.join().unwrap().unwrap();
        let identical = identical.join().unwrap().unwrap();
        let independent = independent.join().unwrap().unwrap();
        assert!(
            progressed,
            "unrelated selections must progress during a build"
        );
        assert_eq!(independent.items[0].gift.gift.number.get(), 1);
        assert!(Arc::ptr_eq(&first.selection, &identical.selection));
        assert_eq!(cache.0.lock().unwrap().entries.len(), 2);
    }

    #[test]
    fn cancelled_builds_are_not_published_and_can_be_retried() {
        let state = state(6000);
        let cache = PortfolioReadCache::default();
        let mut checks = 0;
        let result = cache.read(&state, query(), || {
            checks += 1;
            if checks == 50 {
                Err(PortfolioError::Stopped)
            } else {
                Ok(())
            }
        });
        assert!(matches!(result, Err(PortfolioError::Stopped)));
        assert!(cache.0.lock().unwrap().entries.is_empty());
        assert_eq!(
            cache
                .read(&state, query(), || Ok(()))
                .unwrap()
                .selection
                .count,
            6000
        );
    }

    #[test]
    fn reads_validate_versions_limits_cancellation_and_cache_bounds() {
        let state = state(250);
        let cache = PortfolioReadCache::default();
        let mut invalid = query();
        invalid.limit = 101;
        assert!(cache.read(&state, invalid, || Ok(())).is_err());
        let mut unversioned = query();
        unversioned.offset = 100;
        assert!(
            cache.read(&state, unversioned, || Ok(())).is_err(),
            "continuation requires an explicit dataset version"
        );
        let mut old = query();
        old.version = Some(PortfolioVersion::new());
        assert!(matches!(
            cache.read(&state, old, || Ok(())),
            Err(PortfolioError::RestartRead)
        ));
        assert!(matches!(
            cache.read(&state, query(), || Err(PortfolioError::Stopped)),
            Err(PortfolioError::Stopped)
        ));
        for number in 1..=40 {
            let mut q = query();
            q.filters.number = Some(GiftNumber::new(number).unwrap());
            cache.read(&state, q, || Ok(())).unwrap();
        }
        assert_eq!(cache.0.lock().unwrap().entries.len(), 32);
        let mut last = query();
        last.offset = 200;
        last.version = state.version.clone();
        let page = cache.read(&state, last, || Ok(())).unwrap();
        assert_eq!(page.items.len(), 50);
        assert!(!page.has_more);
    }
}
