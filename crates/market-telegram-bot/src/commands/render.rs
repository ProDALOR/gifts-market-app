use super::Reply;
use market_gifts::SyncState;
pub(super) fn error_reply(error: impl std::fmt::Display) -> Reply {
    Reply {
        text: error.to_string().chars().take(300).collect(),
        ..Default::default()
    }
}
pub(crate) fn launch_reply(command: &super::Command, user: u64) -> Option<Reply> {
    use super::Command;
    use market_telegram::{ReplyButton, ShareLink};
    let usage = "Send a gift link or slug for its estimate. Share gifts, markets and cached portfolios by typing this bot’s username in any chat. Check gifts privately first. Sales show observed executions; history may be incomplete.";
    let text = match command {
        Command::Start => "Send a gift link or slug, browse markets or check your portfolio.",
        Command::Help => usage,
        Command::GiftHelp => "Send one gift link or slug, for example PlushPepe-123.",
        _ => return None,
    };
    let mut reply = Reply::text(text);
    if matches!(command, Command::Start | Command::Help) {
        let target = market_links::Target::Market {
            selection: market_links::MarketSelection::from_query("f=&currency=GRAM")
                .expect("landing selection"),
        };
        reply.buttons.push(vec![ReplyButton::OpenApp {
            text: "Open app".into(),
            payload: target.launch(512).ok()??,
        }]);
        if let Ok(user) = market_portfolio::UserId::new(user) {
            reply.buttons.push(vec![ReplyButton::Callback {
                text: "Check my gifts".into(),
                data: format!("p:{}:GRAM:r:0:pa", user.get()).into_bytes(),
            }]);
        }
        reply.buttons.push(vec![ReplyButton::Share {
            text: "Share in a chat".into(),
            destination: ShareLink::Inline(String::new()),
        }]);
    }
    Some(reply)
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
