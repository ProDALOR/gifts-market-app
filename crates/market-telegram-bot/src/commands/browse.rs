//! Small market search command; all catalog interpretation remains in the data layer.
use super::*;
use market_actors::input::GiftFilters;
use market_gifts::*;
use market_links::MarketSelection;
pub(super) fn parse(text: &str) -> Option<MarketSelection> {
    let args = text.split_once(char::is_whitespace)?.1.trim();
    if args.starts_with("f=") {
        let mut params = url::form_urlencoded::Serializer::new(String::new());
        for part in args.split_whitespace() {
            let (key, value) = part.split_once('=')?;
            if ![
                "f",
                "currency",
                "venue",
                "mode",
                "bid_match",
                "sort",
                "min",
                "max",
                "from",
                "until",
            ]
            .contains(&key)
            {
                return None;
            }
            params.append_pair(key, value);
        }
        return MarketSelection::from_query(&params.finish()).ok();
    }
    if ["c=", "m=", "b=", "s=", "n="]
        .iter()
        .any(|prefix| args.starts_with(prefix))
    {
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("f", args)
            .finish();
        return MarketSelection::from_query(&query).ok();
    }
    let mut filters = GiftFilters::new("unused".into(), "GRAM".into());
    filters.collection_id = None;
    let mut seen = std::collections::BTreeSet::new();
    for part in text.split_whitespace().skip(1) {
        let (key, value) = part.split_once('=')?;
        if !seen.insert(key) {
            return None;
        }
        let value = url::form_urlencoded::parse(format!("v={value}").as_bytes())
            .next()?
            .1
            .into_owned();
        if value.is_empty() {
            return None;
        }
        let selector = |namespace: &str| {
            if value.starts_with('@') {
                Some(value.clone())
            } else {
                GiftAlias::new(namespace, value.clone())
                    .ok()
                    .map(|a| a.selector())
            }
        };
        match key {
            "collection" => {
                if value != "all" {
                    filters.collection_id = Some(selector("slug")?.try_into().ok()?);
                }
            }
            "model" => filters.models.push(selector("name")?.try_into().ok()?),
            "backdrop" => filters.backdrops.push(selector("name")?.try_into().ok()?),
            "symbol" => filters.patterns.push(selector("name")?.try_into().ok()?),
            "currency" => filters.currency_id = value.try_into().ok()?,
            _ => return None,
        }
    }
    if filters.collection_id.is_none() && !filters.models.is_empty() {
        return None;
    }
    Some(MarketSelection::new(filters))
}
fn market_button(
    selection: &MarketSelection,
    public_url: Option<&url::Url>,
) -> Result<market_telegram::ReplyButton, market_links::LinkError> {
    if let Some(payload) = selection.launch_payload()? {
        return Ok(market_telegram::ReplyButton::Preview {
            text: "Open market".into(),
            payload,
        });
    }
    let origin = public_url.ok_or(market_links::LinkError::Invalid(
        "A public app URL is required to open this selection",
    ))?;
    Ok(market_telegram::ReplyButton::Url {
        text: "Open market".into(),
        url: selection.website_url(origin)?,
    })
}
impl MarketCommands {
    pub(super) async fn browse(&self, selection: MarketSelection) -> Reply {
        let button = match market_button(&selection, self.public_url.as_ref()) {
            Ok(payload) => payload,
            Err(error) => return render::error_reply(error),
        };
        let mut query = selection.order_query();
        query.limit = 5;
        let report = match self.market.query_orders(query).await {
            Ok(v) => v,
            Err(e) => return render::error_reply(e),
        };
        let catalog = match self.market.catalog_snapshot().await {
            Ok(v) => v.data,
            Err(e) => return render::error_reply(e),
        };
        let mut text = String::from(match selection.source {
            OrderSource::Asks => "Best asks",
            OrderSource::Bids => "Best bids",
            OrderSource::History => "Recent sales",
        });
        for row in &report.result.data {
            let order = &row.order;
            let collection = catalog
                .collections
                .iter()
                .find(|c| c.id == order.book.collection_id);
            let name = collection
                .map(|c| c.name.as_str())
                .unwrap_or(order.book.collection_id.as_str());
            let number = match &order.target {
                GiftTarget::Gift { gift } => gift
                    .number
                    .map(|n| format!(" #{}", n.get()))
                    .unwrap_or_default(),
                _ => String::new(),
            };
            text.push_str(&format!(
                "\n{name}{number} · {} {}",
                super::report::amount(&order.price),
                order.book.currency_id
            ));
        }
        if report.result.data.is_empty() {
            text.push_str("\nNo matching orders.");
        }
        let buttons = vec![vec![button]];
        Reply {
            text,
            buttons,
            ..Default::default()
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn readable_market_commands_preserve_alternatives_and_controls() {
        let selection = parse("/market f=c=@slug:Gift|m=@name:A,@name:B|b=@name:Black%7CWhite|s=@name:Crown|n=10..99 currency=XTR mode=bid bid_match=any sort=price_desc min=1.20").unwrap();
        assert_eq!(selection.filters.models.len(), 2);
        assert_eq!(
            selection.filters.backdrops[0].to_string(),
            "@name:Black|White"
        );
        assert_eq!(selection.filters.currency_id.as_str(), "XTR");
        assert_eq!(selection.source, OrderSource::Bids);
        assert_eq!(selection.bid_match, BidMatch::Any);
        assert_eq!(selection.sort, Some(GiftSort::PriceDesc));
        assert!(parse("/market c=@slug:Gift|m=@name:A,@name:B").is_some());
        assert!(parse("/market f=c=Gift|m=A f=c=Other").is_none());
        assert!(parse("/market c=Gift|m=A,").is_none());
    }
    #[test]
    fn buttons_keep_large_selections_without_storage_or_default_view_fallback() {
        let mut selection = parse("/market collection=Gift").unwrap();
        assert!(matches!(
            market_button(&selection, None).unwrap(),
            market_telegram::ReplyButton::Preview { .. }
        ));
        selection.filters.models = (0..300)
            .map(|i| format!("@name:Model {i}").try_into().unwrap())
            .collect();
        let origin = url::Url::parse("https://market.example/").unwrap();
        let market_telegram::ReplyButton::Url { url, .. } =
            market_button(&selection, Some(&origin)).unwrap()
        else {
            panic!("large selections require a complete URL")
        };
        assert!(url.query().is_none());
        assert_eq!(
            MarketSelection::from_query(url.fragment().unwrap()).unwrap(),
            selection
        );
        assert!(market_button(&selection, None).is_err());
    }
    #[test]
    fn market_search_uses_scoped_model_and_explicit_alias_selectors() {
        let selection =
            parse("/market collection=ResistanceDog model=Dogfather backdrop=Black symbol=Crown")
                .unwrap();
        let filters = selection.filters;
        assert_eq!(
            filters.collection_id.unwrap().to_string(),
            "@slug:ResistanceDog"
        );
        assert_eq!(filters.models[0].to_string(), "@name:Dogfather");
        assert!(parse("/market model=Dogfather").is_none());
        assert!(
            parse("/market backdrop=@tg:42")
                .unwrap()
                .filters
                .collection_id
                .is_none()
        );
        assert!(parse("/market collection=One collection=Two").is_none());
        assert!(parse("/market symbol=@broken").is_none());
        assert_eq!(
            parse("/market collection=One model=Dark%20Knight")
                .unwrap()
                .filters
                .models[0]
                .to_string(),
            "@name:Dark Knight"
        );
    }
}
