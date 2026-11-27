//! Stateless return links. Small selections open the Mini App; large ones use its website.
use super::*;
use market_portfolio::ResolvedPortfolioFilters;
use market_telegram::ReplyButton;

pub(super) fn portfolio_button(
    request: &PortfolioRequest,
    filters: &ResolvedPortfolioFilters,
    origin: Option<&url::Url>,
) -> Result<ReplyButton, market_links::LinkError> {
    let target = market_links::Target::Portfolio {
        target: sharing::PortfolioTarget {
            user_id: request.user_id,
            currency_id: request.currency_id.id(),
            sort: request.sort,
            filters: market_gifts::PortfolioFilters {
                collection_id: filters.collection_id.clone().map(Into::into),
                models: filters.models.iter().cloned().map(Into::into).collect(),
                backdrops: filters.backdrops.iter().cloned().map(Into::into).collect(),
                patterns: filters.patterns.iter().cloned().map(Into::into).collect(),
                number: filters.number,
            },
        },
    };
    button("Open portfolio", &target, origin)
}

/// All report types use the same launch-size and complete-URL fallback policy.
pub(super) fn button(
    text: &str,
    target: &market_links::Target,
    origin: Option<&url::Url>,
) -> Result<ReplyButton, market_links::LinkError> {
    if let Some(payload) = target.launch(market_links::LAUNCH_LIMIT)? {
        return Ok(ReplyButton::OpenApp {
            text: text.into(),
            payload,
        });
    }
    Ok(ReplyButton::Url {
        text: text.into(),
        url: target.website_url(origin.ok_or(market_links::LinkError::Invalid(
            "A public app URL is required to open this selection",
        ))?)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn portfolio_links_preserve_currency_sort_and_traits_without_persisting_versions() {
        let request = PortfolioRequest {
            user_id: market_portfolio::UserId::new(123).unwrap(),
            currency_id: BotCurrency::Stars,
            sort: market_portfolio::PortfolioSort::NumberDesc,
            page: 2,
            mode: PortfolioMode::Inline,
            action: PortfolioAction::Navigate,
            layout: GridMode::Linked,
        };
        let filters = ResolvedPortfolioFilters {
            collection_id: Some("gift".into()),
            models: vec!["model".into()],
            backdrops: vec!["black".into()],
            patterns: vec!["sun".into()],
            number: Some(42u32.try_into().unwrap()),
        };
        let ReplyButton::OpenApp { payload, .. } =
            portfolio_button(&request, &filters, None).unwrap()
        else {
            panic!()
        };
        let market_links::Target::Portfolio { target } =
            market_links::Target::from_launch(&payload).unwrap()
        else {
            panic!()
        };
        assert_eq!(target.user_id.get(), 123);
        assert_eq!(target.currency_id.as_str(), "XTR");
        assert_eq!(target.sort, request.sort);
        assert_eq!(target.filters.models[0].to_string(), "model");
        assert_eq!(target.filters.number, filters.number);
        let large = ResolvedPortfolioFilters {
            models: (0..100)
                .map(|n| format!("model-{n}").try_into().unwrap())
                .collect(),
            ..filters
        };
        let origin = url::Url::parse("https://app.example/").unwrap();
        let ReplyButton::Url { url, .. } =
            portfolio_button(&request, &large, Some(&origin)).unwrap()
        else {
            panic!()
        };
        assert_eq!(url.path(), "/portfolio/123");
        assert!(url.fragment().unwrap().contains("sort=number_desc"));
        assert!(portfolio_button(&request, &large, None).is_err());
    }
}
