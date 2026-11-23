//! One gift report, rendered identically for private, inline and guest replies.
use super::*;
pub(super) use crate::icons::escape;
use crate::{ReportIcon, ReportIcons};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use market_gifts::*;
use market_valuation::*;
use std::sync::Arc;

pub(super) struct Report {
    pub query: ValueRequest,
    pub value: Arc<ValuationReport>,
    pub catalog: Option<GiftCollectionCatalog>,
    pub floors: Option<Arc<CatalogFloorValuation>>,
    pub artwork: Option<market_media::GiftCover>,
    pub icons: Arc<ReportIcons>,
}
pub(super) fn amount(value: &Amount) -> String {
    value.decimal().round_dp(2).normalize().to_string()
}
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
        let title = self
            .catalog
            .as_ref()
            .map(|catalog| format!("{} #{}", catalog.collection.name, self.query.gift.number()))
            .unwrap_or_else(|| self.query.gift.to_string());
        let gift_url = url::Url::parse(&format!("https://t.me/nft/{}", self.query.gift))
            .expect("validated slug");
        let mut html = format!("<h3><a href=\"{}\">{}</a></h3>", gift_url, escape(&title));
        let mut lines = vec![title.clone()];
        if let (Some(catalog), Some(gift)) = (&self.catalog, self.gift()) {
            let names = [
                gift.traits
                    .model
                    .as_ref()
                    .and_then(|id| catalog.models.iter().find(|v| &v.id == id))
                    .map(|v| (ReportIcon::Model, v.name.as_str())),
                gift.traits
                    .backdrop
                    .as_ref()
                    .and_then(|id| catalog.backdrops.iter().find(|v| &v.id == id))
                    .map(|v| (ReportIcon::Backdrop, v.name.as_str())),
                gift.traits
                    .pattern
                    .as_ref()
                    .and_then(|id| catalog.patterns.iter().find(|v| &v.id == id))
                    .map(|v| (ReportIcon::Symbol, v.name.as_str())),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
            if !names.is_empty() {
                html.push_str(&format!(
                    "<p>{}</p>",
                    names
                        .iter()
                        .map(|(icon, name)| format!("{} {}", self.icons.html(*icon), escape(name)))
                        .collect::<Vec<_>>()
                        .join(" · ")
                ));
                lines.push(
                    names
                        .iter()
                        .map(|(icon, name)| format!("{} {}", self.icons.text(*icon), name))
                        .collect::<Vec<_>>()
                        .join(" · "),
                );
            }
        }
        let mut summaries = Vec::new();
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
                    "<p>{} <b>{}</b>　<b>{}</b></p>",
                    self.icons.html(ReportIcon::Estimate),
                    escape(&label),
                    value_html
                ));
                summaries.push(format!("{label}: {value}"));
            }
        }
        lines.extend(summaries.iter().cloned());
        if let (Some(gift), Some(floors)) = (self.gift(), &self.floors)
            && let Some(collection) = floors
                .collections
                .iter()
                .find(|row| row.collection_id == gift.collection_id)
        {
            let traits = collection.traits.as_ref();
            let rows = [
                (
                    ReportIcon::Collection,
                    "Collection",
                    Some(&collection.floor),
                ),
                (
                    ReportIcon::ModelBackdrop,
                    "Model + backdrop",
                    traits.and_then(|t| t.pair.as_ref().map(|p| &p.floor)),
                ),
                (
                    ReportIcon::Model,
                    "Model",
                    gift.traits
                        .model
                        .as_ref()
                        .and_then(|id| traits.and_then(|t| t.models.get(id))),
                ),
                (
                    ReportIcon::Backdrop,
                    "Backdrop",
                    gift.traits
                        .backdrop
                        .as_ref()
                        .and_then(|id| traits.and_then(|t| t.backdrops.get(id))),
                ),
                (
                    ReportIcon::Symbol,
                    "Symbol",
                    gift.traits
                        .pattern
                        .as_ref()
                        .and_then(|id| traits.and_then(|t| t.patterns.get(id))),
                ),
            ]
            .into_iter()
            .filter_map(|(icon, label, floor)| {
                floor
                    .and_then(|floor| floor.value())
                    .map(|value| (icon, label, amount(value)))
            })
            .collect::<Vec<_>>();
            if !rows.is_empty() {
                html.push_str(&format!(
                    "<table compact><tr><th align=\"left\">Ask floors</th><th align=\"right\">{}</th></tr>",
                    self.icons.currency_html(self.value.currency_id.as_str())
                ));
                lines.push(format!("\nAsk floors · {}", self.value.currency_id));
                for (icon, label, value) in rows {
                    html.push_str(&format!(
                        "<tr><td>{} {label}</td><td align=\"right\">{}</td></tr>",
                        self.icons.html(icon),
                        escape(&value)
                    ));
                    lines.push(format!("{} {label}: {value}", self.icons.text(icon)));
                }
                html.push_str("</table>");
            }
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
        let value = match self.valuation.query(request).await {
            Ok(value) => value,
            Err(error) => {
                return render::error_reply(if error.code() == ValuationErrorCode::Unavailable {
                    "Valuations are temporarily unavailable"
                } else {
                    error.message()
                });
            }
        };
        let mut report = Report {
            query,
            value,
            catalog: None,
            floors: None,
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
                    model_id,
                    backdrop_id,
                });
            let request = ValuationQuery {
                target: ValuationTarget::Catalog(CatalogSelection {
                    collection_ids: vec![gift.collection_id.clone()],
                    details: Some(CatalogDetails {
                        collection_id: gift.collection_id.clone(),
                        pair,
                    }),
                }),
                currency_id: report.query.currency_id.clone(),
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
            let (catalog, floors, artwork) = tokio::join!(
                self.market.collection_snapshot(gift.collection_id.clone()),
                self.valuation.query(request),
                artwork
            );
            report.catalog = catalog.ok().map(|v| v.data);
            if let Some(model) = artwork.0 {
                report.set_artwork(model, artwork.1);
            }
            report.floors = floors.ok().and_then(|v| {
                v.results.iter().find_map(|v| match v {
                    StrategyOutcome::Value(StrategyValuation::CatalogFloor(value)) => {
                        Some(value.clone())
                    }
                    _ => None,
                })
            });
        }
        let report = Arc::new(report);
        self.recent_sales(report).await
    }
}
