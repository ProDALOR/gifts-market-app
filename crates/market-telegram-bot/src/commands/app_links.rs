//! Stateless return links. Small selections open the Mini App; large ones use its website.
use super::*;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use market_gifts::{GiftNumberRange, OrderSource};
use market_portfolio::ResolvedPortfolioFilters;
use market_telegram::ReplyButton;

pub(super) fn portfolio_button(
    request: &PortfolioRequest,
    filters: &ResolvedPortfolioFilters,
    public_url: Option<&url::Url>,
) -> Result<ReplyButton, market_links::LinkError> {
    let selection = MarketSelection {
        filters: market_actors::input::GiftFilters {
            collection_id: filters.collection_id.clone().map(Into::into),
            currency_id: request.currency_id.id(),
            venue_id: None,
            models: filters.models.iter().cloned().map(Into::into).collect(),
            backdrops: filters.backdrops.iter().cloned().map(Into::into).collect(),
            patterns: filters.patterns.iter().cloned().map(Into::into).collect(),
            number: filters.number.map(|n| GiftNumberRange {
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
    };
    selection.validate()?;
    let sort = serde_json::to_value(request.sort).expect("portfolio sort serializes");
    let mut params = url::form_urlencoded::Serializer::new(String::new());
    params
        .append_pair("currency", request.currency_id.as_str())
        .append_pair("f", &selection.filter_line())
        .append_pair("sort", sort.as_str().expect("portfolio sort is a string"));
    let query = params.finish();
    if query.len() > market_links::MAX_SELECTION_BYTES {
        return Err(market_links::LinkError::TooLarge);
    }
    let launch_query = format!("user={}&{query}", request.user_id.get());
    if launch_query.len() <= (market_links::LAUNCH_LIMIT - 2) * 3 / 4 {
        return Ok(ReplyButton::Preview {
            text: "Open portfolio".into(),
            payload: format!("p_{}", URL_SAFE_NO_PAD.encode(launch_query)),
        });
    }
    let origin = public_url.ok_or(market_links::LinkError::Invalid(
        "A public app URL is required to open this selection",
    ))?;
    let mut url = selection.website_url(origin)?;
    url.set_path(&format!(
        "{}/portfolio/{}",
        origin.path().trim_end_matches('/'),
        request.user_id.get()
    ));
    url.set_fragment(Some(&query));
    Ok(ReplyButton::Url {
        text: "Open portfolio".into(),
        url,
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
        let ReplyButton::Preview { payload, .. } =
            portfolio_button(&request, &filters, None).unwrap()
        else {
            panic!()
        };
        let decoded = String::from_utf8(
            URL_SAFE_NO_PAD
                .decode(payload.strip_prefix("p_").unwrap())
                .unwrap(),
        )
        .unwrap();
        let params = url::form_urlencoded::parse(decoded.as_bytes())
            .into_owned()
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(params["user"], "123");
        assert_eq!(params["currency"], "XTR");
        assert_eq!(params["sort"], "number_desc");
        assert_eq!(params["f"], "c=gift|m=model|b=black|s=sun|n=42");
        assert!(!params.contains_key("version"));
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
