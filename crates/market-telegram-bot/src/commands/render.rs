use super::{CommandKind, Reply};
use market_gifts::SyncState;
use market_valuation::{GiftValuationResult, ValuationError, ValuationResult};
pub(super) fn error_reply(error: impl Into<ValuationError>) -> Reply {
    let error = error.into();
    Reply {
        text: error.message().chars().take(300).collect(),
    }
}
pub(super) fn valuation(value: &GiftValuationResult) -> Reply {
    let mut text = format!("{}\nCurrency: {}", value.slug, value.currency_id);
    let mut stale = false;
    for row in &value.strategies {
        text.push_str(&format!("\n\n{}", row.strategy));
        match &row.result {
            Err(error) => text.push_str(&format!(
                "\n{}",
                error.message().chars().take(300).collect::<String>()
            )),
            Ok(result) => {
                let at = match result {
                    ValuationResult::ModelFloor(mark) | ValuationResult::CollectionFloor(mark) => {
                        text.push_str(&format!("\n{} {}", mark.total, mark.currency_id));
                        stale |= mark.sync.state != SyncState::Live;
                        mark.evaluated_at
                    }
                    ValuationResult::ModelMultiplier(mark) => {
                        text.push_str(&format!(
                            "\n{}× ask-floor ratio\nTrait: {} {}; collection: {} {}",
                            mark.factor,
                            mark.evidence.selected_trait.floor,
                            mark.currency_id,
                            mark.evidence.collection.floor,
                            mark.currency_id
                        ));
                        stale |= mark.sync.state != SyncState::Live;
                        mark.evaluated_at
                    }
                    ValuationResult::BackdropMultiplier(mark) => {
                        text.push_str(&format!(
                            "\n{}× ask-floor ratio\nTrait: {} {}; collection: {} {}",
                            mark.factor,
                            mark.evidence.selected_trait.floor,
                            mark.currency_id,
                            mark.evidence.collection.floor,
                            mark.currency_id
                        ));
                        stale |= mark.sync.state != SyncState::Live;
                        mark.evaluated_at
                    }
                };
                text.push_str(&format!(
                    "\nEvaluated: {}",
                    at.strftime("%Y-%m-%d %H:%M:%S UTC")
                ));
            }
        }
    }
    text.push_str("\n\nBased on eligible asks, without fees. Ratios are not price estimates.");
    if stale {
        text.push_str("\nMarket data is not live; using the last synchronized listings.");
    }
    Reply { text }
}
pub(super) fn usage(_kind: CommandKind) -> Reply {
    Reply { text: "Use /value PlushPepe-123 [GRAM|XTR] or /value https://t.me/nft/PlushPepe-123. Only gifts in synchronized market orders can be resolved.".into() }
}
pub(crate) fn launch_reply(command: CommandKind) -> Option<Reply> {
    Some(Reply { text: match command {
        CommandKind::Start | CommandKind::Market => "Browse live gift markets, compare venues and filter offers in one place.",
        CommandKind::Help => "/value <gift URL or slug> [GRAM|XTR] — all valuation strategies. You can also send a gift link directly.\n/status — synchronization status.\n/market — open the market.\nGift lookup uses synchronized market orders; unlisted gifts are unavailable.",
        _ => return None,
    }.into() })
}
pub(super) fn status_reply(status: market_gifts::SyncStatus) -> Reply {
    let state = match status.state {
        SyncState::Starting => "starting",
        SyncState::Syncing => "synchronizing",
        SyncState::Live => "live",
        SyncState::Stale => "stale",
    };
    let detail = if !status.ready {
        "Preparing market data. Please check again shortly."
    } else if status.state == SyncState::Stale {
        "Showing the last synchronized market data."
    } else {
        "Ready to browse."
    };
    Reply {
        text: format!("Market data: {state}. {detail}"),
    }
}
