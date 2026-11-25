//! On-demand comparable sales with fixed coverage and bounded, coalesced page reuse.
use super::orders;
use super::{report::Report, *};
use market_gifts::*;
use std::{collections::VecDeque, sync::Arc};

pub(super) const MAX_PAGES: usize = 10;
pub(super) const PAGE_SIZE: u32 = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Scope {
    Pair,
    Model,
    Backdrop,
    Collection,
}
impl Scope {
    pub const ALL: [Self; 4] = [Self::Pair, Self::Model, Self::Backdrop, Self::Collection];
    pub fn label(self) -> &'static str {
        match self {
            Self::Pair => "Model + backdrop",
            Self::Model => "Model",
            Self::Backdrop => "Backdrop",
            Self::Collection => "Collection",
        }
    }
    pub fn supported(self, gift: &CollectibleGift) -> bool {
        match self {
            Self::Pair => gift.traits.model.is_some() && gift.traits.backdrop.is_some(),
            Self::Model => gift.traits.model.is_some(),
            Self::Backdrop => gift.traits.backdrop.is_some(),
            Self::Collection => true,
        }
    }
}
/// Previous page starts belong to this view, not to the report or shared cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SalesState {
    pub scope: Scope,
    pub until: jiff::Timestamp,
    pub starts: Vec<OrderKey>,
}
impl SalesState {
    pub fn initial(report: &Report) -> Option<Self> {
        let gift = report.gift()?;
        Some(Self {
            scope: Scope::ALL.into_iter().find(|scope| scope.supported(gift))?,
            until: jiff::Timestamp::now(),
            starts: Vec::new(),
        })
    }
    pub fn page(&self) -> usize {
        self.starts.len() + 1
    }
    pub fn query(
        &self,
        gift: &CollectibleGift,
        currency: CurrencyId,
    ) -> Result<ResolvedGiftOrderQuery, BotError> {
        if !self.scope.supported(gift) || self.page() > MAX_PAGES {
            return Err(BotError::InvalidResponse);
        }
        let mut filters = ResolvedGiftFilters::new(gift.collection_id.clone(), currency);
        if matches!(self.scope, Scope::Pair | Scope::Model) {
            filters.models = gift.traits.model.clone().into_iter().collect();
        }
        if matches!(self.scope, Scope::Pair | Scope::Backdrop) {
            filters.backdrops = gift.traits.backdrop.clone().into_iter().collect();
        }
        let mut query = ResolvedGiftOrderQuery::new(filters);
        query.source = OrderSource::History;
        query.sort = Some(GiftSort::TimeDesc);
        query.limit = PAGE_SIZE;
        query.period = Some(Period {
            from: Some(
                self.until
                    .checked_sub(jiff::SignedDuration::from_secs(30 * 86400))
                    .map_err(|_| BotError::InvalidResponse)?,
            ),
            until: Some(self.until),
        });
        query.range.start = self.starts.last().cloned();
        Ok(query)
    }
}
#[derive(Default)]
pub(super) struct Pages(tokio::sync::Mutex<VecDeque<(ResolvedGiftOrderQuery, PageSlot)>>);
type PageSlot = Arc<tokio::sync::OnceCell<Arc<market_actors::OrderResult>>>;
impl Pages {
    pub async fn read<F, Fut>(
        &self,
        query: ResolvedGiftOrderQuery,
        load: F,
    ) -> Option<Arc<market_actors::OrderResult>>
    where
        F: FnOnce(ResolvedGiftOrderQuery) -> Fut,
        Fut: std::future::Future<Output = Option<market_actors::OrderResult>>,
    {
        let slot = {
            let mut entries = self.0.lock().await;
            let slot = entries
                .iter()
                .position(|(key, _)| key == &query)
                .and_then(|index| entries.remove(index))
                .map(|(_, slot)| slot)
                .unwrap_or_default();
            entries.push_back((query.clone(), slot.clone()));
            while entries.len() > 32 {
                entries.pop_front();
            }
            slot
        };
        // Only identical reads wait for each other. Failure/cancellation leaves the slot retryable.
        slot.get_or_try_init(|| async { load(query).await.map(Arc::new).ok_or(()) })
            .await
            .ok()
            .cloned()
    }
}
/// SalesState supplies only collection/currency, one model/backdrop and a fixed period.
/// Keep captured previews and defensive rendering consistent with that live query.
fn matches_sale(query: &ResolvedGiftOrderQuery, order: &GiftOrder) -> bool {
    let Some(time) = order.executed_at else {
        return false;
    };
    if order.kind != GiftOrderKind::Executed
        || query
            .filters
            .collection_id
            .as_ref()
            .is_some_and(|id| &order.book.collection_id != id)
        || order.book.currency_id != query.filters.currency_id
        || query.period.as_ref().is_none_or(|period| {
            period.from.is_none_or(|from| time < from)
                || period.until.is_none_or(|until| time >= until)
        })
    {
        return false;
    }
    let (model, backdrop) = match &order.target {
        GiftTarget::Gift { gift } => (gift.traits.model.as_ref(), gift.traits.backdrop.as_ref()),
        // Constraints with several accepted values cannot establish the executed gift's traits.
        GiftTarget::Criteria { criteria } => (
            (criteria.models.len() == 1).then(|| &criteria.models[0]),
            (criteria.backdrops.len() == 1).then(|| &criteria.backdrops[0]),
        ),
        GiftTarget::Unsupported { .. } => return false,
    };
    query
        .filters
        .models
        .first()
        .is_none_or(|wanted| model == Some(wanted))
        && query
            .filters
            .backdrops
            .first()
            .is_none_or(|wanted| backdrop == Some(wanted))
}
#[cfg(test)]
pub(super) fn captured_page(
    report: &Report,
    state: &SalesState,
    mut rows: Vec<market_actors::OrderedEntry>,
) -> Result<market_actors::OrderResult, BotError> {
    let query = state.query(
        report.gift().ok_or(BotError::InvalidResponse)?,
        report.query.currency_id.clone(),
    )?;
    rows.retain(|row| matches_sale(&query, &row.order));
    rows.sort_by_key(|row| std::cmp::Reverse(row.order.executed_at));
    rows.truncate(query.limit as usize);
    Ok(market_actors::OrderResult {
        data: rows,
        range: market_actors::KeyRange {
            start: None,
            end: None,
        },
    })
}
pub(super) fn render(
    report: &Report,
    state: &SalesState,
    result: &market_actors::OrderResult,
    reply: &mut Reply,
    documents: &orders::ModelDocuments,
) {
    let Some(query) = report
        .gift()
        .and_then(|gift| state.query(gift, report.query.currency_id.clone()).ok())
    else {
        return;
    };
    let period = query.period.as_ref().expect("checked sales period");
    let from = period.from.unwrap();
    let until = period.until.unwrap();
    let same_year = from.strftime("%Y").to_string() == until.strftime("%Y").to_string();
    let coverage = format!(
        "{} – {} · UTC",
        from.strftime(if same_year { "%d %b" } else { "%d %b %Y" }),
        until.strftime("%d %b %Y")
    );
    reply
        .rich_html
        .as_mut()
        .unwrap()
        .push_str("<p>30 days · UTC</p>");
    reply.text.push_str(&format!(
        "\n\nRecent sales · {} · page {}\n{coverage}\n",
        state.scope.label(),
        state.page()
    ));
    let mut rows = result
        .data
        .iter()
        .filter(|row| matches_sale(&query, &row.order))
        .take(query.limit as usize)
        .peekable();
    if rows.peek().is_none() {
        reply
            .rich_html
            .as_mut()
            .unwrap()
            .push_str("<p>No observed executions in this period.</p>");
        reply
            .text
            .push_str("No observed executions in this period.\n");
    } else {
        let collections = report
            .catalog
            .as_ref()
            .map(|catalog| std::slice::from_ref(&catalog.collection))
            .unwrap_or_default();
        orders::render(
            reply,
            rows.map(|row| &row.order),
            &report.icons,
            collections,
            documents,
            report
                .gift()
                .map(|gift| (&gift.collection_id, &report.query.gift)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, time::Duration};
    fn empty_page() -> market_actors::OrderResult {
        market_actors::OrderResult {
            data: vec![],
            range: market_actors::KeyRange {
                start: None,
                end: None,
            },
        }
    }
    fn sale(time: jiff::Timestamp) -> GiftOrder {
        let report = super::super::tests::view();
        GiftOrder::try_from(GiftOrderData {
            id: "sale".into(),
            book: GiftBook {
                venue_id: "tonnel".into(),
                collection_id: "collection".into(),
                currency_id: "GRAM".into(),
            },
            kind: GiftOrderKind::Executed,
            target: GiftTarget::Gift {
                gift: report.gift().unwrap().clone(),
            },
            price: Amount::new("1234.5600").unwrap(),
            quantity: 1,
            observed_at: time,
            executed_at: Some(time),
            expires_at: None,
            market_url: Some("https://example.com/sale?gift=123&venue=tonnel".into()),
        })
        .unwrap()
    }
    #[test]
    fn sales_validation_matches_half_open_period_currency_collection_and_explicit_traits() {
        let report = super::super::tests::view();
        let until = "2026-11-23T12:00:00Z".parse::<jiff::Timestamp>().unwrap();
        let state = SalesState {
            scope: Scope::Pair,
            until,
            starts: vec![],
        };
        let query = state.query(report.gift().unwrap(), "GRAM".into()).unwrap();
        let from = query.period.as_ref().unwrap().from.unwrap();
        assert!(matches_sale(&query, &sale(from)));
        assert!(!matches_sale(
            &query,
            &sale(
                from.checked_sub(jiff::SignedDuration::from_millis(1))
                    .unwrap()
            )
        ));
        assert!(!matches_sale(&query, &sale(until)));
        let time = until
            .checked_sub(jiff::SignedDuration::from_secs(1))
            .unwrap();
        assert!(matches_sale(&query, &sale(time)));
        for changed in [
            "currency",
            "collection",
            "model",
            "backdrop",
            "kind",
            "ambiguous",
        ] {
            let mut draft = sale(time).into_data();
            match changed {
                "currency" => draft.book.currency_id = "XTR".into(),
                "collection" => {
                    draft.book.collection_id = "another".into();
                    let GiftTarget::Gift { gift } = &mut draft.target else {
                        panic!()
                    };
                    gift.collection_id = "another".into();
                }
                "model" => {
                    let GiftTarget::Gift { gift } = &mut draft.target else {
                        panic!()
                    };
                    gift.traits.model = Some("different".into());
                }
                "backdrop" => {
                    let GiftTarget::Gift { gift } = &mut draft.target else {
                        panic!()
                    };
                    gift.traits.backdrop = None;
                }
                "kind" => {
                    draft.kind = GiftOrderKind::Ask;
                    draft.executed_at = None;
                }
                "ambiguous" => {
                    draft.target = GiftTarget::Criteria {
                        criteria: GiftCriteria {
                            models: vec!["m".into(), "different".into()],
                            backdrops: vec!["b".into()],
                            ..Default::default()
                        },
                    }
                }
                _ => unreachable!(),
            }
            assert!(
                !matches_sale(&query, &GiftOrder::try_from(draft).unwrap()),
                "{changed}"
            );
        }
        let mut collection = state.clone();
        collection.scope = Scope::Collection;
        let mut draft = sale(time).into_data();
        draft.target = GiftTarget::Criteria {
            criteria: GiftCriteria::default(),
        };
        assert!(matches_sale(
            &collection
                .query(report.gift().unwrap(), "GRAM".into())
                .unwrap(),
            &GiftOrder::try_from(draft).unwrap()
        ));
    }
    fn grid(report: &Report, rows: &[GiftOrder]) -> Reply {
        let mut reply = Reply::default();
        let collections = report
            .catalog
            .as_ref()
            .map(|c| std::slice::from_ref(&c.collection))
            .unwrap_or_default();
        let documents = [("m".into(), TelegramDocumentId::new(54321).unwrap())].into();
        orders::render(
            &mut reply,
            rows,
            &report.icons,
            collections,
            &documents,
            report
                .gift()
                .map(|gift| (&gift.collection_id, &report.query.gift)),
        );
        reply
    }
    #[test]
    fn sales_grid_links_models_prices_and_safe_venues_and_marks_the_current_gift() {
        let report = super::super::tests::view();
        let time = "2026-11-23T12:34:56Z".parse().unwrap();
        let reply = grid(&report, &[sale(time)]);
        let html = reply.rich_html.unwrap();
        assert!(html.contains("<table striped compact>"));
        assert!(html.contains("https://t.me/nft/Gift-123\"><tg-emoji emoji-id=\"54321\""));
        assert!(html.contains("<b>1 234.56</b>"));
        assert!(html.contains(&report.icons.html(crate::ReportIcon::Gram)));
        assert!(html.contains(&format!(
            "{} <b>#123</b>",
            report.icons.html(crate::ReportIcon::CurrentGift)
        )));
        assert!(html.contains("23 Nov 12:34"));
        assert!(html.contains("https://example.com/sale?gift=123&amp;venue=tonnel\">Tonnel</a>"));
        assert!(reply.text.contains("1 234.56 GRAM"));
        assert_eq!(
            reply.emoji_documents,
            vec![TelegramDocumentId::new(54321).unwrap()]
        );
        for url in [
            "javascript:alert(1)",
            "https://user:private@example.com/sale",
            "not-a-url",
            "https://example.com/a\nb",
        ] {
            let mut draft = sale(time).into_data();
            draft.market_url = Some(url.into());
            let GiftTarget::Gift { gift } = &mut draft.target else {
                panic!()
            };
            gift.number = Some(GiftNumber::new(124).unwrap());
            let reply = grid(&report, &[GiftOrder::try_from(draft).unwrap()]);
            let html = reply.rich_html.unwrap();
            assert!(html.contains("https://t.me/nft/Gift-124"));
            assert!(!html.contains("<b>#124</b>"));
            assert!(!html.contains("javascript:"));
            assert!(!reply.text.contains("private"));
        }
    }
    #[test]
    fn sales_period_caption_keeps_year_boundaries_and_bulk_criteria_do_not_claim_one_gift() {
        let report = super::super::tests::view();
        let until = "2027-01-07T12:00:00Z".parse().unwrap();
        let state = SalesState {
            scope: Scope::Collection,
            until,
            starts: vec![],
        };
        let mut reply = Reply {
            rich_html: Some(String::new()),
            ..Default::default()
        };
        render(
            &report,
            &state,
            &empty_page(),
            &mut reply,
            &Default::default(),
        );
        let html = reply.rich_html.unwrap();
        let text = reply.text;
        assert!(text.contains("08 Dec 2026 – 07 Jan 2027 · UTC"));
        assert!(html.contains("30 days · UTC"));
        assert!(html.contains("No observed executions"));
        let mut draft = sale(until).into_data();
        draft.quantity = 2;
        draft.target = GiftTarget::Criteria {
            criteria: GiftCriteria {
                number: Some(GiftNumberRange {
                    min: Some(GiftNumber::new(123).unwrap()),
                    max: Some(GiftNumber::new(123).unwrap()),
                }),
                ..Default::default()
            },
        };
        let bulk = GiftOrder::try_from(draft).unwrap();
        assert_eq!(orders::number(&bulk), None);
        let reply = grid(&report, &[bulk]);
        assert!(!reply.rich_html.unwrap().contains("t.me/nft"));
    }
    #[test]
    fn bid_grids_keep_bulk_conditions_ambiguous_models_and_native_currency_honest() {
        let report = super::super::tests::view();
        let mut draft = sale("2026-11-23T12:34:56Z".parse().unwrap()).into_data();
        draft.kind = GiftOrderKind::Bid;
        draft.executed_at = None;
        draft.quantity = 5;
        draft.book.currency_id = "XTR".into();
        draft.target = GiftTarget::Criteria {
            criteria: GiftCriteria {
                models: vec!["m".into(), "other".into()],
                number: Some(GiftNumberRange {
                    min: Some(GiftNumber::new(123).unwrap()),
                    max: Some(GiftNumber::new(123).unwrap()),
                }),
                ..Default::default()
            },
        };
        let reply = grid(&report, &[GiftOrder::try_from(draft).unwrap()]);
        let html = reply.rich_html.unwrap();
        assert!(html.contains("<table striped compact>"));
        assert!(html.contains("&lt;unsafe&gt; &amp; friends × 5"));
        assert!(html.contains(&report.icons.html(crate::ReportIcon::Stars)));
        assert!(!html.contains("https://t.me/nft/"));
        assert!(!html.contains("<b>#123</b>"));
        assert!(reply.emoji_documents.is_empty(), "no single model is known");
        assert!(reply.text.contains("1 234.56 XTR"));
        assert!(!reply.text.contains("23 Nov"));
    }
    #[tokio::test]
    async fn visited_pages_coalesce_and_queries_isolate_traits_scope_currency_period_and_boundary()
    {
        let pages = Pages::default();
        let report = super::super::tests::view();
        let state = SalesState::initial(&report).unwrap();
        let query = state.query(report.gift().unwrap(), "GRAM".into()).unwrap();
        let count = Cell::new(0);
        let load = |_| async {
            count.set(count.get() + 1);
            tokio::time::sleep(Duration::from_millis(1)).await;
            Some(empty_page())
        };
        let (a, b) = tokio::join!(
            pages.read(query.clone(), load),
            pages.read(query.clone(), load)
        );
        assert!(Arc::ptr_eq(&a.unwrap(), &b.unwrap()));
        assert_eq!(count.get(), 1, "concurrent visits share one read");
        let mut other = query.clone();
        other.filters.models.clear();
        other.filters.backdrops.clear();
        pages.read(other, load).await.unwrap();
        let mut other = query.clone();
        other.filters.currency_id = "XTR".into();
        pages.read(other, load).await.unwrap();
        let mut other = query.clone();
        other.period.as_mut().unwrap().until = Some(
            state
                .until
                .checked_sub(jiff::SignedDuration::from_secs(1))
                .unwrap(),
        );
        pages.read(other, load).await.unwrap();
        let mut other = query.clone();
        other.range.start = Some("00".to_owned().try_into().unwrap());
        pages.read(other, load).await.unwrap();
        let mut other = query.clone();
        other.filters.models = vec!["updated-model".into()];
        pages.read(other, load).await.unwrap();
        assert_eq!(
            count.get(),
            6,
            "updated resolved traits must not reuse old scope results"
        );
        pages
            .read(query, |_| async {
                panic!("returning to a visited page must not reread")
            })
            .await
            .unwrap();
    }
    #[tokio::test]
    async fn slow_page_reads_do_not_block_cached_pages_or_other_scopes() {
        let pages = Pages::default();
        let report = super::super::tests::view();
        let state = SalesState::initial(&report).unwrap();
        let query = state.query(report.gift().unwrap(), "GRAM".into()).unwrap();
        let cached = pages
            .read(query.clone(), |_| async { Some(empty_page()) })
            .await
            .unwrap();
        let mut slow_query = query.clone();
        slow_query.filters.models.clear();
        let started = tokio::sync::Notify::new();
        let finished = tokio::sync::Notify::new();
        let slow = pages.read(slow_query, |_| async {
            started.notify_one();
            finished.notified().await;
            Some(empty_page())
        });
        tokio::pin!(slow);
        tokio::select! {
            biased;
            _ = &mut slow => panic!("read must wait until released"),
            _ = started.notified() => {}
        }
        let reused = tokio::time::timeout(
            Duration::from_secs(1),
            pages.read(query.clone(), |_| async { panic!("cached page reread") }),
        )
        .await
        .expect("a pending scope must not block a cached page")
        .unwrap();
        assert!(Arc::ptr_eq(&cached, &reused));
        let mut other = query;
        other.filters.backdrops.clear();
        tokio::time::timeout(
            Duration::from_secs(1),
            pages.read(other, |_| async { Some(empty_page()) }),
        )
        .await
        .expect("different scopes must load independently")
        .unwrap();
        finished.notify_one();
        slow.await.unwrap();
    }
    #[tokio::test]
    async fn cancelled_failed_loads_release_the_cache_and_lru_is_bounded() {
        let pages = Pages::default();
        let report = super::super::tests::view();
        let state = SalesState::initial(&report).unwrap();
        let query = state.query(report.gift().unwrap(), "GRAM".into()).unwrap();
        assert!(
            tokio::time::timeout(
                Duration::from_millis(1),
                pages.read(query.clone(), |_| std::future::pending())
            )
            .await
            .is_err()
        );
        assert!(
            pages
                .read(query.clone(), |_| async { None })
                .await
                .is_none()
        );
        pages
            .read(query.clone(), |_| async { Some(empty_page()) })
            .await
            .unwrap();
        for i in 1..=32 {
            let mut next = query.clone();
            next.period.as_mut().unwrap().until = Some(
                state
                    .until
                    .checked_sub(jiff::SignedDuration::from_secs(i))
                    .unwrap(),
            );
            pages
                .read(next, |_| async { Some(empty_page()) })
                .await
                .unwrap();
        }
        assert_eq!(pages.0.lock().await.len(), 32);
        let calls = Cell::new(0);
        pages
            .read(query, |_| async {
                calls.set(calls.get() + 1);
                Some(empty_page())
            })
            .await
            .unwrap();
        assert_eq!(calls.get(), 1, "an evicted page can be loaded again");
    }
}
