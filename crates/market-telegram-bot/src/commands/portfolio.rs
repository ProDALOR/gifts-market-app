use super::*;
use market_portfolio::{PortfolioQuery, PortfolioStage, PortfolioState, UserId};
use market_telegram::ReplyButton;
use std::time::Duration;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortfolioRequest {
    pub user_id: u64,
    pub currency_id: market_gifts::CurrencyId,
    pub refresh: bool,
    pub page: usize,
}
pub(super) fn callback(text: &str) -> Option<PortfolioRequest> {
    let mut p = text.strip_prefix("p:")?.split(':');
    let user_id = p.next()?.parse().ok()?;
    let currency_id = p.next()?.to_owned().try_into().ok()?;
    let action = p.next()?;
    let page = p.next()?.parse::<usize>().ok()?;
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
            return render(&retained, &request);
        }
        let wait = async {
            loop {
                let reply = render(&state, &request);
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
        tokio::time::timeout(Duration::from_secs(80), wait)
            .await
            .unwrap_or_else(|_| {
                let mut reply = render(&watch.current(), &request);
                reply
                    .text
                    .push_str("\nOpen portfolio to keep following progress.");
                if let Some(html) = &mut reply.rich_html {
                    html.push_str("<p>Open portfolio to keep following progress.</p>");
                }
                reply
            })
    }
}
fn relation_label(relation: market_portfolio::GiftRelation) -> Option<&'static str> {
    use market_portfolio::GiftRelation;
    match relation {
        GiftRelation::Owned => None,
        GiftRelation::Hosted => Some("Hosted"),
        GiftRelation::Unverified => Some("Ownership unverified"),
    }
}
fn render(state: &PortfolioState, request: &PortfolioRequest) -> Reply {
    use super::report::{amount, escape};
    let phase = match state.progress.stage {
        PortfolioStage::Idle => "Not checked yet".into(),
        PortfolioStage::Queued => "Waiting to check gifts…".into(),
        PortfolioStage::LoadingInventory => match state.progress.total {
            Some(total) => format!(
                "{} of {total} gifts checked · {} pages",
                state.progress.checked, state.progress.pages
            ),
            None => format!(
                "{} gifts checked · {} pages",
                state.progress.checked, state.progress.pages
            ),
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
    let mut text = format!("Portfolio · {}\n{phase}", request.user_id);
    let mut html = format!(
        "<h3>Portfolio · {}</h3><p>{}</p>",
        request.user_id,
        escape(&phase)
    );
    let mut rows = 0;
    if let Some(report) = &state.valuation {
        let total = report
            .total
            .as_ref()
            .map(amount)
            .unwrap_or_else(|| "—".into());
        text.push_str(&format!(
            "\nEstimate {total} {}\n{} priced · {} unpriced",
            report.currency_id, report.priced, report.unpriced
        ));
        html.push_str(&format!(
            "<h2>{total} {}</h2><p>{} priced · {} unpriced</p>",
            report.currency_id, report.priced, report.unpriced
        ));
        let unverified = report
            .items
            .iter()
            .filter(|item| item.gift.relation == market_portfolio::GiftRelation::Unverified)
            .count();
        if unverified > 0 {
            text.push_str(&format!("\n{unverified} visible · ownership unverified"));
            html.push_str(&format!(
                "<p>{unverified} visible · ownership unverified</p>"
            ));
        }
        html.push_str("<table>");
        let page = request.page.min(report.items.len().saturating_sub(1) / 10);
        for item in report.items.iter().skip(page * 10).take(10) {
            let price = item
                .estimate
                .as_ref()
                .map(amount)
                .unwrap_or_else(|| "—".into());
            let relation = relation_label(item.gift.relation);
            text.push_str(&format!("\n{} · {price}", item.gift.gift.slug));
            if let Some(label) = relation {
                text.push_str(&format!(" · {label}"));
            }
            let note = relation
                .map(|label| format!("<br><small>{label}</small>"))
                .unwrap_or_default();
            html.push_str(&format!(
                "<tr><td><a href=\"https://t.me/nft/{}\">{}</a>{note}</td><td>{price}</td></tr>",
                item.gift.gift.slug,
                escape(&item.gift.gift.slug.to_string())
            ));
        }
        html.push_str("</table>");
        rows = report.items.len();
    }
    if state.valuation.as_ref().is_some_and(|v| v.stale) {
        html.push_str("<p>Market updates delayed</p>");
        text.push_str("\nMarket updates delayed");
    }
    if let Some(inventory) = &state.inventory {
        let checked = inventory.checked_at.strftime("%d %b %H:%M UTC").to_string();
        let suffix = if inventory.complete {
            ""
        } else {
            " · Partial inventory"
        };
        html.push_str(&format!("<p>Gifts checked {checked}{suffix}</p>"));
        text.push_str(&format!("\nGifts checked {checked}{suffix}"));
    }
    let button = |label: &str, page, refresh| ReplyButton::Callback {
        text: label.into(),
        data: format!(
            "p:{}:{}:{}:{page}",
            request.user_id,
            request.currency_id,
            if refresh { "r" } else { "v" }
        )
        .into_bytes(),
    };
    let mut pages = vec![];
    let page = request.page.min(rows.saturating_sub(1) / 10);
    if page > 0 {
        pages.push(button("←", page - 1, false));
    }
    if (page + 1) * 10 < rows {
        pages.push(button("→", page + 1, false));
    }
    let mut buttons = vec![];
    if !pages.is_empty() {
        buttons.push(pages);
    }
    buttons.push(vec![
        button("Refresh gifts", 0, true),
        ReplyButton::Preview {
            text: "Open portfolio".into(),
            payload: format!("portfolio_{}", request.user_id),
        },
    ]);
    Reply {
        text,
        rich_html: Some(html),
        buttons,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let reply = render(&state, &callback("p:123:GRAM:v:0").unwrap());
        assert!(reply.text.contains("Estimate 6 GRAM"));
        assert!(reply.text.contains("Gift-2 · 2 · Ownership unverified"));
        assert!(reply.text.contains("Gift-3 · 2 · Hosted"));
        assert!(
            !reply.text.contains("Gift-1 · 2 ·"),
            "owned gifts need no extra label"
        );
        let html = reply.rich_html.unwrap();
        assert!(html.contains("1 visible · ownership unverified"));
        assert!(html.contains("Gift-2</a><br><small>Ownership unverified</small>"));
        assert!(html.contains("Gift-3</a><br><small>Hosted</small>"));
    }
    #[test]
    fn reports_expose_partial_inventory_and_plain_text_fallback() {
        let request = callback("p:123:GRAM:v:0").unwrap();
        let state = PortfolioState {
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
        let reply = render(&state, &request);
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
