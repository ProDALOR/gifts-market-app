use super::{CommandKind, Reply};
use market_actors::error::DataError;
use market_gifts::{
    SyncState,
    valuation::{UnpricedReason, Valuation},
};
use market_telegram::{
    GiftRelation, InventoryCompletion, InventoryError, PartialReason, VisibleInventory,
};
pub(super) fn error_reply(error: DataError) -> Reply {
    use market_actors::error::DataErrorCode;
    let text = match error.code() {
        DataErrorCode::InvalidQuery => error.message().to_owned(),
        DataErrorCode::CapacityExceeded => {
            "Market queries are busy. Please try again shortly.".into()
        }
        _ => "Market data is unavailable. Please try again shortly.".into(),
    };
    Reply { text }
}
fn freshness(value: &Valuation) -> &'static str {
    if value.sync.state == SyncState::Live {
        ""
    } else {
        "\nMarket data is not live; using the last synchronized listings."
    }
}
fn label(value: &str) -> String {
    let mut chars = value.chars();
    let text: String = chars.by_ref().take(40).collect();
    if chars.next().is_some() {
        format!("{text}…")
    } else {
        text
    }
}
pub(super) fn floor(value: Valuation) -> Reply {
    let row = &value.rows[0];
    let text = if let Some(price) = &row.floor {
        format!(
            "{} · {}\nModel floor: {} {}\nLowest current ask across loaded venues ({}).{}",
            label(&row.collection),
            label(&row.model),
            price,
            label(&value.currency_id),
            label(row.venue_id.as_deref().unwrap_or("market")),
            freshness(&value)
        )
    } else {
        let detail = match row.unpriced {
            Some(UnpricedReason::UnknownCollection) => {
                "Collection not found. Use its exact catalog name."
            }
            Some(UnpricedReason::AmbiguousCollection) => {
                "Collection name is ambiguous. Use its catalog ID."
            }
            Some(UnpricedReason::UnknownModel) => {
                "Model not found in this collection. Use its exact name."
            }
            Some(UnpricedReason::AmbiguousModel) => "Model name is ambiguous. Use its catalog ID.",
            _ => "No current model ask is available in this currency.",
        };
        format!(
            "{} · {}\n{}{}",
            label(&row.collection),
            label(&row.model),
            detail,
            freshness(&value)
        )
    };
    Reply { text }
}
pub(super) fn portfolio(value: Valuation, inventory: &VisibleInventory) -> Reply {
    let regular = inventory.regular_gifts;
    let unsupported = inventory.skipped_count();
    let mut text = format!(
        "Your visible gift portfolio\nTelegram gifts read: {}{} across {} page(s)\nModel-floor estimate: {} {}\nPriced collectibles: {}\nWithout a model floor: {}",
        inventory.gifts_scanned,
        inventory
            .reported_total
            .map(|total| format!(" of {total}"))
            .unwrap_or_default(),
        inventory.pages_read,
        value.total,
        value.currency_id,
        value.priced,
        value.unpriced,
    );
    for row in value.rows.iter().filter(|row| row.value.is_some()).take(10) {
        text.push_str(&format!(
            "\n{} · {} × {} = {} {}",
            label(&row.collection),
            label(&row.model),
            row.quantity,
            row.value.as_deref().unwrap(),
            value.currency_id
        ));
    }
    if value.rows.iter().filter(|row| row.value.is_some()).count() > 10 {
        text.push_str("\nAdditional models are included in the total.");
    }
    if unsupported > 0 {
        text.push_str(&format!(
            "\nUnsupported visible collectibles skipped: {unsupported}."
        ));
    }
    if regular > 0 {
        text.push_str(&format!(
            "\nRegular, non-collectible gifts skipped: {regular}."
        ));
    }
    if inventory.relation_count(GiftRelation::Hosted) > 0 {
        text.push_str(&format!(
            "\nHosted collectibles included: {}. These are linked to your profile; wallet ownership is not verified.",
            inventory.relation_count(GiftRelation::Hosted)
        ));
    }
    if let InventoryCompletion::Partial(reason) = &inventory.completion {
        let detail = match reason {
            PartialReason::NotRead => "inventory was not read",
            PartialReason::LimitReached => "read limit reached",
            PartialReason::TimedOut => "Telegram read timed out",
            PartialReason::MissingCursor => {
                "Telegram returned fewer gifts than advertised without a continuation cursor"
            }
            PartialReason::RepeatedCursor => "Telegram repeated a continuation cursor",
            PartialReason::ReadFailed(InventoryError::RetryAfter(_)) => {
                "Telegram requested a rate-limit wait"
            }
            PartialReason::ReadFailed(_) => "a later Telegram page could not be read",
        };
        text.push_str(&format!(
            "\nPartial inventory: {detail}. Only the gifts read are included in this estimate."
        ));
    }
    for (reason, count) in &inventory.skipped {
        text.push_str(&format!("\nSkipped {}: {count}.", reason.description()));
    }
    let unverified = inventory.relation_count(GiftRelation::Unverified);
    if unverified > 0 {
        text.push_str(&format!(
            "\nCollectibles with undisclosed ownership: {unverified}."
        ));
    }
    if value.unpriced > 0 || unsupported > 0 {
        text.push_str("\nUnpriced gifts are excluded from the estimate.");
    }
    text.push_str(
        "\nBased on current asks, without fees. Only gifts Telegram makes visible to this bot are included.",
    );
    text.push_str(freshness(&value));
    Reply { text }
}

