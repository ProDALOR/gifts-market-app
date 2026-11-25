use super::*;
use market_portfolio::{
    PortfolioQuery, PortfolioReadCache, PortfolioSort, PortfolioStage, PortfolioState,
    ResolvedPortfolioReadQuery, UserId,
};
use market_telegram::ReplyButton;
use std::time::Duration;
const PAGE_SIZE: usize = 30;
const COLUMNS: usize = 3;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PortfolioMode {
    #[default]
    Private,
    Inline,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortfolioRequest {
    pub user_id: u64,
    pub currency_id: market_gifts::CurrencyId,
    pub refresh: bool,
    pub page: usize,
    pub sort: PortfolioSort,
    pub mode: PortfolioMode,
}
const SORTS: [(PortfolioSort, &str, &str); 6] = [
    (PortfolioSort::PositionAsc, "position_asc", "pa"),
    (PortfolioSort::PositionDesc, "position_desc", "pd"),
    (PortfolioSort::NumberAsc, "number_asc", "na"),
    (PortfolioSort::NumberDesc, "number_desc", "nd"),
    (PortfolioSort::ValueAsc, "price_asc", "va"),
    (PortfolioSort::ValueDesc, "price_desc", "vd"),
];
fn parse_sort(value: &str) -> Option<PortfolioSort> {
    SORTS
        .iter()
        .find(|(_, name, code)| *name == value || *code == value)
        .map(|(sort, _, _)| *sort)
}
impl PortfolioRequest {
    fn data(&self, page: usize, refresh: bool, sort: PortfolioSort) -> String {
        let code = SORTS
            .iter()
            .find(|(candidate, _, _)| *candidate == sort)
            .unwrap()
            .2;
        format!(
            "p:{}:{}:{}:{page}:{code}",
            self.user_id,
            self.currency_id,
            if refresh { "r" } else { "v" }
        )
    }
    fn inline_query(&self) -> String {
        let name = SORTS
            .iter()
            .find(|(candidate, _, _)| *candidate == self.sort)
            .unwrap()
            .1;
        format!(
            "portfolio {} {} sort={name}",
            self.user_id, self.currency_id
        )
    }
}
pub(super) fn parse_args(
    text: &str,
    user_id: u64,
    mode: PortfolioMode,
) -> Option<PortfolioRequest> {
    if text.len() > 512 {
        return None;
    }
    let mut request = PortfolioRequest {
        user_id,
        currency_id: "GRAM".into(),
        refresh: false,
        page: 0,
        sort: PortfolioSort::PositionAsc,
        mode,
    };
    let (mut user, mut currency, mut sort) = (false, false, false);
    for word in text.split_whitespace() {
        if let Some(value) = word.strip_prefix("sort=") {
            if sort {
                return None;
            }
            request.sort = parse_sort(value)?;
            sort = true;
        } else if word.starts_with(|c: char| c.is_ascii_digit() || c == '-' || c == '+') {
            if user {
                return None;
            }
            request.user_id = word.parse().ok()?;
            user = true;
        } else {
            if currency {
                return None;
            }
            request.currency_id = match word {
                "TON" => "GRAM",
                "STARS" => "XTR",
                other => other,
            }
            .to_owned()
            .try_into()
            .ok()?;
            currency = true;
        }
    }
    UserId::new(request.user_id).ok()?;
    Some(request)
}
pub(super) fn inline(text: &str, user: u64) -> Option<PortfolioRequest> {
    let text = text.trim();
    if text.is_empty() {
        return parse_args("", user, PortfolioMode::Inline);
    }
    let (name, args) = text.split_once(char::is_whitespace).unwrap_or((text, ""));
    if !matches!(name, "portfolio" | "/portfolio") {
        return None;
    }
    parse_args(args, user, PortfolioMode::Inline)
}
pub(super) fn callback(text: &str) -> Option<PortfolioRequest> {
    let mut p = text.strip_prefix("p:")?.split(':');
    let user_id = p.next()?.parse().ok()?;
    let currency_id = p.next()?.to_owned().try_into().ok()?;
    let action = p.next()?;
    let page = p.next()?.parse::<usize>().ok()?;
    let sort = p
        .next()
        .map(parse_sort)
        .unwrap_or(Some(PortfolioSort::PositionAsc))?;
    if page > 999 {
        return None;
    }
    if p.next().is_some() || !matches!(action, "r" | "v") {
        return None;
    }
    UserId::new(user_id).ok()?;
    Some(PortfolioRequest {
        user_id,
        currency_id,
        refresh: action == "r",
        page,
        sort,
        mode: PortfolioMode::Private,
    })
}
impl MarketCommands {
    pub(super) async fn portfolio_reply(
        &self,
        request: PortfolioRequest,
        updates: Option<tokio::sync::watch::Sender<Reply>>,
    ) -> Reply {
        let Some(service) = &self.portfolio else {
            return Reply::text("Telegram portfolios are not configured.");
        };
        let Ok(user_id) = UserId::new(request.user_id) else {
            return Reply::text("Invalid Telegram user ID.");
        };
        let query = PortfolioQuery {
            user_id,
            currency_id: request.currency_id.clone(),
        };
        if request.mode == PortfolioMode::Inline {
            return match service.cached(query).await {
                Ok(state)
                    if state.valuation.is_some()
                        || state.progress.stage != PortfolioStage::Idle =>
                {
                    self.portfolio_render(state, &request).await
                }
                Ok(_) | Err(market_portfolio::PortfolioError::NotChecked) => Reply {
                    title: Some("Portfolio · check required".into()),
                    summary: Some("Open the bot and send /portfolio first.".into()),
                    ..Reply::text(
                        "Open the bot and send /portfolio to check gifts first, then share the cached portfolio inline.",
                    )
                },
                Err(error) => Reply::text(error.to_string()),
            };
        }
        let refresh_error = if request.refresh {
            service.refresh(query.clone()).await.err()
        } else {
            None
        };
        let mut watch = match service.watch(query).await {
            Ok(w) => w,
            Err(e) => return Reply::text(e.to_string()),
        };
        let mut state = watch.current();
        if let Some(error) = refresh_error {
            // A cooldown must not replace a usable report with a bare error.
            let mut retained = (*state).clone();
            retained.error = Some(error.to_string());
            retained.progress.stage = PortfolioStage::Failed;
            return self
                .portfolio_render(std::sync::Arc::new(retained), &request)
                .await;
        }
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
fn display_amount(value: &market_gifts::Amount) -> String {
    let value = super::report::amount(value);
    let (integer, fractional) = value.split_once('.').unwrap_or((&value, ""));
    let (sign, digits) = integer
        .strip_prefix('-')
        .map_or(("", integer), |v| ("-", v));
    let mut result = String::with_capacity(value.len() + digits.len() / 3);
    result.push_str(sign);
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            result.push(' ');
        }
        result.push(digit);
    }
    if !fractional.is_empty() {
        result.push('.');
        result.push_str(fractional);
    }
    result
}

fn checked_time(at: jiff::Timestamp, now: jiff::Timestamp) -> String {
    let at = at.to_zoned(jiff::tz::TimeZone::UTC);
    let now = now.to_zoned(jiff::tz::TimeZone::UTC);
    let format = if at.date() == now.date() {
        "%H:%M UTC"
    } else if at.year() == now.year() {
        "%d %b %H:%M UTC"
    } else {
        "%d %b %Y %H:%M UTC"
    };
    at.strftime(format).to_string()
}
fn sort_controls(request: &PortfolioRequest) -> String {
    let mut html = "<tg-button-row>".to_owned();
    for (label, ascending, descending, preferred) in [
        (
            "Position",
            PortfolioSort::PositionAsc,
            PortfolioSort::PositionDesc,
            PortfolioSort::PositionAsc,
        ),
        (
            "Price",
            PortfolioSort::ValueAsc,
            PortfolioSort::ValueDesc,
            PortfolioSort::ValueDesc,
        ),
        (
            "Gift #",
            PortfolioSort::NumberAsc,
            PortfolioSort::NumberDesc,
            PortfolioSort::NumberAsc,
        ),
    ] {
        let selected = request.sort == ascending || request.sort == descending;
        let shown = if selected { request.sort } else { preferred };
        let target = if selected {
            if shown == ascending {
                descending
            } else {
                ascending
            }
        } else {
            preferred
        };
        let style = if selected { " style=\"primary\"" } else { "" };
        let direction = if shown == ascending { "↑" } else { "↓" };
        html.push_str(&format!(
            "<tg-button type=\"callback_data\" data=\"{}\"{style}>{label} {direction}</tg-button>",
            super::report::escape(&request.data(0, false, target))
        ));
    }
    html.push_str("</tg-button-row>");
    html
}

fn render(
    state: &PortfolioState,
    request: &PortfolioRequest,
    icons: &crate::ReportIcons,
    reads: &PortfolioReadCache,
    stop: &tokio_util::sync::CancellationToken,
) -> Reply {
    use super::report::escape;
    let phase = match state.progress.stage {
        PortfolioStage::Idle => "Not checked yet".into(),
        PortfolioStage::Queued => "Waiting to check gifts…".into(),
        PortfolioStage::LoadingInventory => match state.progress.total {
            Some(total) => format!("{} of {total} gifts checked", state.progress.checked),
            None => format!("{} gifts checked", state.progress.checked),
        },
        PortfolioStage::Valuing => format!(
            "{} of {} gifts valued",
            state.progress.valued, state.progress.collectibles
        ),
        PortfolioStage::Ready => "".into(),
        PortfolioStage::Failed => state
            .error
            .clone()
            .unwrap_or_else(|| "Refresh failed".into()),
    };
    let mut text = "Portfolio".to_owned();
    let mut html = format!(
        "<h3>{} <a href=\"tg://user?id={}\">Portfolio</a></h3>",
        icons.html(crate::ReportIcon::Portfolio),
        request.user_id
    );
    if !phase.is_empty() {
        text.push_str(&format!("\n{phase}"));
        html.push_str(&format!("<p>{}</p>", escape(&phase)));
    }
    let mut reply = Reply::default();
    let count = state.valuation.as_ref().map_or(0, |v| v.items.len());
    let pages = count.div_ceil(PAGE_SIZE).max(1);
    let page = request.page.min(pages - 1);
    let start = page * PAGE_SIZE;
    let end = (start + PAGE_SIZE).min(count);
    let items = if count > 0 {
        match reads.read(
            state,
            ResolvedPortfolioReadQuery {
                user_id: state.user_id,
                currency_id: request.currency_id.clone(),
                filters: Default::default(),
                sort: request.sort,
                offset: start,
                limit: PAGE_SIZE,
                version: state.version.clone(),
            },
            || {
                if stop.is_cancelled() {
                    Err(market_portfolio::PortfolioError::Stopped)
                } else {
                    Ok(())
                }
            },
        ) {
            Ok(page) => page.items,
            Err(error) => return Reply::text(error.to_string()),
        }
    } else {
        vec![]
    };
    let mut footer = Vec::new();
    if let Some(report) = &state.valuation {
        let total = report
            .total
            .as_ref()
            .map(display_amount)
            .unwrap_or_else(|| "—".into());
        text.push_str(&format!("\nEstimate {total} {}", report.currency_id));
        reply.summary = Some(format!("{count} gifts · {total} {}", report.currency_id));
        // Native currency icons carry the unit; unfamiliar currencies retain their text.
        let currency = crate::ReportIcons::currency(report.currency_id.as_str())
            .map(|icon| icons.html(icon))
            .unwrap_or_else(|| escape(report.currency_id.as_str()));
        html.push_str(&format!("<h2>{total} {currency}</h2>"));
        if report.unpriced > 0 {
            let warning = format!("{} unpriced", report.unpriced);
            html.push_str(&format!("<p>{warning}</p>"));
            text.push_str(&format!("\n{warning}"));
        }
        if count > 0 {
            html.push_str(&sort_controls(request));
            html.push_str("<table striped compact>");
        }
        for row in items.chunks(COLUMNS) {
            html.push_str("<tr>");
            for item in row {
                let price = item
                    .estimate
                    .as_ref()
                    .map(display_amount)
                    .unwrap_or_else(|| "—".into());
                text.push_str(&format!("\n{} · {price}", item.gift.gift.slug));
                let gift = &item.gift.gift;
                let image = reply.custom_emoji(gift.model);
                html.push_str(&format!(
                    "<td><a href=\"https://t.me/nft/{}\">{image} <b>{price}</b> {currency}</a></td>",
                    gift.slug
                ));
            }
            for _ in row.len()..COLUMNS {
                html.push_str("<td></td>");
            }
            html.push_str("</tr>");
        }
        if count > 0 {
            html.push_str("</table>");
            footer.push(format!("{}–{end} of {count}", start + 1));
        } else {
            footer.push("0 gifts".into());
        }
    }
    if state.valuation.as_ref().is_some_and(|v| v.stale) {
        html.push_str("<p>Market updates delayed</p>");
        text.push_str("\nMarket updates delayed");
    }
    if let Some(inventory) = &state.inventory {
        footer.push(format!(
            "Checked {}",
            checked_time(inventory.checked_at, jiff::Timestamp::now())
        ));
        if !inventory.complete {
            html.push_str("<p>Partial inventory</p>");
            text.push_str("\nPartial inventory");
        }
    }
    if state.progress.stage != PortfolioStage::Failed
        && let Some(error) = &state.error
    {
        html.push_str(&format!("<p>{}</p>", escape(error)));
        text.push_str(&format!("\n{error}"));
    }
    if !footer.is_empty() {
        let footer = footer.join(" · ");
        html.push_str(&format!("<footer>{footer}</footer>"));
        text.push_str(&format!("\n{footer}"));
    }
    let button = |label: &str, page, refresh| ReplyButton::Callback {
        text: label.into(),
        data: request.data(page, refresh, request.sort).into_bytes(),
    };
    if pages > 1 {
        let navigation = |label: &str, target: Option<usize>| match target {
            Some(target) => format!(
                "<tg-button type=\"callback_data\" data=\"{}\">{label}</tg-button>",
                escape(&request.data(target, false, request.sort))
            ),
            None => format!("<tg-button type=\"disabled\">{label}</tg-button>"),
        };
        html.push_str(&format!("<tg-button-row>{}<tg-button type=\"disabled\">{} / {pages}</tg-button>{}</tg-button-row>",
            navigation("←", page.checked_sub(1)), page+1,
            navigation("→", (page+1<pages).then_some(page+1))));
    }
    let mut row = vec![];
    if request.mode == PortfolioMode::Private {
        row.push(button("Refresh gifts", 0, true));
    }
    row.extend([
        ReplyButton::SwitchInline {
            text: "Share".into(),
            query: request.inline_query(),
        },
        ReplyButton::Preview {
            text: "Open portfolio".into(),
            payload: format!("portfolio_{}", request.user_id),
        },
    ]);
    reply.text = text;
    reply.rich_html = Some(html);
    reply.title = Some("Portfolio".into());
    reply.buttons = vec![row];
    reply
}

#[cfg(test)]
mod tests {
    use super::*;
    fn render(
        state: &PortfolioState,
        request: &PortfolioRequest,
        icons: &crate::ReportIcons,
    ) -> Reply {
        super::render(
            state,
            request,
            icons,
            &PortfolioReadCache::default(),
            &tokio_util::sync::CancellationToken::new(),
        )
    }
    fn sample(count: usize) -> PortfolioState {
        use market_gifts::{TelegramDocumentId, TelegramGiftDetails, TelegramGiftId};
        let items = (1..=count)
            .map(|n| market_valuation::InventoryValue {
                gift: market_portfolio::InventoryGift {
                    gift: TelegramGiftDetails {
                        id: n.to_string().try_into().unwrap(),
                        slug: format!("ResistanceDog-{n}").try_into().unwrap(),
                        number: (n as u32).try_into().unwrap(),
                        collection: TelegramGiftId::new(123).unwrap(),
                        model: TelegramDocumentId::new(n as i64 + 1000).unwrap(),
                        backdrop: None,
                        pattern: None,
                    },
                    relation: market_portfolio::GiftRelation::Owned,
                },
                resolved: None,
                estimate: Some("2".into()),
                error: None,
            })
            .collect();
        PortfolioState {
            user_id: UserId::new(123).unwrap(),
            refresh_id: 1,
            progress: market_portfolio::PortfolioProgress {
                stage: PortfolioStage::Ready,
                ..Default::default()
            },
            inventory: None,
            version: Some(format!("{count:032x}").try_into().unwrap()),
            refresh_after: None,
            error: None,
            valuation: Some(std::sync::Arc::new(market_valuation::InventoryValuation {
                currency_id: "GRAM".into(),
                evaluated_at: jiff::Timestamp::now(),
                stale: false,
                total: Some((count * 2).to_string().try_into().unwrap()),
                priced: count,
                unpriced: 0,
                items,
            })),
        }
    }
    #[test]
    fn grids_fill_three_columns_and_clamp_pagination_without_losing_gifts() {
        let icons = crate::ReportIcons::default();
        for count in [0, 1, 29, 30, 31, 61] {
            let state = sample(count);
            for requested in [0, usize::MAX] {
                let request = PortfolioRequest {
                    user_id: 123,
                    currency_id: "GRAM".into(),
                    refresh: false,
                    page: requested,
                    sort: PortfolioSort::PositionAsc,
                    mode: PortfolioMode::Private,
                };
                let reply = render(&state, &request, &icons).bounded();
                let html = reply.rich_html.unwrap();
                let page = requested.min(count.div_ceil(PAGE_SIZE).max(1) - 1);
                let size = (count - page * PAGE_SIZE).min(PAGE_SIZE);
                assert_eq!(html.matches("https://t.me/nft/").count(), size);
                assert_eq!(html.matches("<tr>").count(), size.div_ceil(COLUMNS));
                assert_eq!(
                    html.matches("<td").count(),
                    size.div_ceil(COLUMNS) * COLUMNS
                );
                assert_eq!(reply.emoji_documents.len(), size);
                if size > 0 {
                    assert!(html.contains(&format!(
                        "https://t.me/nft/ResistanceDog-{}",
                        page * PAGE_SIZE + 1
                    )));
                    assert!(html.contains("\"><tg-emoji emoji-id=\""));
                    assert!(html.contains("</tg-emoji> <b>2</b> <tg-emoji"));
                    assert_eq!(html.matches("</a></td>").count(), size);
                    assert!(!html.contains("ResistanceD…"));
                    assert!(!html.contains("<br>"));
                    assert!(!html.contains("<p></p>"));
                } else {
                    assert!(!html.contains("<table"));
                }
                assert!(html.len() < 32768);
            }
        }
    }
    #[test]
    fn compact_reports_keep_owner_access_currency_and_one_count() {
        let icons = crate::ReportIcons::default();
        let reply = render(&sample(31), &callback("p:123:GRAM:v:0").unwrap(), &icons);
        let html = reply.rich_html.unwrap();
        let header = html.split("<tg-button-row>").next().unwrap();
        assert!(header.contains("tg://user?id=123\">Portfolio</a>"));
        assert!(!header.contains("Portfolio · 123"));
        assert!(!header.contains("GRAM"));
        assert!(!header.contains("31 gifts"));
        assert!(!html.contains("0 unpriced"));
        assert_eq!(html.matches("1–30 of 31").count(), 1);
        assert!(reply.text.starts_with("Portfolio\nEstimate 62 GRAM"));

        // Currency comes from the report; preserve unknown units when no icon is available.
        let mut state = sample(0);
        std::sync::Arc::get_mut(state.valuation.as_mut().unwrap())
            .unwrap()
            .currency_id = "USD".into();
        let unknown = render(&state, &callback("p:123:USD:v:0").unwrap(), &icons);
        assert!(unknown.rich_html.unwrap().contains("<h2>0 USD</h2>"));
        assert!(unknown.text.contains("0 gifts"));
    }
    #[test]
    fn warning_details_survive_compaction_and_errors_are_not_duplicated() {
        for stage in [PortfolioStage::Ready, PortfolioStage::Failed] {
            let mut state = sample(1);
            state.progress.stage = stage;
            state.error = Some("Refresh <failed>".into());
            let report = std::sync::Arc::get_mut(state.valuation.as_mut().unwrap()).unwrap();
            report.total = None;
            report.items[0].estimate = None;
            report.priced = 0;
            report.unpriced = 1;
            report.stale = true;
            state.inventory = Some(market_portfolio::InventorySummary {
                checked_at: "2025-11-24T03:06:00Z".parse().unwrap(),
                count: 1,
                scanned: 1,
                regular: 0,
                skipped: 0,
                complete: false,
            });
            let reply = render(
                &state,
                &callback("p:123:GRAM:v:0").unwrap(),
                &crate::ReportIcons::default(),
            );
            let html = reply.rich_html.unwrap();
            for warning in ["1 unpriced", "Partial inventory", "Market updates delayed"] {
                assert!(html.contains(warning));
                assert!(reply.text.contains(warning));
            }
            assert_eq!(html.matches("Refresh &lt;failed&gt;").count(), 1);
            assert_eq!(reply.text.matches("Refresh <failed>").count(), 1);
            assert!(reply.text.contains("Estimate — GRAM"));
            assert_eq!(html.matches("<footer>").count(), 1);
            assert!(html.contains("1–1 of 1 · Checked "));
            assert!(!html.contains("Gifts checked"));
        }
    }
    #[test]
    fn amounts_and_check_times_keep_rounding_and_date_boundaries() {
        for (input, expected) in [
            ("0", "0"),
            ("0.5", "0.5"),
            ("999.995", "1 000"),
            ("1234.50", "1 234.5"),
            ("1234567.126", "1 234 567.13"),
        ] {
            assert_eq!(
                display_amount(&market_gifts::Amount::new(input).unwrap()),
                expected
            );
        }
        let now = "2026-11-25T03:07:00Z".parse().unwrap();
        for (input, expected) in [
            ("2026-11-25T03:06:00Z", "03:06 UTC"),
            ("2026-11-24T23:59:59Z", "24 Nov 23:59 UTC"),
            ("2025-11-25T03:06:00Z", "25 Nov 2025 03:06 UTC"),
        ] {
            assert_eq!(checked_time(input.parse().unwrap(), now), expected);
        }
    }
    #[tokio::test]
    #[ignore = "sends one real portfolio preview; requires explicit bot ID, recipient and captured portfolio"]
    async fn send_live_portfolio_grid_preview() {
        use grammers_session::types::{PeerAuth, PeerId, PeerRef};
        use market_telegram_client::{
            BotCredentials, Connection,
            transport::{Message, TelegramActions, Transport},
        };
        let credentials = BotCredentials::new(
            std::env::var("TELEGRAM_BOT_TOKEN").unwrap(),
            std::env::var("TELEGRAM_API_ID").unwrap().parse().unwrap(),
            std::env::var("TELEGRAM_API_HASH").unwrap(),
        )
        .unwrap();
        let expected = std::env::var("MARKET_TEST_BOT_ID")
            .unwrap()
            .parse::<u64>()
            .unwrap();
        let recipient = std::env::var("MARKET_TEST_RECIPIENT")
            .unwrap()
            .parse::<u64>()
            .unwrap();
        assert_eq!(credentials.bot_id(), expected);
        let captured: serde_json::Value = serde_json::from_slice(
            &std::fs::read(std::env::var("MARKET_TEST_PORTFOLIO").unwrap()).unwrap(),
        )
        .unwrap();
        let mut raw = captured["state"].clone();
        raw["valuation"]["items"] = captured["page"]["items"].clone();
        raw["version"] = captured["page"]["version"].clone();
        let state: PortfolioState = serde_json::from_value(raw).unwrap();
        assert_eq!(state.user_id, UserId::new(recipient).unwrap());
        let request = PortfolioRequest {
            user_id: recipient,
            currency_id: "GRAM".into(),
            refresh: false,
            page: 0,
            sort: std::env::var("MARKET_TEST_PORTFOLIO_SORT")
                .ok()
                .map(|value| parse_sort(&value).unwrap())
                .unwrap_or(PortfolioSort::PositionAsc),
            mode: PortfolioMode::Private,
        };
        let reply = render(&state, &request, &crate::ReportIcons::default()).bounded();
        if let Ok(path) = std::env::var("MARKET_TEST_REPLY_OUTPUT") {
            if path.ends_with(".json") {
                std::fs::write(
                    path,
                    serde_json::to_vec(&serde_json::json!({
                        "text": reply.text, "title": reply.title, "summary": reply.summary,
                        "rich_html": reply.rich_html, "emoji_documents": reply.emoji_documents,
                    }))
                    .unwrap(),
                )
                .unwrap();
            } else {
                std::fs::write(path, reply.rich_html.as_ref().unwrap()).unwrap();
            }
        }
        let mut reply = super::super::tests::static_preview(reply);
        for row in &mut reply.buttons {
            row.retain(|button| !matches!(button, ReplyButton::Callback { .. }));
        }
        reply.buttons.retain(|row| !row.is_empty());
        let scheduler = market_invocations::Runtime::start(Default::default()).unwrap();
        let directory = std::path::PathBuf::from(
            std::env::var("MARKET_TEST_SESSION_DIR")
                .expect("reuse an authorized session directory for live previews"),
        );
        let connection = Connection::open(credentials, &directory, scheduler.handle())
            .await
            .unwrap();
        let actions = connection.transport.actions();
        let user = actions.identity().await.unwrap();
        assert_eq!(user.id, expected);
        let event = market_telegram_client::interactions::Event::Message(Message {
            peer: PeerRef {
                id: PeerId::user(recipient as i64).unwrap(),
                auth: PeerAuth::from_hash(0),
            },
            id: super::super::tests::preview_message_id(),
            outgoing: false,
            text: String::new(),
        });
        let result = actions.send_progress(&event, &reply, None).await;
        connection.close().await.unwrap();
        scheduler.shutdown().await;
        let Some(market_telegram_client::interactions::Event::Callback {
            target: market_telegram_client::interactions::CallbackTarget::Message(sent),
            ..
        }) = result.unwrap()
        else {
            panic!("Telegram did not return the portfolio preview message identity");
        };
        eprintln!(
            "Sent 30-gift portfolio preview via @{} to authorized recipient {}; Telegram message_id={}",
            user.username, recipient, sent.id
        );
    }
    #[test]
    fn callbacks_are_bounded_and_do_not_retain_any_transport_credentials() {
        let request = callback("p:123:GRAM:r:2").unwrap();
        assert_eq!(request.user_id, 123);
        assert!(request.refresh);
        assert_eq!(request.page, 2);
        for invalid in [
            "p:0:GRAM:v:0",
            "p:1:GRAM:r:1000",
            "p:1:GRAM:other:0",
            "p:1:GRAM:v:0:extra",
        ] {
            assert!(callback(invalid).is_none(), "{invalid}");
        }
    }
    #[test]
    fn sorts_round_trip_through_pages_refresh_and_sharing() {
        for (sort, name, _) in SORTS {
            let mut request =
                parse_args(&format!("123 XTR sort={name}"), 7, PortfolioMode::Private).unwrap();
            assert_eq!(request.sort, sort);
            for refresh in [false, true] {
                let data = request.data(2, refresh, sort);
                assert!(data.len() <= 64);
                let parsed = callback(&data).unwrap();
                assert_eq!(
                    (
                        parsed.user_id,
                        parsed.currency_id.as_str(),
                        parsed.sort,
                        parsed.page,
                        parsed.refresh
                    ),
                    (123, "XTR", sort, 2, refresh)
                );
            }
            request.mode = PortfolioMode::Inline;
            let parsed = inline(&request.inline_query(), 99).unwrap();
            assert_eq!(
                (
                    parsed.user_id,
                    parsed.sort,
                    parsed.currency_id.as_str(),
                    parsed.mode
                ),
                (123, sort, "XTR", PortfolioMode::Inline)
            );
            for button in sort_controls(&request).split("data=\"").skip(1) {
                assert_eq!(callback(button.split('"').next().unwrap()).unwrap().page, 0);
            }
        }
        assert_eq!(
            callback("p:123:GRAM:v:2").unwrap().sort,
            PortfolioSort::PositionAsc
        );
        for invalid in [
            "p:1:GRAM:v:0:bad",
            "p:1:GRAM:v:0:vd:extra",
            "p:1:GRAM:r:1000:pa",
        ] {
            assert!(callback(invalid).is_none());
        }
        for invalid in [
            "sort=bad",
            "sort=price_asc sort=price_desc",
            "1 2",
            "-1",
            "0",
            "1 GRAM XTR",
        ] {
            assert!(
                parse_args(invalid, 7, PortfolioMode::Private).is_none(),
                "{invalid}"
            );
        }
    }
    #[test]
    fn shared_sorting_keeps_unpriced_last_and_every_price_inside_its_gift_link() {
        let mut state = sample(4);
        let report = std::sync::Arc::get_mut(state.valuation.as_mut().unwrap()).unwrap();
        let original = std::mem::take(&mut report.items);
        report.items = [2, 0, 3, 1]
            .into_iter()
            .map(|i| original[i].clone())
            .collect();
        for item in &mut report.items {
            item.estimate = match item.gift.gift.number.get() {
                1 => Some("10".into()),
                3 => None,
                _ => Some("30".into()),
            };
        }
        report.priced = 3;
        report.unpriced = 1;
        report.total = Some("70".into());
        let cache = PortfolioReadCache::default();
        for (sort, expected) in [
            (PortfolioSort::PositionAsc, [3, 1, 4, 2]),
            (PortfolioSort::PositionDesc, [2, 4, 1, 3]),
            (PortfolioSort::NumberAsc, [1, 2, 3, 4]),
            (PortfolioSort::NumberDesc, [4, 3, 2, 1]),
            (PortfolioSort::ValueAsc, [1, 2, 4, 3]),
            (PortfolioSort::ValueDesc, [2, 4, 1, 3]),
        ] {
            let mut request = callback("p:123:GRAM:v:0").unwrap();
            request.sort = sort;
            let reply = super::render(
                &state,
                &request,
                &crate::ReportIcons::default(),
                &cache,
                &tokio_util::sync::CancellationToken::new(),
            );
            let html = reply.rich_html.unwrap();
            let actual = html
                .split("https://t.me/nft/ResistanceDog-")
                .skip(1)
                .map(|cell| cell.split('"').next().unwrap().parse::<u32>().unwrap())
                .collect::<Vec<_>>();
            assert_eq!(actual, expected, "{sort:?}");
            for cell in html.split("<td><a href=").skip(1) {
                let linked = cell.split("</a>").next().unwrap();
                assert!(linked.contains("<b>"));
                assert!(linked.contains(&format!(
                    "emoji-id=\"{}\"",
                    crate::ReportIcon::Gram.definition().document.get()
                )));
            }
        }
    }
    #[test]
    fn cancelled_sorted_reads_do_not_build_a_reply_and_inline_has_no_refresh_button() {
        let state = sample(31);
        let cache = PortfolioReadCache::default();
        let stop = tokio_util::sync::CancellationToken::new();
        stop.cancel();
        let mut request = callback("p:123:GRAM:v:0:vd").unwrap();
        let cancelled = super::render(
            &state,
            &request,
            &crate::ReportIcons::default(),
            &cache,
            &stop,
        );
        assert!(cancelled.rich_html.is_none());
        request.mode = PortfolioMode::Inline;
        let reply = render(&state, &request, &crate::ReportIcons::default());
        assert!(
            reply
                .buttons
                .iter()
                .flatten()
                .all(|b| !matches!(b, ReplyButton::Callback { .. }))
        );
        assert!(reply.buttons.iter().flatten().any(|b| matches!(b, ReplyButton::SwitchInline { query, .. } if query.contains("sort=price_desc"))));
        assert_eq!(reply.title.as_deref(), Some("Portfolio"));
    }
    #[test]
    fn visible_unverified_and_hosted_gifts_remain_in_reports_and_totals() {
        use market_gifts::{TelegramDocumentId, TelegramGiftDetails, TelegramGiftId};
        use market_portfolio::{GiftRelation, InventoryGift};
        let items = [
            GiftRelation::Owned,
            GiftRelation::Unverified,
            GiftRelation::Hosted,
        ]
        .into_iter()
        .enumerate()
        .map(|(index, relation)| {
            let number = index as u32 + 1;
            market_valuation::InventoryValue {
                gift: InventoryGift {
                    gift: TelegramGiftDetails {
                        id: number.to_string().try_into().unwrap(),
                        slug: format!("Gift-{number}").try_into().unwrap(),
                        number: number.try_into().unwrap(),
                        collection: TelegramGiftId::new(123).unwrap(),
                        model: TelegramDocumentId::new(456).unwrap(),
                        backdrop: None,
                        pattern: None,
                    },
                    relation,
                },
                resolved: None,
                estimate: Some("2".into()),
                error: None,
            }
        })
        .collect();
        let state = PortfolioState {
            version: Some(
                "00000000000000000000000000000001"
                    .to_owned()
                    .try_into()
                    .unwrap(),
            ),
            user_id: UserId::new(123).unwrap(),
            refresh_id: 1,
            progress: market_portfolio::PortfolioProgress {
                stage: PortfolioStage::Ready,
                ..Default::default()
            },
            inventory: None,
            refresh_after: None,
            error: None,
            valuation: Some(std::sync::Arc::new(market_valuation::InventoryValuation {
                currency_id: "GRAM".into(),
                evaluated_at: jiff::Timestamp::now(),
                stale: false,
                total: Some("6".into()),
                priced: 3,
                unpriced: 0,
                items,
            })),
        };
        let reply = render(
            &state,
            &callback("p:123:GRAM:v:0").unwrap(),
            &crate::ReportIcons::default(),
        );
        assert!(reply.text.contains("Estimate 6 GRAM"));
        for number in 1..=3 {
            assert!(reply.text.contains(&format!("Gift-{number} · 2")));
        }
        assert!(!reply.text.contains("On-chain"));
        assert!(!reply.text.contains("Hosted"));
        let html = reply.rich_html.unwrap();
        assert_eq!(html.matches("https://t.me/nft/").count(), 3);
        assert!(!html.contains("On-chain"));
        assert!(!html.contains("Hosted"));
    }
    #[test]
    fn reports_expose_partial_inventory_and_plain_text_fallback() {
        let request = callback("p:123:GRAM:v:0").unwrap();
        let state = PortfolioState {
            version: Some(
                "00000000000000000000000000000001"
                    .to_owned()
                    .try_into()
                    .unwrap(),
            ),
            user_id: UserId::new(123).unwrap(),
            refresh_id: 1,
            progress: market_portfolio::PortfolioProgress {
                stage: PortfolioStage::Ready,
                ..Default::default()
            },
            inventory: Some(market_portfolio::InventorySummary {
                checked_at: jiff::Timestamp::now(),
                count: 0,
                scanned: 0,
                regular: 0,
                skipped: 0,
                complete: false,
            }),
            valuation: Some(std::sync::Arc::new(market_valuation::InventoryValuation {
                currency_id: "GRAM".into(),
                evaluated_at: jiff::Timestamp::now(),
                stale: false,
                total: None,
                priced: 0,
                unpriced: 1,
                items: vec![],
            })),
            refresh_after: None,
            error: None,
        };
        let reply = render(&state, &request, &crate::ReportIcons::default());
        assert!(reply.text.contains("Estimate — GRAM"));
        assert!(
            reply
                .rich_html
                .as_ref()
                .unwrap()
                .contains("Partial inventory")
        );
        assert!(
            reply
                .buttons
                .iter()
                .flatten()
                .any(|b| matches!(b,ReplyButton::Preview{payload,..} if payload=="portfolio_123"))
        );
    }
}
