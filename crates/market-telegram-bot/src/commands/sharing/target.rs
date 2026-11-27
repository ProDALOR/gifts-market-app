//! Readable targets are self-contained. Long selections use a stateless application URL.
use super::*;
use market_gifts::{GiftNumberRange, OrderSource};
use market_telegram::ReplyButton;
pub type PortfolioTarget = market_portfolio::PortfolioTarget<
    market_actors::input::CollectionRef,
    market_actors::input::ModelRef,
    market_actors::input::BackdropRef,
    market_actors::input::PatternRef,
>;
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShareFallback {
    Inline { query: String },
    Link { url: url::Url },
}
fn invalid(error: impl std::fmt::Display) -> ShareError {
    ShareError::Invalid(error.to_string())
}
pub(super) fn market_query(selection: &MarketSelection) -> Result<String, ShareError> {
    let query = selection.to_query().map_err(invalid)?;
    let pairs = url::form_urlencoded::parse(query.as_bytes()).collect::<Vec<_>>();
    if pairs
        .iter()
        .any(|(_, value)| value.chars().any(char::is_whitespace))
    {
        return Ok(format!("market {query}"));
    }
    Ok(format!(
        "market {}",
        pairs
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(" ")
    ))
}
fn selection(target: &PortfolioTarget) -> MarketSelection {
    MarketSelection {
        filters: market_actors::input::GiftFilters {
            collection_id: target.filters.collection_id.clone(),
            currency_id: target.currency_id.clone(),
            venue_id: None,
            models: target.filters.models.clone(),
            backdrops: target.filters.backdrops.clone(),
            patterns: target.filters.patterns.clone(),
            number: target.filters.number.map(|n| GiftNumberRange {
                min: Some(n),
                max: Some(n),
            }),
            price: None,
        },
        source: OrderSource::Asks,
        bid_match: Default::default(),
        sort: None,
        period: None,
        gift: None,
    }
}
pub(in crate::commands) fn portfolio_params(
    target: &PortfolioTarget,
) -> Result<String, ShareError> {
    target.validate().map_err(invalid)?;
    let selection = selection(target);
    selection.validate().map_err(invalid)?;
    let mut params = url::form_urlencoded::Serializer::new(String::new());
    let sort = serde_json::to_value(target.sort).map_err(invalid)?;
    params
        .append_pair("currency", target.currency_id.as_str())
        .append_pair("f", &selection.filter_line())
        .append_pair("sort", sort.as_str().expect("portfolio sort"));
    Ok(params.finish())
}
pub(super) fn portfolio_query(target: &PortfolioTarget) -> Result<String, ShareError> {
    let params = portfolio_params(target)?;
    if target.filters == Default::default() {
        let sort = serde_json::to_value(target.sort).map_err(invalid)?;
        return Ok(format!(
            "portfolio {} {} sort={}",
            target.user_id.get(),
            target.currency_id,
            sort.as_str().unwrap()
        ));
    }
    Ok(format!("portfolio user={}&{params}", target.user_id.get()))
}
fn fallback(
    query: String,
    link: impl FnOnce() -> Result<url::Url, ShareError>,
) -> Result<ShareFallback, ShareError> {
    if query.chars().count() <= 256 {
        Ok(ShareFallback::Inline { query })
    } else {
        Ok(ShareFallback::Link { url: link()? })
    }
}
pub(in crate::commands) fn market_fallback(
    selection: &MarketSelection,
    origin: Option<&url::Url>,
) -> Result<ShareFallback, ShareError> {
    fallback(market_query(selection)?, || {
        selection
            .website_url(
                origin.ok_or_else(|| {
                    invalid("Configure a public app URL to share large selections")
                })?,
            )
            .map_err(invalid)
    })
}
pub(in crate::commands) fn portfolio_fallback(
    target: &PortfolioTarget,
    origin: Option<&url::Url>,
) -> Result<ShareFallback, ShareError> {
    fallback(portfolio_query(target)?, || {
        let origin = origin
            .ok_or_else(|| invalid("Configure a public app URL to share large selections"))?;
        let mut url = selection(target).website_url(origin).map_err(invalid)?;
        url.set_path(&format!(
            "{}/portfolio/{}",
            origin.path().trim_end_matches('/'),
            target.user_id.get()
        ));
        url.set_fragment(Some(&portfolio_params(target)?));
        Ok(url)
    })
}
pub(in crate::commands) fn with_share(mut reply: Reply, fallback: &ShareFallback) -> Reply {
    reply.buttons.iter_mut().for_each(|row| {
        row.retain(|button| {
            !matches!(button, ReplyButton::SwitchInline { .. })
                && !matches!(button,ReplyButton::Url { text,.. } if text=="Share")
        })
    });
    reply.buttons.retain(|row| !row.is_empty());
    let button = match fallback {
        ShareFallback::Inline { query } => ReplyButton::SwitchInline {
            text: "Share".into(),
            query: query.clone(),
        },
        ShareFallback::Link { url } => {
            let mut share = url::Url::parse("https://t.me/share/url").unwrap();
            share.query_pairs_mut().append_pair("url", url.as_str());
            ReplyButton::Url {
                text: "Share".into(),
                url: share,
            }
        }
    };
    reply.buttons.push(vec![button]);
    reply
}
pub(in crate::commands) fn parse_target(query: &str) -> Option<Command> {
    if let Some(args) = query.strip_prefix("market ") {
        let selection = if args.chars().any(char::is_whitespace) {
            browse::parse(query)?
        } else {
            MarketSelection::from_query(args).ok()?
        };
        if selection.filters.collection_id.is_none() {
            return None;
        }
        return Some(Command::Browse(selection));
    }
    let args = query.strip_prefix("portfolio user=")?;
    let mut params = std::collections::BTreeMap::new();
    let query = format!("user={args}");
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        if !["user", "currency", "f", "sort"].contains(&key.as_ref())
            || params
                .insert(key.into_owned(), value.into_owned())
                .is_some()
        {
            return None;
        }
    }
    let user_id = market_portfolio::UserId::new(params.remove("user")?.parse().ok()?).ok()?;
    let sort = portfolio::parse_sort(&params.remove("sort")?)?;
    let mut encoded = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in params {
        encoded.append_pair(&key, &value);
    }
    let selection = MarketSelection::from_query(&encoded.finish()).ok()?;
    let f = selection.filters;
    let number = if let Some(n) = f.number {
        if n.min.is_none() || n.min != n.max {
            return None;
        }
        n.min
    } else {
        None
    };
    let target = PortfolioTarget {
        user_id,
        currency_id: f.currency_id,
        sort,
        filters: market_portfolio::PortfolioFilters {
            collection_id: f.collection_id,
            models: f.models,
            backdrops: f.backdrops,
            patterns: f.patterns,
            number,
        },
    };
    target.validate().ok()?;
    Some(Command::PortfolioTarget(target))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn portfolio() -> PortfolioTarget {
        PortfolioTarget {
            user_id: market_portfolio::UserId::new(123).unwrap(),
            currency_id: "GRAM".into(),
            sort: market_portfolio::PortfolioSort::ValueDesc,
            filters: Default::default(),
        }
    }
    #[test]
    fn market_targets_round_trip_every_filter_without_a_snapshot_identity() {
        for input in [
            "/market collection=Gift model=Dark%20Knight backdrop=Black symbol=Sun%2BMoon currency=stars",
            "/market f=c=@slug:Gift|m=@name:Dark%20Knight|b=@name:Black currency=GRAM mode=bid bid_match=any sort=price_desc min=1 max=20",
            "/market f=c=@slug:Gift currency=GRAM mode=executed from=2026-01-01T00:00:00Z until=2026-02-01T00:00:00Z",
        ] {
            let selection = browse::parse(input).unwrap();
            let query = market_query(&selection).unwrap();
            let Some(Command::Browse(restored)) = parse_target(&query) else {
                panic!("{query}")
            };
            assert_eq!(restored, selection);
            assert!(!query.starts_with("s:"));
        }
    }
    #[test]
    fn filtered_portfolios_are_stateless_and_normal_queries_remain_readable() {
        let base = portfolio();
        let query = portfolio_query(&base).unwrap();
        let request = portfolio::inline(&query, 999).unwrap();
        assert_eq!(request.user_id, base.user_id);
        assert_eq!(request.sort, base.sort);
        let target = PortfolioTarget {
            filters: market_portfolio::PortfolioFilters {
                collection_id: Some("gift".into()),
                models: vec!["model".into()],
                backdrops: vec!["@name:Black".try_into().unwrap()],
                patterns: vec!["symbol".into()],
                number: Some(42.try_into().unwrap()),
            },
            ..base
        };
        let query = portfolio_query(&target).unwrap();
        let Some(Command::PortfolioTarget(restored)) = parse_target(&query) else {
            panic!("{query}")
        };
        assert_eq!(restored, target);
        assert!(!query.contains("version"));
        assert!(!query.contains("offset"));
        for extra in [
            "&version=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "&offset=30",
            "&user=456",
            "&f=",
        ] {
            assert!(parse_target(&format!("{query}{extra}")).is_none());
        }
    }
    #[test]
    fn large_selections_use_complete_stateless_urls_not_a_shortener() {
        let origin = url::Url::parse("https://app.example/base/").unwrap();
        let target = PortfolioTarget {
            filters: market_portfolio::PortfolioFilters {
                collection_id: Some("gift".into()),
                models: (0..100)
                    .map(|i| format!("model-{i}").try_into().unwrap())
                    .collect(),
                ..Default::default()
            },
            ..portfolio()
        };
        let ShareFallback::Link { url } = portfolio_fallback(&target, Some(&origin)).unwrap()
        else {
            panic!()
        };
        assert_eq!(url.path(), "/base/portfolio/123");
        let params = url::form_urlencoded::parse(url.fragment().unwrap().as_bytes())
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(params["f"], selection(&target).filter_line());
        assert_eq!(params["sort"], "value_desc");
        assert!(portfolio_fallback(&target, None).is_err());
        let chosen = selection(&target);
        let ShareFallback::Link { url } = market_fallback(&chosen, Some(&origin)).unwrap() else {
            panic!()
        };
        assert_eq!(
            MarketSelection::from_query(url.fragment().unwrap()).unwrap(),
            chosen
        );
    }
    #[test]
    fn share_buttons_never_retain_expired_payloads() {
        let reply = Reply {
            buttons: vec![vec![
                ReplyButton::SwitchInline {
                    text: "Share".into(),
                    query: "s:old".into(),
                },
                ReplyButton::Preview {
                    text: "Open".into(),
                    payload: "v_target".into(),
                },
            ]],
            ..Reply::text("Report")
        };
        let inline = ShareFallback::Inline {
            query: "Gift-123 GRAM".into(),
        };
        let updated = with_share(reply, &inline);
        assert!(
            updated.buttons.iter().flatten().any(
                |b| matches!(b,ReplyButton::SwitchInline { query,.. } if query=="Gift-123 GRAM")
            )
        );
        assert!(
            updated
                .buttons
                .iter()
                .flatten()
                .any(|b| matches!(b,ReplyButton::Preview { payload,.. } if payload=="v_target"))
        );
        let url = url::Url::parse("https://app.example/#f=c%3Dgift").unwrap();
        let updated = with_share(updated, &ShareFallback::Link { url: url.clone() });
        assert!(
            !updated
                .buttons
                .iter()
                .flatten()
                .any(|b| matches!(b, ReplyButton::SwitchInline { .. }))
        );
        let ReplyButton::Url { url: share, .. } = &updated.buttons.last().unwrap()[0] else {
            panic!()
        };
        assert_eq!(
            share.query_pairs().find(|(k, _)| k == "url").unwrap().1,
            url.as_str()
        );
    }
}
