//! Small market search command; all catalog interpretation remains in the data layer.
use super::*;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use market_actors::input::{GiftFilters, GiftOrderQuery};
use market_gifts::*;
pub(super) fn parse(text: &str) -> Option<GiftFilters> {
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
    Some(filters)
}
impl MarketCommands {
    pub(super) async fn browse(&self, filters: GiftFilters) -> Reply {
        let mut query = GiftOrderQuery::new(filters.clone());
        query.limit = 5;
        let report = match self.market.query_orders(query).await {
            Ok(v) => v,
            Err(e) => return render::error_reply(e),
        };
        let catalog = match self.market.catalog_snapshot().await {
            Ok(v) => v.data,
            Err(e) => return render::error_reply(e),
        };
        let mut text = String::from("Best asks");
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
            text.push_str("\nNo matching asks.");
        }
        let mut params = url::form_urlencoded::Serializer::new(String::new());
        if let Some(collection) = &filters.collection_id {
            params.append_pair("collection", &collection.to_string());
        }
        params.append_pair("currency", filters.currency_id.as_str());
        let traits = serde_json::json!({"models":filters.models,"backdrops":filters.backdrops,"patterns":filters.patterns});
        params.append_pair("filters", &traits.to_string());
        let payload = format!("v_{}", URL_SAFE_NO_PAD.encode(params.finish()));
        let buttons = if payload.len() <= 512 {
            vec![vec![market_telegram::ReplyButton::Preview {
                text: "Open market".into(),
                payload,
            }]]
        } else {
            vec![]
        };
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
    fn market_search_uses_scoped_model_and_explicit_alias_selectors() {
        let filters =
            parse("/market collection=ResistanceDog model=Dogfather backdrop=Black symbol=Crown")
                .unwrap();
        assert_eq!(
            filters.collection_id.unwrap().to_string(),
            "@slug:ResistanceDog"
        );
        assert_eq!(filters.models[0].to_string(), "@name:Dogfather");
        assert!(parse("/market model=Dogfather").is_none());
        assert!(
            parse("/market backdrop=@tg:42")
                .unwrap()
                .collection_id
                .is_none()
        );
        assert!(parse("/market collection=One collection=Two").is_none());
        assert!(parse("/market symbol=@broken").is_none());
        assert_eq!(
            parse("/market collection=One model=Dark%20Knight")
                .unwrap()
                .models[0]
                .to_string(),
            "@name:Dark Knight"
        );
    }
}
