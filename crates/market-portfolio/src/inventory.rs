//! Bounded normalized-page accumulation, independent of actor scheduling and I/O.
use crate::*;
use market_gifts::GiftId;
use std::collections::{BTreeMap, HashSet};
#[derive(Default)]
pub(crate) struct InventoryBuilder {
    pub cursor: Option<String>,
    cursors: HashSet<String>,
    gifts: BTreeMap<GiftId, InventoryGift>,
    regular: usize,
    skipped: BTreeMap<SkipReason, usize>,
    pub pages: usize,
    pub scanned: usize,
    pub total: Option<usize>,
}
impl InventoryBuilder {
    pub fn snapshot(self, completion: InventoryCompletion) -> InventorySnapshot {
        InventorySnapshot {
            gifts: self.gifts.into_values().collect(),
            regular_gifts: self.regular,
            skipped: self.skipped,
            pages_read: self.pages,
            gifts_scanned: self.scanned,
            reported_total: self.total,
            completion,
            fetched_at: jiff::Timestamp::now(),
        }
    }
    pub fn merge(
        &mut self,
        page: InventoryPage,
        limit: usize,
        config: &PortfolioConfig,
    ) -> Result<Option<InventoryCompletion>, InventoryError> {
        let accounted = page
            .skipped
            .values()
            .copied()
            .chain([page.gifts.len(), page.regular_gifts])
            .try_fold(0usize, usize::checked_add);
        if page.scanned > limit
            || accounted.is_none_or(|count| count != page.scanned)
            || page
                .next_cursor
                .as_ref()
                .is_some_and(|cursor| cursor.len() > 4096)
        {
            return Err(InventoryError::InvalidData(
                "inventory page exceeds its bounds".into(),
            ));
        }
        self.pages += 1;
        self.scanned += page.scanned;
        self.regular += page.regular_gifts;
        self.total = page.reported_total.or(self.total);
        for (reason, count) in page.skipped {
            *self.skipped.entry(reason).or_default() += count;
        }
        for gift in page.gifts {
            self.gifts.entry(gift.gift.id.clone()).or_insert(gift);
        }
        let Some(next) = page.next_cursor.filter(|cursor| !cursor.is_empty()) else {
            return Ok(Some(
                if self.total.is_none_or(|total| self.scanned >= total) {
                    InventoryCompletion::Complete
                } else {
                    InventoryCompletion::Partial(PartialReason::MissingCursor)
                },
            ));
        };
        if !self.cursors.insert(next.clone()) {
            return Ok(Some(InventoryCompletion::Partial(
                PartialReason::RepeatedCursor,
            )));
        }
        if self.scanned >= config.max_gifts || self.pages >= config.max_pages {
            return Ok(Some(InventoryCompletion::Partial(
                PartialReason::LimitReached,
            )));
        }
        self.cursor = Some(next);
        Ok(None)
    }
}