pub(super) fn usage(kind: CommandKind) -> Reply {
    Reply{text:match kind {
        CommandKind::Floor=>"Use /floor Collection | Model\nExample: /floor Bunny Muffin | Red Alert\nOptional: /floor Collection | Model | GRAM",
        CommandKind::Portfolio=>"Use /portfolio or /portfolio GRAM to value your Telegram-visible collectibles.",
        _=>"Use /help to see available commands.",
    }.into()}
}
pub(crate) fn launch_reply(command: CommandKind) -> Option<Reply> {
    Some(Reply{text:match command {
        CommandKind::Start|CommandKind::Market=>"Browse live gift markets, compare venues and filter offers in one place.",
        CommandKind::Help=>"/floor Collection | Model — lowest current model ask. Add | GRAM for a quote currency.\n/portfolio — floor estimate of your Telegram-visible collectibles.\n/status — market synchronization status.\n/market — open the market when a launch URL is configured.",
        _=>return None,
    }.into()})
}
pub(super) fn inventory_error(error: InventoryError) -> Reply {
    Reply{text:match error {
        InventoryError::Authorization=>"Telegram authorization failed.".into(),
        InventoryError::Forbidden=>"Telegram did not allow this bot to read your profile gifts. Your portfolio could not be valued.".into(),
        InventoryError::RetryAfter(seconds)=>format!("Telegram requested a wait. Please try /portfolio again in {seconds} seconds."),
        InventoryError::Unavailable=>"Your visible gifts could not be read from Telegram. Please try again shortly.".into(),
    }}
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

#[cfg(test)]
mod tests {
    use super::*;
    use market_telegram::SkipReason;
    #[test]
    fn portfolio_reports_precise_totals_skip_reasons_partial_reads_and_freshness() {
        let value = market_gifts::valuation::Valuation {
            rows: vec![market_gifts::valuation::ModelValue {
                identity: None,
                collection: "Collection".into(),
                model: "Model".into(),
                quantity: 2,
                floor: Some("2.000000000000000001".into()),
                value: Some("4.000000000000000002".into()),
                venue_id: Some("market".into()),
                unpriced: None,
            }],
            total: "4.000000000000000002".into(),
            priced: 2,
            unpriced: 1,
            currency_id: "GRAM".into(),
            cursor: "c".into(),
            sync: market_gifts::SyncStatus {
                ready: true,
                state: market_gifts::SyncState::Stale,
                updated_at: None,
            },
        };
        let inventory = VisibleInventory {
            gifts: vec![market_telegram::ProfileGift {
                id: 1,
                collection_id: market_gifts::TelegramGiftId::new(2).unwrap(),
                model_document_id: market_gifts::TelegramDocumentId::new(3).unwrap(),
                relation: GiftRelation::Hosted,
            }],
            regular_gifts: 45,
            skipped: [(SkipReason::MissingModel, 129)].into(),
            pages_read: 1,
            gifts_scanned: 100,
            reported_total: Some(700),
            completion: InventoryCompletion::Partial(PartialReason::TimedOut),
        };
        let reply = portfolio(value, &inventory);
        for text in [
            "4.000000000000000002 GRAM",
            "100 of 700",
            "Without a model floor: 1",
            "Skipped missing model document: 129",
            "Hosted collectibles included: 1",
            "wallet ownership is not verified",
            "Partial inventory: Telegram read timed out",
            "Market data is not live",
            "Unpriced gifts are excluded",
        ] {
            assert!(reply.text.contains(text), "missing {text}");
        }
    }
}
