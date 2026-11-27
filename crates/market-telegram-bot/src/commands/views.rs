//! Immutable view destinations over shared reports and bounded reusable sales pages.
use super::{
    history::{self, Pages, SalesState, Scope},
    report::{Report, escape, report_error},
    reports::{ReportAction, ReportCallback, Reports},
    *,
};
use market_gifts::OrderKey;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Tab {
    Floors,
    Sales,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ViewState {
    pub tab: Tab,
    pub sales: SalesState,
    pub mode: GridMode,
}
impl ViewState {
    fn use_report(&mut self, previous: &Report, next: &Report) {
        let filters = |report: &Report| {
            report
                .gift()
                .and_then(|gift| {
                    self.sales
                        .query(gift, report.query.currency_id.clone())
                        .ok()
                })
                .map(|query| query.filters)
        };
        if filters(previous) != filters(next) {
            self.sales.starts.clear();
        }
        if let Some(gift) = next.gift()
            && !self.sales.scope.supported(gift)
        {
            self.sales.scope = Scope::ALL
                .into_iter()
                .find(|scope| scope.supported(gift))
                .expect("collection scope always supported");
        }
    }
}
#[derive(Clone)]
pub(super) enum Action {
    Tab(Tab),
    Scope(Scope),
    Page(usize),
    Refresh,
    Retry,
    Mode(GridMode),
}
#[derive(Clone)]
pub(super) struct View {
    pub report: Arc<Report>,
    pub state: ViewState,
    pages: Arc<Pages>,
    documents: Arc<orders::ModelDocuments>,
    next: Option<OrderKey>,
}
fn model_documents(report: &Report) -> orders::ModelDocuments {
    report
        .gift()
        .and_then(|gift| gift.traits.model.clone())
        .zip(report.artwork)
        .and_then(|(id, cover)| {
            market_gifts::TelegramDocumentId::new(cover.model.get())
                .ok()
                .map(|document| (id, document))
        })
        .into_iter()
        .collect()
}
impl View {
    pub fn initial(report: Arc<Report>) -> Option<Self> {
        let documents = model_documents(&report);
        Some(Self {
            state: ViewState {
                tab: Tab::Floors,
                sales: SalesState::initial(&report)?,
                mode: GridMode::Linked,
            },
            pages: Arc::new(Pages::default()),
            documents: Arc::new(documents),
            report,
            next: None,
        })
    }
    fn destination(&self, action: Action) -> Destination {
        let mut target = Destination {
            tab: self.state.tab,
            scope: self.state.sales.scope,
            page: self.state.sales.page(),
            mode: GridMode::Linked,
            refresh: false,
        };
        match action {
            Action::Tab(tab) => target.tab = tab,
            Action::Scope(scope) => {
                target.tab = Tab::Sales;
                target.scope = scope;
                target.page = 1;
            }
            Action::Page(page) => target.page = page,
            Action::Refresh => target.refresh = true,
            Action::Retry => {}
            Action::Mode(mode) => target.mode = mode,
        }
        target
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Destination {
    pub tab: Tab,
    pub scope: Scope,
    pub page: usize,
    pub mode: GridMode,
    pub refresh: bool,
}
struct Buttons {
    id: u64,
}
impl Buttons {
    fn button(&self, view: &View, action: Option<Action>, label: &str, selected: bool) -> String {
        let data = action.map(|action| {
            ReportCallback::new(self.id, ReportAction::Gift(view.destination(action))).to_string()
        });
        rich::button(label, data.as_deref(), selected)
    }
    fn choices(&self, view: &View) -> String {
        let mut html = String::from("<tg-button-row>");
        for (tab, label) in [(Tab::Floors, "Floors"), (Tab::Sales, "Sales")] {
            let selected = view.state.tab == tab;
            html.push_str(&self.button(
                view,
                (!selected).then_some(Action::Tab(tab)),
                label,
                selected,
            ));
        }
        html.push_str("</tg-button-row>");
        html
    }
}
impl MarketCommands {
    pub(super) fn open_report(&self, report: Arc<Report>) -> Reply {
        match View::initial(report.clone()) {
            Some(view) => render(&self.reports, &view, None, None),
            None => report.render(),
        }
    }
    pub(super) async fn navigate_gift(&self, original: &View, target: Destination) -> Reply {
        let mut view = original.clone();
        let mut notice = None;
        if !view
            .report
            .gift()
            .is_some_and(|gift| target.scope.supported(gift))
        {
            return Reply::notice("These gift conditions are unavailable. Request a fresh report.");
        }
        if target.scope != view.state.sales.scope {
            if target.page != 1 {
                return Reply::notice("Open the first page of these sales first.");
            }
            view.state.sales.starts.clear();
        } else if target.page <= view.state.sales.page() {
            view.state
                .sales
                .starts
                .truncate(target.page.saturating_sub(1));
        } else if target.page == view.state.sales.page() + 1 {
            let Some(next) = view.next.clone() else {
                return Reply::notice("No more sales.");
            };
            view.state.sales.starts.push(next);
        } else {
            return Reply::notice("Open the previous sales page first.");
        }
        view.state.tab = target.tab;
        view.state.sales.scope = target.scope;
        view.state.mode = target.mode;
        if target.refresh {
            match self.load_report(view.report.query.clone()).await {
                Ok(report) => {
                    view.state.use_report(&view.report, &report);
                    view.documents = Arc::new(model_documents(&report));
                    view.report = report;
                }
                Err(error) => notice = Some(report_error(&error)),
            }
        }
        let mut page = None;
        if view.state.tab == Tab::Sales
            && let Some(gift) = view.report.gift()
            && let Ok(query) = view
                .state
                .sales
                .query(gift, view.report.query.currency_id.clone())
        {
            page = view
                .pages
                .read(query, |query| async {
                    tokio::time::timeout(
                        std::time::Duration::from_secs(3),
                        self.market.query_orders(market_actors::input::query(query)),
                    )
                    .await
                    .ok()?
                    .ok()
                    .map(|read| read.result)
                })
                .await;
            if let Some(page) = &page {
                let missing = self.order_documents(&page.data, &view.documents).await;
                if !missing.is_empty() {
                    Arc::make_mut(&mut view.documents).extend(missing);
                }
            }
        }
        let mut reply = render(&self.reports, &view, page.as_deref(), notice.as_deref());
        if view.state.tab == Tab::Sales
            && view.state.mode == GridMode::Gallery
            && let Some(page) = &page
        {
            reply.artwork = Some(
                self.order_gallery(&page.data, &view.documents, view.report.catalog.as_ref(), 2)
                    .await,
            );
        }
        reply
    }
}
pub(super) fn render(
    reports: &Reports,
    view: &View,
    page: Option<&market_actors::OrderResult>,
    notice: Option<&str>,
) -> Reply {
    let mut cached = view.clone();
    cached.next = page
        .and_then(|p| p.range.end.as_ref())
        .and_then(|key| key.encode().try_into().ok());
    let has_next = cached.next.is_some();
    let views = Buttons {
        id: reports.gift(cached),
    };
    let mut reply = view.report.render_panel(
        view.state.tab == Tab::Floors,
        &views.choices(view),
        &if view.state.mode == GridMode::Linked {
            views.button(view, Some(Action::Refresh), "↻", false)
        } else {
            String::new()
        },
    );
    let html = reply.rich_html.as_mut().expect("gift report HTML");
    if let Some(notice) = notice {
        html.push_str(&format!("<p>{}</p>", escape(notice)));
        reply.text.push_str(&format!("\n\n{notice}"));
    }
    if view.state.tab == Tab::Sales {
        html.push_str("<tg-button-row>");
        for (mode, label) in [
            (
                GridMode::Linked,
                if view.state.mode == GridMode::Gallery {
                    "Browse"
                } else {
                    "Linked"
                },
            ),
            (GridMode::Gallery, "Gallery"),
        ] {
            html.push_str(&views.button(
                view,
                (mode != view.state.mode).then_some(Action::Mode(mode)),
                label,
                mode == view.state.mode,
            ));
        }
        html.push_str("</tg-button-row>");
        let gift = view.report.gift().expect("resolved report gift");
        let scopes = Scope::ALL
            .into_iter()
            .filter(|scope| scope.supported(gift))
            .collect::<Vec<_>>();
        if view.state.mode == GridMode::Linked && scopes.len() > 1 {
            for row in scopes.chunks(2) {
                html.push_str("<tg-button-row>");
                for scope in row {
                    let selected = view.state.sales.scope == *scope;
                    html.push_str(&views.button(
                        view,
                        (!selected).then_some(Action::Scope(*scope)),
                        scope.label(),
                        selected,
                    ));
                }
                html.push_str("</tg-button-row>");
            }
        }
        match page {
            Some(page) => history::render(
                &view.report,
                &view.state.sales,
                page,
                &mut reply,
                &view.documents,
            ),
            None => {
                let notice = "Sales are temporarily unavailable.";
                html.push_str(&format!("<p>{notice}</p>"));
                reply.text.push_str(&format!("\n\n{notice}"));
                html.push_str(&views.button(view, Some(Action::Retry), "Retry sales", false));
            }
        }
        let html = reply.rich_html.as_mut().expect("gift report HTML");
        let next = (has_next && view.state.sales.page() < history::MAX_PAGES)
            .then_some(Action::Page(view.state.sales.page() + 1));
        let previous = (!view.state.sales.starts.is_empty())
            .then_some(Action::Page(view.state.sales.page().saturating_sub(1)));
        if view.state.mode == GridMode::Linked && (previous.is_some() || next.is_some()) {
            html.push_str("<tg-button-row>");
            html.push_str(&views.button(view, previous, "←", false));
            html.push_str(&views.button(view, None, &view.state.sales.page().to_string(), false));
            html.push_str(&views.button(view, next, "→", false));
            html.push_str("</tg-button-row>");
        }
    }
    reply
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refreshed_reports_seed_model_documents_from_the_new_artwork() {
        let mut report = super::super::tests::view();
        Arc::get_mut(&mut report).unwrap().artwork = Some(market_media::GiftCover {
            model: market_media::DocumentId::new(10).unwrap(),
            pattern: None,
            palette: None,
        });
        let view = View::initial(report.clone()).unwrap();
        assert_eq!(view.documents["m"].get(), 10);
        let mut refreshed = Arc::new(Report {
            query: report.query.clone(),
            value: report.value.clone(),
            catalog: report.catalog.clone(),
            floors: super::super::report::FloorQuotes::default(),
            icons: report.icons.clone(),
            artwork: Some(market_media::GiftCover {
                model: market_media::DocumentId::new(20).unwrap(),
                pattern: None,
                palette: None,
            }),
        });
        assert_eq!(model_documents(&refreshed)["m"].get(), 20);
        Arc::get_mut(&mut refreshed).unwrap().artwork = None;
        assert!(model_documents(&refreshed).is_empty());
    }
    #[test]
    fn sales_hide_unusable_controls_but_keep_navigation_and_recovery_when_needed() {
        let mut report = super::super::tests::view();
        let value = Arc::make_mut(&mut Arc::get_mut(&mut report).unwrap().value);
        let market_valuation::ResolvedTarget::Gift { gift, .. } = &mut value.target else {
            panic!()
        };
        gift.traits = Default::default();
        let mut view = View::initial(report).unwrap();
        view.state.tab = Tab::Sales;
        let page = market_actors::OrderResult {
            data: vec![],
            range: market_actors::KeyRange {
                start: None,
                end: None,
            },
        };
        let views = Reports::default();
        let reply = render(&views, &view, Some(&page), None);
        let html = reply.rich_html.unwrap();
        assert_eq!(
            html.matches("<tg-button-row>").count(),
            2,
            "only tabs and layout choices remain"
        );
        assert!(!html.contains(">Collection</tg-button>"));
        assert!(!html.contains(">1</tg-button>"));
        assert!(reply.text.contains("Recent sales · Collection · page 1"));
        assert!(html.contains("No observed executions"));
        assert!(html.contains("<h1>"));
        assert_eq!(html.matches(">↻</tg-button>").count(), 1);

        view.state
            .sales
            .starts
            .push("00".to_owned().try_into().unwrap());
        let missing = render(&views, &view, None, Some("Updates unavailable"));
        let html = missing.rich_html.unwrap();
        assert!(html.contains("type=\"callback_data\" data=\"r:"));
        assert!(
            html.contains(">←</tg-button>"),
            "can return to a cached page"
        );
        assert!(html.contains(">2</tg-button>"));
        assert!(html.contains("type=\"disabled\">→</tg-button>"));
        assert!(html.contains(">Retry sales</tg-button>"));
        assert!(!html.contains("Retry valuation"));
        assert_eq!(html.matches(">↻</tg-button>").count(), 1);
        assert!(missing.text.contains("Updates unavailable"));
    }
    #[test]
    fn report_updates_preserve_tabs_and_only_reset_incompatible_sales_coverage() {
        let base = super::super::tests::view();
        for changed in ["none", "model", "backdrop_removed", "pattern", "currency"] {
            let mut next = super::super::tests::view();
            let report = Arc::get_mut(&mut next).unwrap();
            let value = Arc::make_mut(&mut report.value);
            let market_valuation::ResolvedTarget::Gift { gift, .. } = &mut value.target else {
                panic!()
            };
            match changed {
                "model" => gift.traits.model = Some("changed-model".into()),
                "backdrop_removed" => gift.traits.backdrop = None,
                "pattern" => gift.traits.pattern = Some("changed-symbol".into()),
                "currency" => {
                    report.query.currency_id = "XTR".into();
                    value.currency_id = "XTR".into();
                }
                _ => {}
            }
            let mut state = View::initial(base.clone()).unwrap().state;
            state.tab = Tab::Sales;
            state.sales.starts.push("00".to_owned().try_into().unwrap());
            let until = state.sales.until;
            state.use_report(&base, &next);
            assert_eq!(state.tab, Tab::Sales);
            assert_eq!(state.sales.until, until);
            assert_eq!(
                state.sales.page(),
                if matches!(changed, "none" | "pattern") {
                    2
                } else {
                    1
                },
                "{changed}"
            );
            assert_eq!(
                state.sales.scope,
                if changed == "backdrop_removed" {
                    Scope::Model
                } else {
                    Scope::Pair
                }
            );
        }
    }
}
