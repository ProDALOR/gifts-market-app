use super::*;
pub(super) use crate::commands::rich::amount as display_amount;

pub(super) fn checked_time(at: jiff::Timestamp, now: jiff::Timestamp) -> String {
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
pub(super) fn sort_controls(request: &PortfolioRequest) -> String {
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
        let direction = if shown == ascending { "↑" } else { "↓" };
        html.push_str(&rich::button(
            &format!("{label} {direction}"),
            Some(&request.data(0, PortfolioAction::Navigate, target)),
            selected,
        ));
    }
    html.push_str("</tg-button-row>");
    html
}

pub(super) fn render(
    state: &PortfolioState,
    request: &PortfolioRequest,
    icons: &crate::ReportIcons,
    reads: &PortfolioReadCache,
    stop: &tokio_util::sync::CancellationToken,
) -> Reply {
    render_page(state, request, icons, reads, stop, None)
}
pub(super) fn render_page(
    state: &PortfolioState,
    request: &PortfolioRequest,
    icons: &crate::ReportIcons,
    reads: &PortfolioReadCache,
    stop: &tokio_util::sync::CancellationToken,
    mut selected: Option<(market_portfolio::PortfolioPage, usize)>,
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
        request.user_id.get()
    );
    if !phase.is_empty() {
        text.push_str(&format!("\n{phase}"));
        html.push_str(&format!("<p>{}</p>", escape(&phase)));
    }
    let mut reply = Reply::default();
    let count = selected.as_ref().map_or_else(
        || state.valuation.as_ref().map_or(0, |v| v.items.len()),
        |(p, _)| p.selection.count,
    );
    let size = request.page_size();
    let pages = count.div_ceil(size).max(1);
    let page = request.page.min(pages - 1);
    let start = selected.as_ref().map_or(page * size, |(_, offset)| *offset);
    let end = (start + selected.as_ref().map_or(size, |(p, _)| p.items.len())).min(count);
    let items = if let Some((selected, _)) = &mut selected {
        std::mem::take(&mut selected.items)
    } else if count > 0 {
        match reads.read(
            state,
            ResolvedPortfolioReadQuery {
                user_id: state.user_id,
                currency_id: request.currency_id.id(),
                filters: Default::default(),
                sort: request.sort,
                offset: start,
                limit: size,
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
        let total = selected
            .as_ref()
            .map_or(&report.total, |(p, _)| &p.selection.total)
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
        let unpriced = selected
            .as_ref()
            .map_or(report.unpriced, |(p, _)| p.selection.unpriced);
        if unpriced > 0 {
            let warning = format!("{unpriced} unpriced");
            html.push_str(&format!("<p>{warning}</p>"));
            text.push_str(&format!("\n{warning}"));
        }
        if count > 0 && selected.is_none() {
            html.push_str(&sort_controls(request));
            html.push_str("<tg-button-row>");
            for (layout, label) in [(GridMode::Linked, "Linked"), (GridMode::Gallery, "Gallery")] {
                let target = PortfolioRequest {
                    layout,
                    page: 0,
                    ..request.clone()
                };
                let data = target.data(0, PortfolioAction::Navigate, request.sort);
                html.push_str(&rich::button(
                    label,
                    (layout != request.layout).then_some(data.as_str()),
                    layout == request.layout,
                ));
            }
            html.push_str("</tg-button-row>");
        }
        let mut cells = Vec::with_capacity(items.len());
        for item in &items {
            let price = item
                .estimate
                .as_ref()
                .map(display_amount)
                .unwrap_or_else(|| "—".into());
            text.push_str(&format!("\n{} · {price}", item.gift.gift.slug));
            let gift = &item.gift.gift;
            let image = reply.custom_emoji(gift.model);
            cells.push(rich::price(
                &image,
                &price,
                &currency,
                Some(&format!("https://t.me/nft/{}", gift.slug)),
            ));
        }
        if count > 0 {
            html.push_str(&rich::grid(&cells, rich::COLUMNS));
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
    let button = |label: &str, page, action| ReplyButton::Callback {
        text: label.into(),
        data: request.data(page, action, request.sort).into_bytes(),
    };
    if pages > 1 && selected.is_none() {
        let navigation = |label: &str, target: Option<usize>| {
            let data =
                target.map(|target| request.data(target, PortfolioAction::Navigate, request.sort));
            rich::button(label, data.as_deref(), false)
        };
        html.push_str(&format!("<tg-button-row>{}<tg-button type=\"disabled\">{} / {pages}</tg-button>{}</tg-button-row>",
            navigation("←", page.checked_sub(1)), page+1,
            navigation("→", (page+1<pages).then_some(page+1))));
    }
    let mut row = vec![];
    if request.mode == PortfolioMode::Private {
        row.push(button("Refresh gifts", 0, PortfolioAction::Refresh));
    }
    row.extend([
        ReplyButton::SwitchInline {
            text: "Share".into(),
            query: request.inline_query(),
        },
        ReplyButton::Preview {
            text: "Open portfolio".into(),
            payload: format!("portfolio_{}", request.user_id.get()),
        },
    ]);
    reply.text = text;
    reply.rich_html = Some(html);
    reply.title = Some("Portfolio".into());
    reply.buttons = vec![row];
    reply
}
