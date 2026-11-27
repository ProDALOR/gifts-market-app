//! One cached immutable report view, with creator ownership and typed destinations in buttons.
use super::{cache::Cache, history::Scope, views::Tab, *};
use market_telegram::{CallbackTarget, Event, ReplyButton};
use std::{collections::VecDeque, sync::Arc};

mod callback;
pub(super) use callback::ReportAction;
pub use callback::ReportCallback;
impl ReportAction {
    fn portfolio_request(self, state: &State, inline: bool) -> Option<PortfolioRequest> {
        let (
            State::Portfolio { user, currency },
            Self::Portfolio {
                page,
                sort,
                layout,
                mut action,
            },
        ) = (state, self)
        else {
            return None;
        };
        if inline && action == PortfolioAction::Refresh {
            action = PortfolioAction::Navigate;
        }
        Some(PortfolioRequest {
            user_id: *user,
            currency_id: *currency,
            page,
            sort,
            layout,
            action,
            mode: if inline {
                PortfolioMode::Inline
            } else {
                PortfolioMode::Private
            },
        })
    }
}
#[derive(Clone)]
pub(super) enum State {
    Gift(views::View),
    Market(browse::View),
    Portfolio {
        user: market_portfolio::UserId,
        currency: BotCurrency,
    },
}
impl State {
    fn accepts(&self, action: ReportAction) -> bool {
        matches!(
            (self, action),
            (Self::Gift(_), ReportAction::Gift(_))
                | (Self::Market(_), ReportAction::Market(_))
                | (Self::Portfolio { .. }, ReportAction::Portfolio { .. })
        )
    }
}
type ReplySlot = Arc<tokio::sync::OnceCell<Reply>>;
#[derive(Default)]
struct Responses(tokio::sync::Mutex<VecDeque<(ReportAction, ReplySlot)>>);
impl Responses {
    async fn slot(&self, action: ReportAction) -> ReplySlot {
        let mut entries = self.0.lock().await;
        let slot = entries
            .iter()
            .position(|(a, _)| *a == action)
            .and_then(|i| entries.remove(i))
            .map(|(_, s)| s)
            .unwrap_or_default();
        entries.push_back((action, slot.clone()));
        while entries.len() > 4 {
            entries.pop_front();
        }
        slot
    }
}
#[derive(Clone)]
struct OwnedReport {
    creator: Option<u64>,
    state: Arc<State>,
    responses: Arc<Responses>,
}
pub(super) struct Reports(Cache<OwnedReport>);
impl Default for Reports {
    fn default() -> Self {
        Self(Cache::new("r", 320))
    }
}
fn token_id(token: &[u8]) -> u64 {
    interactions::callback_id(std::str::from_utf8(&token[2..]).expect("ASCII token"))
        .expect("report ID")
}
impl Reports {
    fn insert(&self, state: State) -> u64 {
        token_id(&self.0.insert(OwnedReport {
            creator: None,
            state: Arc::new(state),
            responses: Default::default(),
        }))
    }
    pub fn gift(&self, view: views::View) -> u64 {
        self.insert(State::Gift(view))
    }
    pub fn market(&self, view: browse::View) -> u64 {
        self.insert(State::Market(view))
    }
    #[cfg(test)]
    pub fn gift_view(&self, callback: ReportCallback) -> Option<views::View> {
        match self.0.get(callback.id)?.state.as_ref() {
            State::Gift(view) => Some(view.clone()),
            _ => None,
        }
    }
    fn bind(&self, creator: u64, data: &str) -> Option<String> {
        let (report, action) = if let Ok(callback) = data.parse::<ReportCallback>() {
            let (report, same_creator) = self.0.update(callback.id, |report| {
                if !report.state.accepts(callback.action) {
                    return None;
                }
                if report.creator.is_none() {
                    report.creator = Some(creator);
                }
                Some((report.clone(), report.creator == Some(creator)))
            })??;
            if same_creator {
                return Some(callback.to_string());
            }
            (report, callback.action)
        } else {
            // Legacy private portfolio markup already carries an absolute destination.
            let request = portfolio::callback(data)?;
            let state = State::Portfolio {
                user: request.user_id,
                currency: request.currency_id,
            };
            (
                OwnedReport {
                    creator: None,
                    state: Arc::new(state),
                    responses: Default::default(),
                },
                ReportAction::Portfolio {
                    page: request.page,
                    sort: request.sort,
                    layout: request.layout,
                    action: request.action,
                },
            )
        };
        let (token, _) = self.0.find_or_insert(
            |existing| existing.creator == Some(creator) && (Arc::ptr_eq(&existing.state, &report.state) || matches!((existing.state.as_ref(), report.state.as_ref()), (State::Portfolio { user: a, currency: b }, State::Portfolio { user: c, currency: d }) if a == c && b == d)),
            || OwnedReport { creator: Some(creator), ..report.clone() },
        );
        Some(ReportCallback::new(token_id(&token), action).to_string())
    }
    pub fn reply(&self, creator: u64, mut reply: Reply) -> Reply {
        for button in reply.buttons.iter_mut().flatten() {
            if let ReplyButton::Callback { data, .. } = button
                && let Some(bound) = std::str::from_utf8(data)
                    .ok()
                    .and_then(|data| self.bind(creator, data))
            {
                *data = bound.into_bytes();
            }
        }
        if let Some(html) = &mut reply.rich_html {
            const PREFIX: &str = super::rich::CALLBACK_PREFIX;
            let mut remaining = html.as_str();
            let mut rendered = String::with_capacity(html.len());
            while let Some(start) = remaining.find(PREFIX) {
                let end_prefix = start + PREFIX.len();
                rendered.push_str(&remaining[..end_prefix]);
                remaining = &remaining[end_prefix..];
                let Some(end) = remaining.find('"') else {
                    break;
                };
                let original = &remaining[..end];
                rendered.push_str(
                    &self
                        .bind(creator, original)
                        .unwrap_or_else(|| original.to_owned()),
                );
                remaining = &remaining[end..];
            }
            rendered.push_str(remaining);
            *html = rendered;
        }
        reply
    }
    pub fn parse(&self, event: &Event, usernames: &[String]) -> Option<CommandRequest> {
        let Event::Callback {
            user_id,
            data,
            target,
            ..
        } = event
        else {
            return interactions::parse_event(event, usernames);
        };
        let private = matches!(target, CallbackTarget::Message(message) if market_telegram::transport::incoming(message).user_id == Some(*user_id));
        if private && data.starts_with(b"p:") {
            return interactions::parse_event(event, usernames);
        }
        let command = match std::str::from_utf8(data)
            .ok()
            .and_then(|data| data.parse::<ReportCallback>().ok())
            .and_then(|callback| self.0.get(callback.id).map(|report| (callback, report)))
        {
            Some((callback, report))
                if report.creator == Some(*user_id) && report.state.accepts(callback.action) =>
            {
                callback
                    .action
                    .portfolio_request(&report.state, !private)
                    .map(Command::Portfolio)
                    .unwrap_or(Command::Report(callback))
            }
            Some((_, report)) if report.creator.is_some_and(|creator| creator != *user_id) => {
                Command::CallbackDenied
            }
            _ => Command::CallbackExpired,
        };
        Some(CommandRequest {
            command,
            user_id: *user_id,
            message_id: 0,
            inline: !private,
        })
    }
}
impl MarketCommands {
    pub(super) async fn report_request(
        &self,
        callback: ReportCallback,
        creator: u64,
        inline: bool,
    ) -> Reply {
        let Some(report) = self.reports.0.get(callback.id) else {
            return Reply::notice("These controls have expired. Request a fresh report.");
        };
        if report.creator != Some(creator) {
            return Reply::notice("Only the person who created this report can use its controls.");
        }
        self.read_report(callback, report, inline).await
    }
    #[cfg(test)]
    pub(super) async fn report(&self, callback: ReportCallback) -> Reply {
        let Some(report) = self.reports.0.get(callback.id) else {
            return Reply::notice("These controls have expired. Request a fresh report.");
        };
        self.read_report(callback, report, false).await
    }
    async fn read_report(
        &self,
        callback: ReportCallback,
        report: OwnedReport,
        inline: bool,
    ) -> Reply {
        if let Some(query) = callback.action.portfolio_request(&report.state, inline) {
            // Inventory and valuation caches own freshness. Do not freeze a portfolio render here.
            return self.portfolio_reply(query, None).await;
        }
        if !report.state.accepts(callback.action) {
            return Reply::notice("These controls are unavailable. Request a fresh report.");
        }
        // Only requested destinations allocate a result slot, never every displayed button.
        let slot = report.responses.slot(callback.action).await;
        let result = slot
            .get_or_try_init(|| async {
                let reply = match (report.state.as_ref(), callback.action) {
                    (State::Gift(view), ReportAction::Gift(destination)) => {
                        self.navigate_gift(view, destination).await
                    }
                    (State::Market(view), ReportAction::Market(destination)) => {
                        self.navigate_market(view, destination).await
                    }
                    _ => Reply::notice("These controls are unavailable. Request a fresh report."),
                };
                if reply.kind == market_telegram::ReplyKind::Notice {
                    Err(reply)
                } else {
                    Ok(reply)
                }
            })
            .await;
        match result {
            Ok(reply) => reply.clone(),
            Err(reply) => reply,
        }
    }
}

#[cfg(test)]
mod tests;
