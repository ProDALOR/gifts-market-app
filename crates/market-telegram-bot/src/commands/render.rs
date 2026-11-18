use super::{CommandKind, Reply};
use market_gifts::SyncState;
use market_valuation::GiftFloorMark;
use market_valuation::ValuationMetric;
pub(super) fn error_reply(error: impl std::fmt::Display) -> Reply {
    Reply {
        text: error.to_string().chars().take(300).collect(),
    }
}
pub(super) fn valuation(value: &GiftFloorMark) -> Reply {
    let label = value
        .slug
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_else(|| "Gift valuation".into());
    let mut text = format!("{label}\nCurrency: {}", value.currency_id);
    for (label, metric, ratio, rounded) in [
        ("Collection floor", &value.collection_floor, false, false),
        ("Model floor", &value.model_floor, false, false),
        ("Backdrop floor", &value.backdrop_floor, false, false),
        (
            "Model × backdrop floor",
            &value.model_backdrop_floor,
            false,
            false,
        ),
        ("Model multiplier", &value.model_multiplier, true, true),
        (
            "Backdrop multiplier",
            &value.backdrop_multiplier,
            true,
            true,
        ),
        ("Final mark", &value.final_mark, false, true),
    ] {
        text.push_str(&format!("\n\n{label}: "));
        match metric {
            ValuationMetric::Value { value: amount } => {
                let amount = if rounded {
                    format!("{:.2}", amount.decimal())
                } else {
                    amount.to_string()
                };
                if ratio {
                    text.push_str(&format!("{amount}×"));
                } else {
                    text.push_str(&format!("{amount} {}", value.currency_id));
                }
            }
            ValuationMetric::Unavailable { error } => {
                text.push_str(&error.message.chars().take(300).collect::<String>())
            }
        }
    }
    text.push_str(&format!(
        "\n\nEvaluated: {}",
        value.evaluated_at.strftime("%Y-%m-%d %H:%M:%S UTC")
    ));
    text.push_str(
        "\nBased on eligible asks, before fees. The final mark uses unrounded multipliers.",
    );
    if value.sync.state != SyncState::Live {
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
        CommandKind::Help => "/value <gift URL or slug> [GRAM|XTR] — unified gift valuation. You can also send a gift link directly.\n/status — synchronization status.\n/market — open the market.\nGift lookup uses synchronized market orders; unlisted gifts are unavailable.",
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

#[cfg(test)]
mod tests {
    use super::*;
    use market_gifts::*;
    use market_valuation::*;
    #[test]
    fn unified_reply_preserves_floor_precision_and_formats_final_mark_and_ratios() {
        let metric = |value| ValuationMetric::Value {
            value: Amount::new(value).unwrap(),
        };
        let report = GiftFloorValuation {
            slug: Some(GiftSlug::new("Gift-123").unwrap()),
            gift: CollectibleGift {
                id: "gift".into(),
                collection_id: "collection".into(),
                number: Some(GiftNumber::new(123).unwrap()),
                traits: GiftTraits::default(),
            },
            currency_id: "GRAM".into(),
            collection_floor: metric("3.123456789"),
            model_floor: metric("10"),
            backdrop_floor: metric("11"),
            model_backdrop_floor: metric("100"),
            model_multiplier: metric("1.2"),
            backdrop_multiplier: metric("3.67"),
            final_mark: metric("36.67"),
            evidence: GiftFloorEvidence {
                collection_samples: 12,
                model_samples: 6,
                backdrop_samples: 6,
                model_backdrop_samples: 3,
                minimum_multiplier_samples: 3,
            },
            sync: SyncStatus {
                ready: true,
                state: SyncState::Live,
                updated_at: None,
            },
        };
        let mark = market_valuation::Mark {
            strategy: "gift_floor",
            data: report,
            revision: "pinned".into(),
            evaluated_at: jiff::Timestamp::now(),
        };
        let text = valuation(&mark).text;
        assert!(text.contains("Collection floor: 3.123456789 GRAM"));
        assert!(text.contains("Model multiplier: 1.20×"));
        assert!(text.contains("Backdrop multiplier: 3.67×"));
        assert!(text.contains("Final mark: 36.67 GRAM"));
        assert!(text.contains("Model × backdrop floor: 100 GRAM"));
    }
}
