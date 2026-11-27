//! Public navigation targets. No actor, session, report-cache or Telegram-client dependency.
use crate::*;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use market_gifts::*;
use market_gifts_hub::input::{
    BackdropRef, CatalogSelector, CollectionRef, GiftFilters, ModelRef, PatternRef,
};
use serde::{Deserialize, Serialize};
pub type PortfolioLink = PortfolioTarget<CollectionRef, ModelRef, BackdropRef, PatternRef>;
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Target {
    Gift {
        slug: GiftSlug,
        currency_id: CurrencyId,
    },
    Market {
        selection: MarketSelection,
    },
    Portfolio {
        target: PortfolioLink,
    },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShareDestination {
    Inline { query: String },
    Link { url: url::Url },
}
fn invalid() -> LinkError {
    LinkError::Invalid("Invalid navigation target")
}
fn currency(text: &str) -> Result<CurrencyId> {
    let text = match text.to_ascii_uppercase().as_str() {
        "TON" => "GRAM".to_owned(),
        "STARS" => "XTR".to_owned(),
        _ => text.to_ascii_uppercase(),
    };
    CurrencyId::new(text).map_err(|_| invalid())
}
impl Target {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Gift { .. } => Ok(()),
            Self::Market { selection } => selection.validate(),
            Self::Portfolio { target } => target.validate().map_err(|_| invalid()),
        }
    }
    fn portfolio_selection(target: &PortfolioLink) -> MarketSelection {
        MarketSelection {
            filters: GiftFilters {
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
            bid_match: BidMatch::All,
            sort: None,
            period: None,
            gift: None,
        }
    }
    pub fn query(&self) -> Result<String> {
        self.validate()?;
        let mut params = url::form_urlencoded::Serializer::new(String::new());
        match self {
            Self::Gift { slug, currency_id } => {
                params
                    .append_pair("t", "gift")
                    .append_pair("gift", &slug.to_string())
                    .append_pair("currency", currency_id.as_str());
            }
            Self::Market { selection } => {
                params.append_pair("t", "market");
                for (k, v) in url::form_urlencoded::parse(selection.to_query()?.as_bytes()) {
                    params.append_pair(&k, &v);
                }
            }
            Self::Portfolio { target } => {
                let selection = Self::portfolio_selection(target);
                selection.validate()?;
                let sort = serde_json::to_value(target.sort).map_err(|_| invalid())?;
                params
                    .append_pair("t", "portfolio")
                    .append_pair("user", &target.user_id.get().to_string())
                    .append_pair("currency", target.currency_id.as_str())
                    .append_pair("f", &selection.filter_line())
                    .append_pair("sort", sort.as_str().unwrap());
            }
        }
        let query = params.finish();
        if query.len() > MAX_SELECTION_BYTES {
            Err(LinkError::TooLarge)
        } else {
            Ok(query)
        }
    }
    pub fn from_query(query: &str) -> Result<Self> {
        if query.len() > MAX_SELECTION_BYTES {
            return Err(LinkError::TooLarge);
        }
        let mut params = std::collections::BTreeMap::new();
        for (k, v) in url::form_urlencoded::parse(query.trim_start_matches('?').as_bytes()) {
            if params.insert(k.into_owned(), v.into_owned()).is_some() {
                return Err(invalid());
            }
        }
        let kind = params.remove("t").unwrap_or_else(|| {
            if params.contains_key("user") {
                "portfolio"
            } else if params.contains_key("gift")
                && !params.get("f").is_some_and(|v| !v.is_empty())
                && ![
                    "collection",
                    "filters",
                    "mode",
                    "bid_match",
                    "sort",
                    "venue",
                    "min",
                    "max",
                    "from",
                    "until",
                ]
                .iter()
                .any(|k| params.contains_key(*k))
            {
                "gift"
            } else {
                "market"
            }
            .to_owned()
        });
        if let Some(c) = params.get_mut("currency") {
            *c = currency(c)?.to_string();
        }
        const MARKET_KEYS: &[&str] = &[
            "gift",
            "currency",
            "f",
            "collection",
            "mode",
            "bid_match",
            "sort",
            "venue",
            "filters",
            "min",
            "max",
            "from",
            "until",
        ];
        if matches!(kind.as_str(), "gift" | "market")
            && params.keys().any(|k| !MARKET_KEYS.contains(&k.as_str()))
        {
            return Err(invalid());
        }
        let target = match kind.as_str() {
            "gift" | "market" => {
                let mut p = url::form_urlencoded::Serializer::new(String::new());
                for (k, v) in params {
                    p.append_pair(&k, &v);
                }
                let selection = MarketSelection::from_query(&p.finish())?;
                if kind == "gift" {
                    Self::Gift {
                        slug: selection.gift.ok_or_else(invalid)?,
                        currency_id: selection.filters.currency_id,
                    }
                } else {
                    Self::Market { selection }
                }
            }
            "portfolio" => {
                if params.keys().any(|k| {
                    !["user", "currency", "f", "sort", "collection", "filters"]
                        .contains(&k.as_str())
                }) {
                    return Err(invalid());
                }
                let user = params.remove("user").ok_or_else(invalid)?;
                if user.starts_with('0') || !user.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(invalid());
                }
                let user_id =
                    UserId::new(user.parse().map_err(|_| invalid())?).map_err(|_| invalid())?;
                let sort = match params.remove("sort").as_deref().unwrap_or("position_asc") {
                    "price_asc" | "value_asc" => PortfolioSort::ValueAsc,
                    "price_desc" | "value_desc" => PortfolioSort::ValueDesc,
                    "position_asc" => PortfolioSort::PositionAsc,
                    "position_desc" => PortfolioSort::PositionDesc,
                    "number_asc" => PortfolioSort::NumberAsc,
                    "number_desc" => PortfolioSort::NumberDesc,
                    _ => return Err(invalid()),
                };
                let mut p = url::form_urlencoded::Serializer::new(String::new());
                for (k, v) in params {
                    p.append_pair(&k, &v);
                }
                let f = MarketSelection::from_query(&p.finish())?.filters;
                let number = if let Some(n) = f.number {
                    if n.min.is_none() || n.min != n.max {
                        return Err(invalid());
                    }
                    n.min
                } else {
                    None
                };
                Self::Portfolio {
                    target: PortfolioLink {
                        user_id,
                        currency_id: f.currency_id,
                        sort,
                        filters: PortfolioFilters {
                            collection_id: f.collection_id,
                            models: f.models,
                            backdrops: f.backdrops,
                            patterns: f.patterns,
                            number,
                        },
                    },
                }
            }
            _ => return Err(invalid()),
        };
        target.validate()?;
        Ok(target)
    }
    pub fn launch(&self, limit: usize) -> Result<Option<String>> {
        let query = self.query()?;
        if query.len().saturating_mul(4).div_ceil(3).saturating_add(2) > limit {
            return Ok(None);
        }
        let payload = format!("t_{}", URL_SAFE_NO_PAD.encode(query));
        Ok((payload.len() <= limit).then_some(payload))
    }
    pub fn from_launch(payload: &str) -> Result<Self> {
        if payload.len() > LAUNCH_LIMIT {
            return Err(LinkError::TooLarge);
        }
        if let Some(user) = payload.strip_prefix("portfolio_") {
            let user = UserId::new(user.parse().map_err(|_| invalid())?).map_err(|_| invalid())?;
            return Self::from_query(&format!("t=portfolio&user={}", user.get()));
        }
        let data = payload
            .strip_prefix("t_")
            .or_else(|| payload.strip_prefix("v_"))
            .or_else(|| payload.strip_prefix("p_"))
            .ok_or_else(invalid)?;
        let bytes = URL_SAFE_NO_PAD.decode(data).map_err(|_| invalid())?;
        {
            let query = std::str::from_utf8(&bytes).map_err(|_| invalid())?;
            if payload.starts_with("v_") {
                Self::from_query(&format!("t=market&{query}"))
            } else if payload.starts_with("p_") {
                Self::from_query(&format!("t=portfolio&{query}"))
            } else {
                if !url::form_urlencoded::parse(query.as_bytes()).any(|(k, _)| k == "t") {
                    return Err(invalid());
                }
                Self::from_query(query)
            }
        }
    }
    pub fn website_url(&self, origin: &url::Url) -> Result<url::Url> {
        let mut url = crate::codec::website_base(origin)?;
        let query = self.query()?;
        let mut p = url::form_urlencoded::parse(query.as_bytes())
            .filter(|(k, _)| k != "t")
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect::<Vec<_>>();
        if let Self::Portfolio { target } = self {
            url.set_path(&format!(
                "{}/portfolio/{}",
                origin.path().trim_end_matches('/'),
                target.user_id.get()
            ));
            p.retain(|(k, _)| k != "user");
        }
        let mut params = url::form_urlencoded::Serializer::new(String::new());
        for (k, v) in p {
            params.append_pair(&k, &v);
        }
        if matches!(self, Self::Gift { .. }) {
            params.append_pair("f", "");
        }
        url.set_query(None);
        url.set_fragment(Some(&params.finish()));
        Ok(url)
    }
    pub fn from_url(url: &url::Url) -> Result<Self> {
        if !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(invalid());
        }
        if url.query().map_or(0, str::len) + url.fragment().map_or(0, str::len)
            > MAX_SELECTION_BYTES
        {
            return Err(LinkError::TooLarge);
        }
        const KEYS: &[&str] = &[
            "t",
            "user",
            "currency",
            "f",
            "sort",
            "gift",
            "collection",
            "mode",
            "bid_match",
            "venue",
            "filters",
            "min",
            "max",
            "from",
            "until",
        ];
        fn source(value: Option<&str>) -> Result<std::collections::BTreeMap<String, String>> {
            let mut p = std::collections::BTreeMap::new();
            for (k, v) in url::form_urlencoded::parse(value.unwrap_or("").as_bytes()) {
                if KEYS.contains(&k.as_ref()) && p.insert(k.into_owned(), v.into_owned()).is_some()
                {
                    return Err(invalid());
                }
            }
            Ok(p)
        }
        let mut params = source(url.query())?;
        let fragment = source(url.fragment())?;
        let attributes = ["f", "collection", "filters"]
            .iter()
            .any(|k| params.contains_key(*k));
        if fragment.contains_key("f") || fragment.contains_key("gift") {
            for (k, v) in fragment {
                if !params.contains_key(&k)
                    && !(attributes && ["f", "collection", "filters"].contains(&k.as_str()))
                {
                    params.insert(k, v);
                }
            }
        }
        let path_portfolio = url
            .path()
            .trim_end_matches('/')
            .rsplit_once("/portfolio/")
            .map(|(_, user)| user);
        let explicit_market = url.query_pairs().any(|(k, v)| k == "page" && v == "market")
            || ["f", "collection", "filters", "gift"]
                .iter()
                .any(|k| params.contains_key(*k));
        if path_portfolio.is_none()
            && !explicit_market
            && let Some((_, payload)) = url.query_pairs().find(|(k, _)| k == "tgWebAppStartParam")
        {
            let launch = Self::from_launch(&payload)?;
            for (k, v) in url::form_urlencoded::parse(launch.query()?.as_bytes()) {
                params
                    .entry(k.into_owned())
                    .or_insert_with(|| v.into_owned());
            }
        }
        if let Some(user) = path_portfolio {
            params.insert("t".into(), "portfolio".into());
            params.insert("user".into(), user.into());
        }
        let mut encoded = url::form_urlencoded::Serializer::new(String::new());
        for (k, v) in params {
            encoded.append_pair(&k, &v);
        }
        Self::from_query(&encoded.finish())
    }
    pub fn inline_query(&self) -> Result<String> {
        let query = self.query()?;
        if let Self::Gift { slug, currency_id } = self {
            return Ok(format!("{slug} {currency_id}"));
        }
        let (name, params) = match self {
            Self::Market { .. } => ("market", query),
            Self::Portfolio { .. } => ("portfolio", query),
            _ => unreachable!(),
        };
        let pairs = url::form_urlencoded::parse(params.as_bytes())
            .filter(|(k, _)| k != "t")
            .collect::<Vec<_>>();
        Ok(format!(
            "{name} {}",
            pairs
                .iter()
                .map(|(k, v)| {
                    let value = if k == "f" {
                        v.to_string()
                    } else {
                        let mut encoded = url::form_urlencoded::Serializer::new(String::new());
                        encoded.append_pair("", v);
                        encoded.finish()[1..].to_owned()
                    };
                    format!("{k}={value}")
                })
                .collect::<Vec<_>>()
                .join(" ")
        ))
    }
    pub fn share(&self, origin: Option<&url::Url>) -> Result<ShareDestination> {
        let query = self.inline_query()?;
        if query.chars().count() <= 256 {
            Ok(ShareDestination::Inline { query })
        } else {
            let url = self.website_url(origin.ok_or(LinkError::Invalid(
                "Configure a public app URL to share this selection",
            ))?)?;
            if url.as_str().chars().count() > 4096 {
                return Err(LinkError::Invalid(
                    "Select fewer attributes to share this selection in Telegram",
                ));
            }
            Ok(ShareDestination::Link { url })
        }
    }
    pub fn bot_url(&self, username: &str, origin: Option<&url::Url>) -> Result<url::Url> {
        if username.is_empty()
            || !username
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return Err(invalid());
        }
        let mut url =
            url::Url::parse(&format!("https://t.me/{username}")).map_err(|_| invalid())?;
        if let Some(payload) = self.launch(64)? {
            url.query_pairs_mut().append_pair("start", &payload);
            return Ok(url);
        }
        let text = self.inline_query()?;
        if text.chars().count() <= 4096 {
            url.query_pairs_mut().append_pair("text", &text);
            return Ok(url);
        }
        self.website_url(origin.ok_or(LinkError::TooLarge)?)
    }
    /// Decode a copied bot launch or draft link. Identity allowlisting belongs to the caller.
    pub fn from_bot_url(url: &url::Url, user: u64) -> Result<Self> {
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str() != Some("t.me")
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(invalid());
        }
        let mut parameters = url
            .query_pairs()
            .filter(|(key, _)| matches!(key.as_ref(), "start" | "startapp" | "text"));
        let (kind, value) = parameters.next().ok_or_else(invalid)?;
        if parameters.next().is_some() {
            return Err(invalid());
        }
        match kind.as_ref() {
            "text" if value.chars().count() <= 4096 => Self::parse_text(&value, user),
            "start" if value.len() <= 64 => Self::from_launch(&value),
            "startapp" => Self::from_launch(&value),
            _ => Err(invalid()),
        }
    }
    pub fn parse_text(text: &str, user: u64) -> Result<Self> {
        let text = text.trim();
        if text.len() > MAX_SELECTION_BYTES {
            return Err(LinkError::TooLarge);
        }
        let (name, args) = text.split_once(char::is_whitespace).unwrap_or((text, ""));
        if !matches!(name, "market" | "/market" | "portfolio" | "/portfolio") {
            let mut p = text.split_whitespace();
            let slug = GiftSlug::new(p.next().ok_or_else(invalid)?).map_err(|_| invalid())?;
            let currency_id = currency(p.next().unwrap_or("GRAM"))?;
            if p.next().is_some() {
                return Err(invalid());
            }
            return Ok(Self::Gift { slug, currency_id });
        }
        let portfolio = name.trim_start_matches('/') == "portfolio";
        // Accept the former URL-query portfolio spelling as a decoding adapter.
        if args.contains('&') && !args.chars().any(char::is_whitespace) {
            return Self::from_query(&format!(
                "t={}&{args}",
                if portfolio { "portfolio" } else { "market" }
            ))
            .and_then(Self::public_text);
        }
        let mut params = std::collections::BTreeMap::<String, String>::new();
        let mut groups = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        params.insert(
            "t".into(),
            if portfolio { "portfolio" } else { "market" }.into(),
        );
        for (index, word) in args.split_whitespace().enumerate() {
            if !portfolio && index == 0 && word.starts_with("c=") {
                params.insert("f".into(), word.into());
                continue;
            }
            if let Some((key, value)) = word.split_once('=') {
                if !seen.insert(key) {
                    return Err(invalid());
                }
                if ![
                    "f",
                    "currency",
                    "sort",
                    "venue",
                    "mode",
                    "bid_match",
                    "min",
                    "max",
                    "from",
                    "until",
                    "user",
                    "collection",
                    "model",
                    "backdrop",
                    "symbol",
                    "gift",
                ]
                .contains(&key)
                {
                    return Err(invalid());
                }
                if ["collection", "model", "backdrop", "symbol"].contains(&key)
                    && !word.starts_with("f=")
                {
                    let value = url::form_urlencoded::parse(format!("v={value}").as_bytes())
                        .next()
                        .ok_or_else(invalid)?
                        .1
                        .into_owned();
                    if key == "collection" && value == "all" {
                        continue;
                    }
                    let namespace = if key == "collection" { "slug" } else { "name" };
                    let value = if value.starts_with('@') {
                        value
                    } else {
                        GiftAlias::new(namespace, value)
                            .map_err(|_| invalid())?
                            .selector()
                    };
                    let escaped = url::form_urlencoded::byte_serialize(value.as_bytes())
                        .collect::<String>()
                        .replace('+', "%20");
                    let tag = match key {
                        "collection" => "c",
                        "model" => "m",
                        "backdrop" => "b",
                        _ => "s",
                    };
                    groups.push(format!("{tag}={escaped}"));
                } else if params
                    .insert(
                        key.into(),
                        if key == "f" && value.contains('=') {
                            value.into()
                        } else {
                            url::form_urlencoded::parse(format!("v={value}").as_bytes())
                                .next()
                                .ok_or_else(invalid)?
                                .1
                                .into_owned()
                        },
                    )
                    .is_some()
                {
                    return Err(invalid());
                }
            } else if portfolio && index == 0 && word.bytes().all(|b| b.is_ascii_digit()) {
                params.insert("user".into(), word.into());
            } else if portfolio && !params.contains_key("currency") {
                params.insert("currency".into(), currency(word)?.to_string());
            } else {
                return Err(invalid());
            }
        }
        if portfolio {
            params
                .entry("user".into())
                .or_insert_with(|| user.to_string());
        }
        if !groups.is_empty() {
            if params.contains_key("f") {
                return Err(invalid());
            }
            params.insert("f".into(), groups.join("|"));
        }
        if let Some(c) = params.get_mut("currency") {
            *c = currency(c)?.to_string();
        }
        let mut encoded = url::form_urlencoded::Serializer::new(String::new());
        for (k, v) in params {
            encoded.append_pair(&k, &v);
        }
        Self::from_query(&encoded.finish())?.public_text()
    }
    fn public_text(self) -> Result<Self> {
        let public = match &self {
            Self::Market { selection } => public_refs(
                selection.filters.collection_id.as_ref(),
                &selection.filters.models,
                &selection.filters.backdrops,
                &selection.filters.patterns,
            ),
            Self::Portfolio { target } => public_refs(
                target.filters.collection_id.as_ref(),
                &target.filters.models,
                &target.filters.backdrops,
                &target.filters.patterns,
            ),
            _ => true,
        };
        if public { Ok(self) } else { Err(invalid()) }
    }
}
fn public_refs(
    c: Option<&CollectionRef>,
    m: &[ModelRef],
    b: &[BackdropRef],
    p: &[PatternRef],
) -> bool {
    fn public<I>(r: &CatalogSelector<I>) -> bool {
        matches!(r, CatalogSelector::Ref(_))
    }
    c.is_none_or(public) && m.iter().all(public) && b.iter().all(public) && p.iter().all(public)
}
