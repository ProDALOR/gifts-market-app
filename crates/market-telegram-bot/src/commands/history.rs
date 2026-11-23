//! Bounded comparable executions in a collapsed section of the gift report. This is an order read, not another valuation strategy.
use super::*;
use super::{
    interactions::View,
    report::{Report, amount, escape},
};
use market_gifts::*;
use std::sync::Arc;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Scope {
    Pair,
    Model,
    Backdrop,
    Collection,
}
impl Scope {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pair => "Same model + backdrop",
            Self::Model => "Same model",
            Self::Backdrop => "Same backdrop",
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
#[derive(Clone)]
pub(super) struct HistoryRequest {
    pub scope: Scope,
    pub start: Option<OrderKey>,
    pub until: jiff::Timestamp,
    pub page: u8,
}
impl HistoryRequest {
    pub fn initial(report: &Report) -> Option<Self> {
        let gift = report.gift()?;
        let scope = [
            Scope::Pair,
            Scope::Model,
            Scope::Backdrop,
            Scope::Collection,
        ]
        .into_iter()
        .find(|scope| scope.supported(gift))?;
        Some(Self {
            scope,
            start: None,
            until: jiff::Timestamp::now(),
            page: 1,
        })
    }
    pub fn query(
        &self,
        gift: &CollectibleGift,
        currency: CurrencyId,
    ) -> Result<GiftOrderQuery, BotError> {
        if !self.scope.supported(gift) || self.page > 10 {
            return Err(BotError::InvalidResponse);
        }
        let mut filters = GiftFilters::new(gift.collection_id.clone(), currency);
        if matches!(self.scope, Scope::Pair | Scope::Model) {
            filters.models = gift.traits.model.clone().into_iter().collect();
        }
        if matches!(self.scope, Scope::Pair | Scope::Backdrop) {
            filters.backdrops = gift.traits.backdrop.clone().into_iter().collect();
        }
        let mut query = GiftOrderQuery::new(filters);
        query.source = OrderSource::History;
        query.sort = Some(GiftSort::TimeDesc);
        query.limit = 5;
        query.period = Some(Period {
            from: Some(
                self.until
                    .checked_sub(jiff::SignedDuration::from_secs(30 * 86400))
                    .map_err(|_| BotError::InvalidResponse)?,
            ),
            until: Some(self.until),
        });
        query.range.start = self.start.clone();
        Ok(query)
    }
}
impl MarketCommands {
    pub(super) async fn recent_sales(&self, report: Arc<Report>) -> Reply {
        let Some(request) = HistoryRequest::initial(&report) else {
            return report.render();
        };
        self.history(report, request).await
    }
    pub(super) async fn history(&self, report: Arc<Report>, request: HistoryRequest) -> Reply {
        let Some(gift) = report.gift() else {
            return Reply::text("Gift identity is unavailable.");
        };
        let query = match request.query(gift, report.query.currency_id.clone()) {
            Ok(query) => query,
            Err(_) => return Reply::text("Request a fresh valuation to browse sales."),
        };
        // History is optional decoration; a slow read must not suppress the estimate.
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            self.market.query_orders(query),
        )
        .await;
        render(
            &self.views,
            report,
            request,
            result.ok().and_then(Result::ok).map(|read| read.result),
        )
    }
}
impl super::interactions::Views {
    fn history_button(&self, report: Arc<Report>, history: HistoryRequest, label: &str) -> String {
        let data = self.insert(View { report, history });
        let data = std::str::from_utf8(&data).expect("ASCII callback token");
        format!(
            "<tg-button type=\"callback_data\" style=\"link\" data=\"{}\">{}</tg-button>",
            escape(data),
            escape(label)
        )
    }
}
pub(super) fn render(
    views: &super::interactions::Views,
    report: Arc<Report>,
    request: HistoryRequest,
    result: Option<market_actors::OrderResult>,
) -> Reply {
    let gift = report.gift().expect("resolved gift report");
    let mut reply = report.render();
    let mut html = reply.rich_html.take().unwrap_or_default();
    html.push_str(&format!(
        "<details><summary>{} Recent sales</summary>",
        report.icons.html(crate::ReportIcon::History)
    ));
    let result = match result {
        Some(result) => result,
        _ => {
            html.push_str("<p>Sales are temporarily unavailable.</p>");
            html.push_str(&views.history_button(report, request, "Retry sales"));
            html.push_str("</details>");
            reply.rich_html = Some(html);
            reply
                .text
                .push_str("\n\nRecent sales are temporarily unavailable.");
            return reply;
        }
    };
    html.push_str(&format!(
        "<p>{} · last 30 days · page {}</p>",
        escape(request.scope.label()),
        request.page
    ));
    let mut text = format!(
        "{}\n\nRecent sales · {} · last 30 days\n",
        reply.text,
        request.scope.label()
    );
    if result.data.is_empty() {
        html.push_str("<p>No observed executions in this period.</p>");
        text.push_str("No observed executions in this period.\n");
    } else {
        html.push_str("<table compact><tr><th>Gift</th><th align=\"right\">Paid</th><th>Executed</th><th>Venue</th></tr>");
        for row in &result.data {
            let order = &row.order;
            let Some(time) = order.executed_at else {
                continue;
            };
            let number = match &order.target {
                GiftTarget::Gift { gift } => gift.number,
                GiftTarget::Criteria { criteria } => criteria
                    .number
                    .as_ref()
                    .and_then(|range| range.min.filter(|v| Some(*v) == range.max)),
                _ => None,
            };
            let label = number
                .map(|n| format!("#{n}"))
                .unwrap_or_else(|| "Gift".into());
            let link = number.and_then(|number| {
                report
                    .catalog
                    .as_ref()
                    .and_then(|catalog| catalog.collection.slug.as_ref())
                    .map(|slug| format!("https://t.me/nft/{slug}-{number}"))
            });
            let label_html = link
                .map(|url| format!("<a href=\"{}\">{}</a>", escape(&url), escape(&label)))
                .unwrap_or_else(|| escape(&label));
            html.push_str(&format!(
                "<tr><td>{label_html}</td><td align=\"right\">{} {}</td><td>{}</td><td>{}</td></tr>",
                escape(&amount(&order.price)),
                escape(order.book.currency_id.as_str()),
                time.strftime("%d %b %H:%M UTC"),
                escape(order.book.venue_id.as_str())
            ));
            text.push_str(&format!(
                "{label} · {} {} · {} · {}\n",
                amount(&order.price),
                order.book.currency_id,
                time.strftime("%d %b %H:%M UTC"),
                order.book.venue_id
            ));
        }
        html.push_str("</table>");
    }
    html.push_str("<p><i>Observed executions only. Market history may be incomplete.</i></p>");
    let scopes = [
        Scope::Pair,
        Scope::Model,
        Scope::Backdrop,
        Scope::Collection,
    ]
    .into_iter()
    .filter(|scope| scope.supported(gift))
    .collect::<Vec<_>>();
    for row in scopes.chunks(2) {
        html.push_str("<tg-button-row>");
        for scope in row {
            let label = format!(
                "{}{}",
                if *scope == request.scope { "✓ " } else { "" },
                scope.label()
            );
            html.push_str(&views.history_button(
                report.clone(),
                HistoryRequest {
                    scope: *scope,
                    start: None,
                    until: request.until,
                    page: 1,
                },
                &label,
            ));
        }
        html.push_str("</tg-button-row>");
    }
    if let Some(next) = result.range.end
        && request.page < 10
        && let Ok(start) = next.encode().try_into()
    {
        html.push_str(&views.history_button(
            report,
            HistoryRequest {
                start: Some(start),
                page: request.page + 1,
                ..request
            },
            "More sales",
        ));
    }
    html.push_str("</details>");
    reply.rich_html = Some(html);
    reply.text = text;
    reply
}
