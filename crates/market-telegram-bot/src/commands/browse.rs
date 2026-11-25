//! Small market search command; all catalog interpretation remains in the data layer.
use super::orders;
use super::*;
use market_gifts::*;
use market_links::MarketSelection;
pub(super) fn parse(text: &str) -> Option<MarketSelection> {
    use market_actors::input::CatalogSelector;
    fn public_reference<I>(reference: &CatalogSelector<I>) -> bool {
        matches!(reference, CatalogSelector::Ref(_))
    }
    let mut selection = parse_selection(text)?;
    selection.filters.currency_id =
        BotCurrency::parse(selection.filters.currency_id.as_str())?.id();
    let filters = &selection.filters;
    (filters.collection_id.as_ref().is_none_or(public_reference)
        && filters.models.iter().all(public_reference)
        && filters.backdrops.iter().all(public_reference)
        && filters.patterns.iter().all(public_reference))
    .then_some(selection)
}
fn parse_selection(text: &str) -> Option<MarketSelection> {
    let args = text.split_once(char::is_whitespace)?.1.trim();
    let short = ["c=", "m=", "b=", "s=", "n="]
        .iter()
        .any(|prefix| args.starts_with(prefix));
    if args.starts_with("f=") || short {
        let mut params = url::form_urlencoded::Serializer::new(String::new());
        for (i, part) in args.split_whitespace().enumerate() {
            let (key, value) = if i == 0 && short {
                ("f", part)
            } else {
                part.split_once('=')?
            };
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
    // Translate named arguments into the shared readable-link codec. Existing
    // command and URL spellings retain the same alias semantics.
    let mut groups = Vec::new();
    let mut params = url::form_urlencoded::Serializer::new(String::new());
    let mut seen = std::collections::BTreeSet::new();
    for part in args.split_whitespace() {
        let (key, raw) = part.split_once('=')?;
        if !seen.insert(key) {
            return None;
        }
        let value = url::form_urlencoded::parse(format!("v={raw}").as_bytes())
            .next()?
            .1
            .into_owned();
        if value.is_empty() {
            return None;
        }
        let (group, namespace) = match key {
            "collection" if value == "all" => continue,
            "collection" => ("c", "slug"),
            "model" => ("m", "name"),
            "backdrop" => ("b", "name"),
            "symbol" => ("s", "name"),
            "currency" => {
                params.append_pair("currency", BotCurrency::parse(&value)?.as_str());
                continue;
            }
            _ => return None,
        };
        let reference = if value.starts_with('@') {
            value
        } else {
            GiftAlias::new(namespace, value).ok()?.selector()
        };
        let escaped = url::form_urlencoded::byte_serialize(reference.as_bytes())
            .collect::<String>()
            .replace('+', "%20");
        groups.push(format!("{group}={escaped}"));
    }
    params.append_pair("f", &groups.join("|"));
    MarketSelection::from_query(&params.finish()).ok()
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
mod pager;
pub(super) use pager::View;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_named_arguments_and_readable_links_share_validation_and_currency_normalization() {
        let named = parse("/market collection=Gift model=Dark%20Knight backdrop=Black%7CWhite symbol=Sun%2BMoon currency=stars").unwrap();
        let readable = parse("/market f=c=@slug:Gift|m=@name:Dark%20Knight|b=@name:Black%7CWhite|s=@name:Sun%2BMoon currency=xtr").unwrap();
        assert_eq!(named, readable);
        assert_eq!(parse("/market c=@slug:Gift|m=@name:Dark%20Knight|b=@name:Black%7CWhite|s=@name:Sun%2BMoon currency=STARS").unwrap(), named);
        for spelling in ["GRAM", "gram", "TON", "Ton"] {
            assert_eq!(
                parse(&format!("/market f=c=@slug:Gift currency={spelling}"))
                    .unwrap()
                    .filters
                    .currency_id
                    .as_str(),
                "GRAM"
            );
        }
        for command in [
            "/market collection=Gift currency=USD",
            "/market f=c=@slug:Gift currency=GRAM:bad",
            "/market currency=sort=price_desc",
            "/market collection=Gift model=Dark%20Knight symbol=@tg:42",
        ] {
            assert!(parse(command).is_none(), "{command}");
        }
        let url = named
            .website_url(&url::Url::parse("https://market.example/").unwrap())
            .unwrap();
        assert_eq!(
            MarketSelection::from_query(url.fragment().or(url.query()).unwrap()).unwrap(),
            named
        );
    }
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
        assert!(parse("/market backdrop=@name:Black").is_some());
        for command in [
            "/market backdrop=@tg:42",
            "/market collection=@tg.slug:Gift",
            "/market collection=Gift model=@tg:42",
            "/market symbol=@partner:42",
            "/market c=@slug:Gift|b=@tg:42",
            "/market f=c=@slug:Gift|s=@tg:42 currency=GRAM",
        ] {
            assert!(parse(command).is_none(), "{command}");
        }
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
