use super::{CommandKind, Reply};
use market_gifts::SyncState;
use market_valuation::DisplayValue;
use market_valuation::ValuationReport;
pub(super) fn error_reply(error: impl std::fmt::Display) -> Reply {
    Reply {
        text: error.to_string().chars().take(300).collect(),
    }
}
pub(super) fn valuation(value: &ValuationReport) -> Reply {
    let label = value
        .target
        .slug()
        .map(ToString::to_string)
        .unwrap_or_else(|| "Gift valuation".into());
    let mut text = format!("{label}\nCurrency: {}", value.currency_id);
    for result in &value.results {
        let name = market_valuation::strategy(result.strategy())
            .map_or(result.strategy(), |metadata| metadata.name);
        text.push_str(&format!("\n\n{name}"));
        let display = result.display(&value.currency_id);
        for field in display.fields {
            text.push_str(&format!("\n\n{}: ", field.label));
            match field.value {
                DisplayValue::Currency {
                    amount,
                    currency_id,
                } => text.push_str(&format!("{amount} {currency_id}")),
                DisplayValue::Number { amount, suffix } => {
                    text.push_str(&format!("{amount}{suffix}"))
                }
                DisplayValue::Unavailable { message, .. } => {
                    text.push_str(&message.chars().take(300).collect::<String>())
                }
            }
        }
        if let Some(summary) = display.summary {
            text.push_str(&format!("\n{summary}"));
        }
        if !display.note.is_empty() {
            text.push_str(&format!("\n{}", display.note));
        }
    }
    text.push_str(&format!(
        "\n\nEvaluated: {}",
        value.evaluated_at.strftime("%Y-%m-%d %H:%M:%S UTC")
    ));
    if value.stale {
        text.push_str("\nMarket updates are delayed; this estimate may be outdated.");
    }
    Reply { text }
}
pub(super) fn usage(_kind: CommandKind) -> Reply {
    Reply { text: "Use /value PlushPepe-123 [GRAM|XTR] or /value https://t.me/nft/PlushPepe-123. Only gifts listed in available market data can be resolved.".into() }
}
pub(crate) fn launch_reply(command: CommandKind) -> Option<Reply> {
    Some(Reply { text: match command {
        CommandKind::Start | CommandKind::Market => "Browse live gift markets, compare venues and filter offers in one place.",
        CommandKind::Help => "/value <gift URL or slug> [GRAM|XTR] — unified gift valuation. You can also send a gift link directly.\n/status — market availability.\n/market — open the market.\nGift lookup uses available market listings; unlisted gifts are unavailable.",
        _ => return None,
    }.into() })
}
pub(super) fn status_reply(status: market_gifts::SyncStatus) -> Reply {
    let state = if !status.ready {
        "unavailable"
    } else if status.state == SyncState::Live {
        "available"
    } else {
        "updates delayed"
    };
    let detail = if !status.ready {
        "Preparing market data. Please check again shortly."
    } else if status.state != SyncState::Live {
        "Showing previously received listings."
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
            collection_floor: metric("3.123456789"),
            model_floor: metric("10"),
            backdrop_floor: metric("11"),
            model_backdrop_floor: metric("100"),
            model_multiplier: ValuationMetric::Value {
                value: Multiplier::new("1.2").unwrap(),
            },
            backdrop_multiplier: ValuationMetric::Value {
                value: Multiplier::new("3.67").unwrap(),
            },
            final_mark: metric("36.67"),
            evidence: GiftFloorEvidence {
                collection: FloorEvidence {
                    listings: 12,
                    gifts: 12,
                },
                model: FloorEvidence {
                    listings: 6,
                    gifts: 6,
                },
                backdrop: FloorEvidence {
                    listings: 6,
                    gifts: 6,
                },
                model_backdrop: FloorEvidence {
                    listings: 3,
                    gifts: 3,
                },
                minimum_multiplier_gifts: 3,
            },
        };
        let mark = ValuationReport {
            target: ResolvedTarget::Gift {
                gift: CollectibleGift {
                    id: "gift".into(),
                    collection_id: "collection".into(),
                    number: Some(GiftNumber::new(123).unwrap()),
                    traits: GiftTraits::default(),
                },
                slug: Some(GiftSlug::new("Gift-123").unwrap()),
            },
            currency_id: "GRAM".into(),
            evaluated_at: jiff::Timestamp::now(),
            stale: false,
            results: vec![StrategyOutcome::Value(StrategyValuation::GiftFloor(report))],
        };
        let mut mark = mark;
        mark.results.insert(
            0,
            StrategyOutcome::Value(StrategyValuation::AskFloor(AskFloorValuation {
                price: metric("100"),
                evidence: FloorEvidence {
                    listings: 2,
                    gifts: 1,
                },
            })),
        );
        mark.results.push(StrategyOutcome::Error {
            strategy: StrategyId::new("external_strategy").unwrap(),
            error: StrategyFailure {
                code: StrategyFailureCode::NotEnoughData,
                message: "History unavailable".into(),
            },
        });
        let text = valuation(&mark).text;
        assert!(text.contains("Collection floor: 3.123456789 GRAM"));
        assert!(text.contains("Model multiplier: 1.20×"));
        assert!(text.contains("Backdrop multiplier: 3.67×"));
        assert!(text.contains("Final mark: 36.67 GRAM"));
        assert!(text.contains("Model & backdrop floor: 100 GRAM"));
        assert!(text.contains("Ask floor"));
        assert!(text.contains("Gift floor"));
        assert!(text.contains("external_strategy"));
        assert!(text.contains("History unavailable"));
        assert!(text.contains("Lowest eligible ask, before fees."));
        assert!(!text.contains("uses unrounded multipliers"));
    }
}
