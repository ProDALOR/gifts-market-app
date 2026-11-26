use super::*;
use std::sync::Arc;
use tokio::sync::OnceCell;
const PAGE_SIZE: u32 = 12;
const MAX_PAGES: usize = 10;
#[derive(Clone)]
enum Action {
    Next,
    Previous,
    Refresh,
    Mode(GridMode),
}
struct Snapshot {
    id: ReadSnapshotId,
    market: MarketDataHandle,
    runtime: tokio::runtime::Handle,
}
impl Drop for Snapshot {
    fn drop(&mut self) {
        let id = self.id.clone();
        let market = self.market.clone();
        self.runtime.spawn(async move {
            let _ = market.release_read_snapshot(id).await;
        });
    }
}
#[derive(Clone)]
pub(in crate::commands) struct View {
    selection: Arc<MarketSelection>,
    query: ResolvedGiftOrderQuery,
    _lease: Option<Arc<Snapshot>>,
    pages: Vec<Arc<market_actors::OrderResult>>,
    page: usize,
    catalog: Arc<GiftCatalog>,
    documents: Arc<orders::ModelDocuments>,
    mode: GridMode,
    action: Action,
    reply: Arc<OnceCell<Reply>>,
}
impl MarketCommands {
    pub(in crate::commands) async fn browse(&self, selection: MarketSelection) -> Reply {
        match self.market_page(selection, GridMode::Linked).await {
            Ok(view) => self.render_market(&view).await,
            Err(error) => render::error_reply(error),
        }
    }
    async fn market_page(
        &self,
        selection: MarketSelection,
        mode: GridMode,
    ) -> Result<View, String> {
        let filters = self
            .market
            .resolve_filters(selection.filters.clone())
            .await
            .map_err(|e| e.to_string())?;
        let mut query = ResolvedGiftOrderQuery::new(filters);
        let source = selection.order_query();
        query.source = source.source;
        query.sort = source.sort;
        query.bid_match = source.bid_match;
        query.period = source.period;
        query.limit = PAGE_SIZE;
        query.view = ReadView::Snapshot { id: None };
        let read = self
            .market
            .query_orders(market_actors::input::query(query.clone()))
            .await
            .map_err(|e| e.to_string())?;
        let id = read
            .snapshot
            .ok_or("Market page unavailable. Please retry.")?;
        let lease = Arc::new(Snapshot {
            id: id.clone(),
            market: self.market.clone(),
            runtime: tokio::runtime::Handle::current(),
        });
        query.view = ReadView::Snapshot { id: Some(id) };
        let catalog = Arc::new(
            self.market
                .catalog_snapshot()
                .await
                .map_err(|e| e.to_string())?
                .data,
        );
        let documents = Arc::new(
            self.order_documents(&read.result.data, &Default::default())
                .await,
        );
        Ok(View {
            selection: Arc::new(selection),
            query,
            // A complete one-page result never needs another snapshot read.
            _lease: read.result.range.end.as_ref().map(|_| lease),
            pages: vec![Arc::new(read.result)],
            page: 0,
            catalog,
            documents,
            mode,
            action: Action::Refresh,
            reply: Default::default(),
        })
    }
    pub(in crate::commands) async fn market_view(&self, id: u64) -> Reply {
        let Some(view) = self.markets.get(id) else {
            return Reply::notice(
                "This market view has expired. Send /market with the same filters again.",
            );
        };
        view.reply
            .get_or_init(|| self.navigate_market(&view))
            .await
            .clone()
    }
    async fn navigate_market(&self, original: &View) -> Reply {
        if matches!(original.action, Action::Refresh) {
            return match self
                .market_page((*original.selection).clone(), GridMode::Linked)
                .await
            {
                Ok(view) => self.render_market(&view).await,
                Err(_) => Reply::notice("Market refresh unavailable. Please retry."),
            };
        }
        let mut view = original.clone();
        if !matches!(original.action, Action::Mode(GridMode::Gallery)) {
            view.mode = GridMode::Linked;
        }
        match original.action {
            Action::Mode(mode) => view.mode = mode,
            Action::Previous => view.page = view.page.saturating_sub(1),
            Action::Next => {
                if view.page + 1 >= MAX_PAGES {
                    return Reply::notice("Open the market app to browse further.");
                }
                if view.page + 1 == view.pages.len() {
                    let Some(key) = view.pages[view.page]
                        .range
                        .end
                        .as_ref()
                        .and_then(|key| key.encode().try_into().ok())
                    else {
                        return Reply::notice("No more results.");
                    };
                    let mut query = view.query.clone();
                    query.range.start = Some(key);
                    match self
                        .market
                        .query_orders(market_actors::input::query(query))
                        .await
                    {
                        Ok(read) => view.pages.push(Arc::new(read.result)),
                        Err(_) => {
                            return Reply::notice(
                                "This market page has expired or is unavailable. Refresh the report to continue.",
                            );
                        }
                    }
                    let missing = self
                        .order_documents(&view.pages[view.page + 1].data, &view.documents)
                        .await;
                    if !missing.is_empty() {
                        Arc::make_mut(&mut view.documents).extend(missing);
                    }
                }
                view.page += 1;
            }
            Action::Refresh => unreachable!(),
        }
        self.render_market(&view).await
    }
    fn market_button(
        &self,
        view: &View,
        action: Option<Action>,
        label: &str,
        selected: bool,
    ) -> String {
        let data = action.map(|action| {
            self.markets.insert(View {
                action,
                reply: Default::default(),
                ..view.clone()
            })
        });
        rich::button(
            label,
            data.as_ref()
                .map(|data| std::str::from_utf8(data).expect("ASCII token")),
            selected,
        )
    }
    async fn render_market(&self, view: &View) -> Reply {
        let result = &view.pages[view.page];
        let heading = match view.query.source {
            OrderSource::Asks => "Best asks",
            OrderSource::Bids => "Best bids",
            OrderSource::History => "Recent sales",
        };
        let collection = view
            .query
            .filters
            .collection_id
            .as_ref()
            .and_then(|id| view.catalog.collections.iter().find(|c| &c.id == id));
        let title =
            collection.map_or_else(|| heading.to_owned(), |c| format!("{heading} · {}", c.name));
        let mut reply = Reply {
            text: title.clone(),
            title: Some(title.clone()),
            rich_html: Some(format!("<h3>{}</h3>", report::escape(&title))),
            ..Default::default()
        };
        let html = reply.rich_html.as_mut().unwrap();
        html.push_str("<tg-button-row>");
        for (mode, label) in [
            (
                GridMode::Linked,
                if view.mode == GridMode::Gallery {
                    "Browse"
                } else {
                    "Linked"
                },
            ),
            (GridMode::Gallery, "Gallery"),
        ] {
            html.push_str(&self.market_button(
                view,
                (mode != view.mode).then_some(Action::Mode(mode)),
                label,
                mode == view.mode,
            ));
        }
        html.push_str("</tg-button-row>");
        if result.data.is_empty() {
            html.push_str("<p>No matching orders.</p>");
            reply.text.push_str("\nNo matching orders.");
        } else {
            orders::render(
                &mut reply,
                result.data.iter().map(|row| &row.order),
                &self.icons,
                &view.catalog.collections,
                &view.documents,
                None,
            );
            if view.mode == GridMode::Gallery {
                reply.artwork = Some(
                    self.order_gallery(&result.data, &view.documents, None, 3)
                        .await,
                );
            }
        }
        let html = reply.rich_html.as_mut().unwrap();
        if view.query.source == OrderSource::History {
            html.push_str("<footer>UTC · Observed sales</footer>");
        }
        if view.mode == GridMode::Linked {
            html.push_str("<tg-button-row>");
            html.push_str(&self.market_button(
                view,
                (view.page > 0).then_some(Action::Previous),
                "←",
                false,
            ));
            html.push_str(&rich::button(&(view.page + 1).to_string(), None, false));
            html.push_str(&self.market_button(
                view,
                (result.range.end.is_some() && view.page + 1 < MAX_PAGES).then_some(Action::Next),
                "→",
                false,
            ));
            html.push_str(&self.market_button(view, Some(Action::Refresh), "↻", false));
            html.push_str("</tg-button-row>");
        }
        if let Ok(button) = super::market_button(&view.selection, self.public_url.as_ref()) {
            reply.buttons = vec![vec![button]];
        }
        reply
    }
}
