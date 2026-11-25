//! One gift report, rendered identically for private, inline and guest replies.
use super::*;
pub(super) use crate::icons::escape;
use crate::{ReportIcon, ReportIcons};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use market_gifts::*;
use market_valuation::*;
use std::sync::Arc;

#[derive(Default)]
pub(super) struct FloorQuotes {
    pub gram: Option<Arc<CatalogFloorValuation>>,
    pub stars: Option<Arc<CatalogFloorValuation>>,
}
pub(super) struct Report {
    pub query: ValueRequest,
    pub value: Arc<ValuationReport>,
    pub catalog: Option<GiftCollectionCatalog>,
    pub floors: FloorQuotes,
    pub artwork: Option<market_media::GiftCover>,
    pub icons: Arc<ReportIcons>,
}
pub(super) use super::rich::amount;
fn formatted_amount(value: &str) -> String {
    Amount::new(value)
        .map(|value| amount(&value))
        .unwrap_or_else(|_| value.to_owned())
}
fn display(value: &DisplayValue, icons: &ReportIcons) -> (String, String) {
    let text = match value {
        DisplayValue::Currency {
            amount,
            currency_id,
        } => {
            let amount = formatted_amount(amount);
            return (
                format!("{amount} {currency_id}"),
                format!(
                    "{} {}",
                    escape(&amount),
                    icons.currency_html(currency_id.as_str())
                ),
            );
        }
        DisplayValue::Number { amount, suffix } => format!("{amount}{suffix}"),
        DisplayValue::Unavailable { message, .. } => message.chars().take(200).collect(),
    };
    let html = escape(&text);
    (text, html)
}

