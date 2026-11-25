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
pub(super) use request::{callback, inline, parse_args};
impl MarketCommands {
    pub(super) async fn portfolio_reply(
        &self,
        request: PortfolioRequest,
        updates: Option<tokio::sync::watch::Sender<Reply>>,
    ) -> Reply {
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
        if request.mode == PortfolioMode::Inline || request.action == PortfolioAction::Navigate {
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
        let stop = tokio_util::sync::CancellationToken::new();
        let _cancel = stop.clone().drop_guard();
        tokio::task::spawn_blocking(move || render(&state, &request, &icons, &reads, &stop))
            .await
            .unwrap_or_else(|_| Reply::text("Portfolio could not be displayed. Please try again."))
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
