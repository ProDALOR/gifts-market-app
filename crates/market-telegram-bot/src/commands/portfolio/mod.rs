use super::*;
use market_portfolio::{
    PortfolioQuery, PortfolioReadCache, PortfolioSort, PortfolioStage, PortfolioState,
    ResolvedPortfolioReadQuery, UserId,
};
use market_telegram::ReplyButton;
use std::time::Duration;
const PAGE_SIZE: usize = 30;
mod render;
mod request;
use render::render;
pub use request::{PortfolioAction, PortfolioMode, PortfolioRequest};
pub(super) use request::{callback, inline, parse_args, parse_sort, sort_code};
impl MarketCommands {
    pub(super) async fn portfolio_target(&self, target: sharing::PortfolioTarget) -> Reply {
        match self.load_portfolio_target(target).await {
            Ok(reply) => reply,
            Err(error) => Reply::text(error.to_string()),
        }
    }
    pub(super) async fn load_portfolio_target(
        &self,
        mut target: sharing::PortfolioTarget,
    ) -> Result<Reply, sharing::ShareError> {
        use sharing::ShareError;
        target
            .validate()
            .map_err(|e| ShareError::Invalid(e.to_string()))?;
        target.currency_id = BotCurrency::parse(target.currency_id.as_str())
            .ok_or_else(|| ShareError::Invalid("Unsupported currency".into()))?
            .id();
        let fallback = sharing::portfolio_fallback(&target, self.public_url.as_ref())?;
        let input = market_actors::input::GiftFilters {
            collection_id: target.filters.collection_id,
            currency_id: target.currency_id.clone(),
            models: target.filters.models,
            backdrops: target.filters.backdrops,
            patterns: target.filters.patterns,
            venue_id: None,
            price: None,
            number: None,
        };
        let filters = if input.has_aliases() {
            self.market
                .resolve_filters(input)
                .await
                .map_err(|e| ShareError::Unavailable(e.to_string()))?
        } else {
            input
                .into_resolved()
                .map_err(|e| ShareError::Invalid(e.to_string()))?
        };
        let query = market_portfolio::ResolvedPortfolioTarget {
            user_id: target.user_id,
            currency_id: target.currency_id,
            sort: target.sort,
            filters: market_portfolio::ResolvedPortfolioFilters {
                collection_id: filters.collection_id,
                models: filters.models,
                backdrops: filters.backdrops,
                patterns: filters.patterns,
                number: target.filters.number,
            },
        }
        .into_query();
        self.shared_portfolio(query)
            .await
            .map(|reply| sharing::with_share(reply, &fallback))
    }
    pub(super) async fn shared_portfolio(
        &self,
        query: ResolvedPortfolioReadQuery,
    ) -> Result<Reply, super::sharing::ShareError> {
        use super::sharing::ShareError;
        let service = self
            .portfolio
            .as_ref()
            .ok_or_else(|| ShareError::Unavailable("Portfolios are unavailable".into()))?;
        let state = service
            .cached(query.subject())
            .await
            .map_err(|e| ShareError::Unavailable(e.to_string()))?;
        if state.valuation.is_none() {
            return Err(ShareError::Unavailable(
                "Check this portfolio before sharing it".into(),
            ));
        }
        let request = PortfolioRequest {
            user_id: query.user_id,
            currency_id: BotCurrency::parse(query.currency_id.as_str())
                .ok_or_else(|| ShareError::Invalid("Unsupported currency".into()))?,
            sort: query.sort,
            page: 0,
            action: PortfolioAction::Navigate,
            mode: PortfolioMode::Inline,
            layout: GridMode::Linked,
        };
        let app_link =
            super::app_links::portfolio_button(&request, &query.filters, self.public_url.as_ref())
                .map_err(|e| ShareError::Invalid(e.to_string()))?;
        let (reads, icons) = (self.portfolio_reads.clone(), self.icons.clone());
        let stop = tokio_util::sync::CancellationToken::new();
        let _cancel = stop.clone().drop_guard();
        tokio::task::spawn_blocking(move || {
            let offset = query.offset;
            let page = reads
                .read(&state, query, || {
                    if stop.is_cancelled() {
                        Err(market_portfolio::PortfolioError::Stopped)
                    } else {
                        Ok(())
                    }
                })
                .map_err(|e| ShareError::Unavailable(e.to_string()))?;
            if offset > 0 && offset >= page.selection.count {
                return Err(ShareError::Invalid(
                    "No gifts at this position; reload the portfolio page".into(),
                ));
            }
            let mut reply = render::render_page(
                &state,
                &request,
                &icons,
                &reads,
                &stop,
                Some((page, offset)),
            );
            for row in &mut reply.buttons {
                for button in row {
                    if matches!(button, ReplyButton::Preview { .. }) {
                        *button = app_link.clone();
                    }
                }
            }
            Ok(reply)
        })
        .await
        .map_err(|_| ShareError::Unavailable("Portfolio could not be displayed".into()))?
    }