impl Report {
    pub fn gift(&self) -> Option<&CollectibleGift> {
        self.value.target.gift()
    }
    pub fn set_artwork(&mut self, model: TelegramDocumentId, pattern: Option<TelegramDocumentId>) {
        let palette = self
            .catalog
            .as_ref()
            .zip(self.gift())
            .and_then(|(catalog, gift)| {
                let id = gift.traits.backdrop.as_ref()?;
                let palette = catalog
                    .backdrops
                    .iter()
                    .find(|option| &option.id == id)?
                    .palette
                    .as_ref()?;
                let color = |value: &RgbColor| {
                    u32::from_str_radix(value.to_string().trim_start_matches('#'), 16)
                        .expect("validated RGB color")
                };
                Some(market_media::CoverPalette {
                    center: color(&palette.center),
                    edge: color(&palette.edge),
                    pattern: color(&palette.pattern),
                })
            });
        self.artwork = Some(market_media::GiftCover {
            model: market_media::DocumentId::new(model.get()).expect("validated document"),
            pattern: pattern.and_then(|id| market_media::DocumentId::new(id.get())),
            palette,
        });
    }
    pub fn render(&self) -> Reply {
        self.render_panel(true, "", "")
    }
    pub fn render_panel(&self, floors_visible: bool, navigation: &str, refresh: &str) -> Reply {
        let title = self
            .catalog
            .as_ref()
            .map(|catalog| format!("{} #{}", catalog.collection.name, self.query.gift.number()))
            .unwrap_or_else(|| self.query.gift.to_string());
        let gift_url = url::Url::parse(&format!("https://t.me/nft/{}", self.query.gift))
            .expect("validated slug");
        let mut html = format!("<h3><a href=\"{}\">{}</a></h3>", gift_url, escape(&title));
        let mut lines = vec![title.clone()];
        let mut summaries = Vec::new();
        let mut refresh = refresh;
        for result in &self.value.results {
            let strategy_name =
                strategy(result.strategy()).map_or(result.strategy(), |meta| meta.name);
            let view = result.display(&self.value.currency_id);
            let primary = view
                .fields
                .iter()
                .filter(|field| field.primary)
                .collect::<Vec<_>>();
            for field in &primary {
                let label = if primary.len() > 1 {
                    format!("{strategy_name} · {}", field.label)
                } else if self.value.results.len() == 1 {
                    "Estimate".to_owned()
                } else {
                    strategy_name.to_owned()
                };
                let (value, value_html) = display(&field.value, &self.icons);
                html.push_str(&format!(
                    "<p>{} <b>{}</b> {}</p>",
                    self.icons.html(ReportIcon::Estimate),
                    escape(&label),
                    refresh
                ));
                refresh = "";
                match &field.value {
                    DisplayValue::Unavailable { .. } => {
                        html.push_str(&format!("<p>{value_html}</p>"));
                    }
                    _ => html.push_str(&format!("<h1>{value_html}</h1>")),
                }
                summaries.push(format!("{label}: {value}"));
            }
        }
        html.push_str(refresh);
        lines.extend(summaries.iter().cloned());
        html.push_str(navigation);
        if floors_visible {
            self.render_attributes(&mut html, &mut lines);
        }
        if self.value.stale {
            html.push_str(&format!(
                "<p>{} Updates delayed</p>",
                self.icons.html(ReportIcon::Warning)
            ));
            lines.push("Updates delayed".into());
        }
        let mut reply = Reply {
            text: lines.join("\n"),
            title: Some(title),
            summary: Some(summaries.join(" · ")),
            rich_html: Some(html),
            artwork: self.artwork,
            ..Default::default()
        };
        reply
            .buttons
            .push(vec![market_telegram::ReplyButton::Preview {
                text: "Open preview".into(),
                payload: self.preview_payload(),
            }]);
        reply
            .buttons
            .push(vec![market_telegram::ReplyButton::SwitchInline {
                text: "Share".into(),
                query: format!("{} {}", self.query.gift, self.query.currency_id),
            }]);
        reply
    }
    fn reference_floor<'a>(
        &self,
        icon: ReportIcon,
        quotes: Option<&'a CatalogFloorValuation>,
    ) -> Option<&'a FloorValuation> {
        let gift = self.gift()?;
        let collection = quotes?
            .collections
            .iter()
            .find(|row| row.collection_id == gift.collection_id)?;
        if icon == ReportIcon::Collection {
            return Some(&collection.floor);
        }
        let traits = collection.traits.as_ref()?;
        match icon {
            ReportIcon::ModelBackdrop => traits
                .pair
                .as_ref()
                .filter(|pair| {
                    Some(&pair.model_id) == gift.traits.model.as_ref()
                        && Some(&pair.backdrop_id) == gift.traits.backdrop.as_ref()
                })
                .map(|pair| &pair.floor),
            ReportIcon::Model => traits.models.get(gift.traits.model.as_ref()?),
            ReportIcon::Backdrop => traits.backdrops.get(gift.traits.backdrop.as_ref()?),
            ReportIcon::Symbol => traits.patterns.get(gift.traits.pattern.as_ref()?),
            _ => None,
        }
    }
    fn render_attributes(&self, html: &mut String, lines: &mut Vec<String>) {
        let Some(gift) = self.gift() else { return };
        let model = self
            .catalog
            .as_ref()
            .and_then(|catalog| {
                gift.traits
                    .model
                    .as_ref()
                    .and_then(|id| catalog.models.iter().find(|v| &v.id == id))
            })
            .map(|v| v.name.as_str());
        let backdrop = self
            .catalog
            .as_ref()
            .and_then(|catalog| {
                gift.traits
                    .backdrop
                    .as_ref()
                    .and_then(|id| catalog.backdrops.iter().find(|v| &v.id == id))
            })
            .map(|v| v.name.as_str());
        let symbol = self
            .catalog
            .as_ref()
            .and_then(|catalog| {
                gift.traits
                    .pattern
                    .as_ref()
                    .and_then(|id| catalog.patterns.iter().find(|v| &v.id == id))
            })
            .map(|v| v.name.as_str());
        let gram = self.icons.currency_html("GRAM");
        let stars = format!("{} Stars", self.icons.html(ReportIcon::Stars));
        html.push_str(&format!("<table compact><tr><th align=\"left\">Attribute</th><th align=\"right\">{gram}</th><th align=\"right\">{stars}</th></tr>"));
        lines.push("\nFloors · GRAM | Stars".into());
        for (icon, label, visible) in [
            (ReportIcon::Collection, "Collection", true),
            (
                ReportIcon::ModelBackdrop,
                "Model + backdrop",
                gift.traits.model.is_some() && gift.traits.backdrop.is_some(),
            ),
            (
                ReportIcon::Model,
                model.unwrap_or("Model"),
                gift.traits.model.is_some(),
            ),
            (
                ReportIcon::Backdrop,
                backdrop.unwrap_or("Backdrop"),
                gift.traits.backdrop.is_some(),
            ),
            (
                ReportIcon::Symbol,
                symbol.unwrap_or("Symbol"),
                gift.traits.pattern.is_some(),
            ),
        ] {
            if !visible {
                continue;
            }
            let values =
                [self.floors.gram.as_deref(), self.floors.stars.as_deref()].map(|quotes| {
                    self.reference_floor(icon, quotes)
                        .and_then(|floor| floor.value())
                        .map(amount)
                        .unwrap_or_else(|| "—".into())
                });
            html.push_str(&format!(
                "<tr><td>{} {}</td><td align=\"right\">{}</td><td align=\"right\">{}</td></tr>",
                self.icons.html(icon),
                escape(label),
                values[0],
                values[1]
            ));
            lines.push(format!(
                "{} {label}: {} GRAM · {} Stars",
                self.icons.text(icon),
                values[0],
                values[1]
            ));
        }
        html.push_str("</table>");
    }
    pub fn preview_payload(&self) -> String {
        let mut params = url::form_urlencoded::Serializer::new(String::new());
        if let Some(gift) = self.gift() {
            params.append_pair("collection", gift.collection_id.as_str());
        }
        params
            .append_pair("currency", self.query.currency_id.as_str())
            .append_pair("gift", &self.query.gift.to_string());
        format!("v_{}", URL_SAFE_NO_PAD.encode(params.finish()))
    }
}
impl MarketCommands {
    pub(super) async fn value(&self, query: ValueRequest) -> Reply {
        match self.load_report(query).await {
            Ok(report) => self.open_report(report),
            Err(error) => render::error_reply(report_error(&error)),
        }
    }
    pub(super) async fn load_report(
        &self,
        query: ValueRequest,
    ) -> Result<Arc<Report>, ValuationError> {
        let request = ValuationQuery {
            target: ValuationTarget::Gift {
                gift: query.gift.clone().into(),
            },
            currency_id: query.currency_id.clone(),
            strategies: strategies_for(ValuationTargetKind::Gift, false)
                .filter(|meta| meta.purpose == StrategyPurpose::Estimate)
                .map(|meta| StrategyId::new(meta.id).expect("registered ID"))
                .collect(),
            period: None,
        };
        let value = self.valuation.query(request).await?;
        let mut report = Report {
            query,
            value,
            catalog: None,
            floors: FloorQuotes::default(),
            artwork: None,
            icons: self.icons.clone(),
        };
        if let Some(gift) = report.gift().cloned() {
            let pair = gift
                .traits
                .model
                .clone()
                .zip(gift.traits.backdrop.clone())
                .map(|(model_id, backdrop_id)| PairSelection {
                    model_id: model_id.into(),
                    backdrop_id: backdrop_id.into(),
                });
            let target = ValuationTarget::Catalog(CatalogSelection {
                collection_ids: vec![gift.collection_id.clone().into()],
                details: Some(CatalogDetails {
                    collection_id: gift.collection_id.clone().into(),
                    pair,
                }),
            });
            let request = |currency: &'static str| ValuationQuery {
                target: target.clone(),
                currency_id: currency.into(),
                strategies: vec![StrategyId::new("catalog_floor").expect("registered ID")],
                period: None,
            };
            let artwork = async {
                let model = async {
                    match gift.traits.model.clone() {
                        Some(id) => self
                            .market
                            .media_document(MediaTarget::Model(id))
                            .await
                            .ok(),
                        None => None,
                    }
                };
                let pattern =
                    async {
                        match gift.traits.pattern.clone().filter(|_| {
                            gift.traits.model.is_some() && gift.traits.backdrop.is_some()
                        }) {
                            Some(id) => self
                                .market
                                .media_document(MediaTarget::Pattern(id))
                                .await
                                .ok(),
                            None => None,
                        }
                    };
                tokio::join!(model, pattern)
            };
            let (catalog, gram, stars, artwork) = tokio::join!(
                self.market
                    .collection_snapshot(gift.collection_id.clone().into()),
                self.valuation.query(request("GRAM")),
                self.valuation.query(request("XTR")),
                artwork
            );
            report.catalog = catalog.ok().map(|v| v.data);
            if let Some(model) = artwork.0 {
                report.set_artwork(model, artwork.1);
            }
            report.floors = FloorQuotes {
                gram: catalog_floors(gram),
                stars: catalog_floors(stars),
            };
        }
        Ok(Arc::new(report))
    }
}

pub(super) fn report_error(error: &ValuationError) -> String {
    if error.code() == ValuationErrorCode::Unavailable {
        "Valuations are temporarily unavailable".into()
    } else {
        error.message().to_owned()
    }
}

fn catalog_floors(
    result: Result<Arc<ValuationReport>, ValuationError>,
) -> Option<Arc<CatalogFloorValuation>> {
    result.ok()?.results.iter().find_map(|result| match result {
        StrategyOutcome::Value(StrategyValuation::CatalogFloor(value)) => Some(value.clone()),
        _ => None,
    })
}
