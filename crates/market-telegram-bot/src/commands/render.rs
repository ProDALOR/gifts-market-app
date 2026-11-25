use super::Reply;
use market_gifts::SyncState;
pub(super) fn error_reply(error: impl std::fmt::Display) -> Reply {
    Reply {
        text: error.to_string().chars().take(300).collect(),
        ..Default::default()
    }
}
pub(crate) fn launch_reply(command: &super::Command) -> Option<Reply> {
    use super::Command;
    let usage =
        "Send a gift link or slug, for example PlushPepe-123. Add TON or XTR to choose currency.";
    Some(Reply::text(match command {
        Command::Start => format!("{usage}\nOpen the market to browse gifts."),
        Command::Help => {
            let commands = super::CommandKind::ALL
                .into_iter()
                .map(|command| format!("{} — {}", command.usage(), command.description()))
                .collect::<Vec<_>>()
                .join("\n");
            format!(
                "{usage}\n{commands}\nSort portfolios by position, price or gift number using the buttons.\nInline: type this bot’s username, then a gift slug or portfolio. Check gifts privately first; inline uses cached data.\nSales show observed executions; history may be incomplete."
            )
        }
        Command::GiftHelp => usage.to_owned(),
        _ => return None,
    }))
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
        ..Default::default()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use market_gifts::*;
    use market_valuation::*;
    pub(crate) fn fixture() -> ValuationReport {
        let metric = |value| ValuationMetric::Value {
            value: Amount::new(value).unwrap(),
        };
        let report = GiftFloorValuation {
            collection_floor: FloorValuation {
                price: metric("3.123456789"),
                count: 12,
            },
            model_floor: FloorValuation {
                price: metric("10"),
                count: 6,
            },
            backdrop_floor: FloorValuation {
                price: metric("11"),
                count: 6,
            },
            model_backdrop_floor: FloorValuation {
                price: metric("100"),
                count: 6,
            },
            model_multiplier: ValuationMetric::Value {
                value: Multiplier::new("1.2").unwrap(),
            },
            backdrop_multiplier: ValuationMetric::Value {
                value: Multiplier::new("3.67").unwrap(),
            },
            model_premium: metric("6.876543211"),
            backdrop_premium: metric("7.876543211"),
            additive_estimate: metric("17.876543211"),
            final_mark: metric("17.88"),
            basis: Some(GiftFloorBasis::PairCapped),
        };
        ValuationReport {
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
            results: vec![StrategyOutcome::Value(StrategyValuation::GiftFloor(
                report.into(),
            ))],
        }
    }
}