    pub(super) async fn portfolio_reply(
        &self,
        mut request: PortfolioRequest,
        updates: Option<tokio::sync::watch::Sender<Reply>>,
    ) -> Reply {
        // Legacy gallery paging/refresh callbacks move back to browsing without rendering a new PNG.
        if request.layout == GridMode::Gallery
            && matches!(
                request.action,
                PortfolioAction::Navigate | PortfolioAction::Refresh
            )
        {
            request.page = request.page.saturating_mul(request.page_size()) / PAGE_SIZE;
            request.layout = GridMode::Linked;
        }
        let Some(service) = &self.portfolio else {
            return recovery(
                &request,
                Some("Telegram portfolios are not configured.".into()),
            );
        };
        let query = PortfolioQuery {
            user_id: request.user_id,
            currency_id: request.currency_id.id(),
        };
        if request.mode == PortfolioMode::Inline
            || matches!(
                request.action,
                PortfolioAction::Navigate | PortfolioAction::Preview
            )
        {
            return match service.cached(query).await {
                Ok(state)
                    if state.valuation.is_some()
                        || (request.action == PortfolioAction::Open
                            && state.progress.stage != PortfolioStage::Idle) =>
                {
                    self.portfolio_render(state, &request).await
                }
                Ok(_) | Err(market_portfolio::PortfolioError::NotChecked) => {
                    recovery(&request, None)
                }
                Err(error) => recovery(&request, Some(error.to_string())),
            };
        }
        if request.action == PortfolioAction::Refresh
            && let Err(error) = service.refresh(query.clone()).await
        {
            return recovery(&request, Some(error.to_string()));
        }
        let mut watch = match service.watch(query).await {
            Ok(w) => w,
            Err(error) => return recovery(&request, Some(error.to_string())),
        };
        let mut state = watch.current();
        let wait = async {
            loop {
                let reply = self.portfolio_render(state.clone(), &request).await;
                if matches!(
                    state.progress.stage,
                    PortfolioStage::Ready | PortfolioStage::Failed
                ) {
                    return reply;
                }
                if let Some(updates) = &updates {
                    updates.send_replace(reply);
                }
                state = match watch.changed().await {
                    Ok(state) => state,
                    Err(e) => return Reply::text(e.to_string()),
                };
            }
        };
        match tokio::time::timeout(Duration::from_secs(80), wait).await {
            Ok(reply) => reply,
            Err(_) => {
                let mut reply = self.portfolio_render(watch.current(), &request).await;
                reply
                    .text
                    .push_str("\nOpen portfolio to keep following progress.");
                if let Some(html) = &mut reply.rich_html {
                    html.push_str("<p>Open portfolio to keep following progress.</p>");
                }
                reply
            }
        }
    }
    async fn portfolio_render(
        &self,
        state: std::sync::Arc<PortfolioState>,
        request: &PortfolioRequest,
    ) -> Reply {
        let (icons, reads, request) = (
            self.icons.clone(),
            self.portfolio_reads.clone(),
            request.clone(),
        );
        let retained = state.clone();
        let spec = request.clone();
        let stop = tokio_util::sync::CancellationToken::new();
        let _cancel = stop.clone().drop_guard();
        let mut reply =
            tokio::task::spawn_blocking(move || render(&state, &request, &icons, &reads, &stop))
                .await
                .unwrap_or_else(|_| {
                    Reply::text("Portfolio could not be displayed. Please try again.")
                });
        if spec.layout == GridMode::Gallery
            && let Some(report) = &retained.valuation
        {
            let count = report.items.len();
            if count > 0 {
                let query = ResolvedPortfolioReadQuery {
                    user_id: spec.user_id,
                    currency_id: spec.currency_id.id(),
                    filters: Default::default(),
                    sort: spec.sort,
                    offset: spec.page.min(count.div_ceil(spec.page_size()).max(1) - 1)
                        * spec.page_size(),
                    limit: spec.page_size(),
                    version: retained.version.clone(),
                };
                let reads = self.portfolio_reads.clone();
                let stop = tokio_util::sync::CancellationToken::new();
                let _cancel = stop.clone().drop_guard();
                if let Ok(Ok(page)) = tokio::task::spawn_blocking(move || {
                    reads.read(&retained, query, || {
                        if stop.is_cancelled() {
                            Err(market_portfolio::PortfolioError::Stopped)
                        } else {
                            Ok(())
                        }
                    })
                })
                .await
                {
                    reply.artwork =
                        Some(self.portfolio_gallery(&page.items, spec.currency_id).await);
                }
            }
        }
        reply
    }
}

fn recovery(request: &PortfolioRequest, error: Option<String>) -> Reply {
    let command = request.check_command();
    let missing = error.is_none();
    let text = error
        .unwrap_or_else(|| format!("Open the bot and send {command} to check this portfolio."));
    if request.action != PortfolioAction::Open {
        Reply::notice(text)
    } else {
        Reply {
            title: Some(
                if missing {
                    "Portfolio · check required"
                } else {
                    "Portfolio unavailable"
                }
                .into(),
            ),
            summary: Some(if missing {
                format!("Check privately: {command}")
            } else {
                text.clone()
            }),
            ..Reply::text(text)
        }
    }
}

#[cfg(test)]
mod tests;
