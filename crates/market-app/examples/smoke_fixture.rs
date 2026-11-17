//! Synthetic contract fixture. Never opens Telegram accounts or contacts a venue.
use market_hub_protocol::wire as w;
use std::time::Duration;
use tonic::{
    Request,
    metadata::MetadataValue,
    transport::{Channel, Endpoint},
};
struct FixtureClient {
    client: w::market_write_client::MarketWriteClient<Channel>,
    authorization: MetadataValue<tonic::metadata::Ascii>,
}
impl FixtureClient {
    async fn connect(endpoint: String, token: String) -> Result<Self, Box<dyn std::error::Error>> {
        let channel = Endpoint::from_shared(endpoint)?
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .connect()
            .await?;
        let mut authorization: MetadataValue<_> = format!("Bearer {token}").parse()?;
        authorization.set_sensitive(true);
        Ok(Self {
            client: w::market_write_client::MarketWriteClient::new(channel),
            authorization,
        })
    }
    fn request<T>(&self, value: T) -> Request<T> {
        let mut request = Request::new(value);
        request
            .metadata_mut()
            .insert("authorization", self.authorization.clone());
        request
    }
    async fn configure(&mut self, entries: Vec<w::Entity>) -> Result<(), tonic::Status> {
        let request = self.request(tokio_stream::iter([w::CatalogChunk {
            bundle_id: "test.market".into(),
            changes: entries
                .into_iter()
                .map(|entity| w::Change {
                    change: Some(w::change::Change::Upsert(entity)),
                })
                .collect(),
        }]));
        self.client.configure_catalog(request).await?;
        Ok(())
    }
    async fn observe(&mut self) -> Result<w::Observation, tonic::Status> {
        let request = self.request(w::FeedRequest {
            feed_id: "test.offers".into(),
        });
        Ok(self.client.observe_feed(request).await?.into_inner())
    }
    async fn publish(
        &mut self,
        observation: &w::Observation,
        changes: Vec<w::Change>,
    ) -> Result<(), tonic::Status> {
        let request = self.request(w::PublishRequest {
            observation_token: observation.token.clone(),
            changes,
        });
        self.client.publish(request).await?;
        Ok(())
    }
}
fn upsert(order: w::Order) -> w::Change {
    w::Change {
        change: Some(w::change::Change::Upsert(entry(E::Order(order)))),
    }
}
fn remove(venue: &str, id: &str) -> w::Change {
    w::Change {
        change: Some(w::change::Change::Remove(w::Removal {
            key: Some(w::Key {
                kind: w::EntityKind::Order as i32,
                venue_id: venue.into(),
                id: id.into(),
            }),
            observed_at_ms: None,
        })),
    }
}
use w::entity::Entity as E;
fn entry(value: E) -> w::Entity {
    w::Entity {
        entity: Some(value),
    }
}
fn facts(number: i64) -> std::collections::BTreeMap<String, w::AttributePredicate> {
    [
        (
            "tg.model".into(),
            w::AttributePredicate {
                predicate: Some(w::attribute_predicate::Predicate::Eq(w::AttributeValue {
                    value: Some(w::attribute_value::Value::OptionId(
                        if number % 2 == 0 {
                            "tg.gift.123.model.blue"
                        } else {
                            "tg.gift.123.model.gold"
                        }
                        .into(),
                    )),
                })),
            },
        ),
        (
            "tg.number".into(),
            w::AttributePredicate {
                predicate: Some(w::attribute_predicate::Predicate::Eq(w::AttributeValue {
                    value: Some(w::attribute_value::Value::Integer(number)),
                })),
            },
        ),
    ]
    .into()
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut feed = FixtureClient::connect(
        std::env::var("MARKET_FIXTURE_HUB_URL")?,
        std::env::var("MARKET_HUB_TOKEN")?,
    )
    .await?;
    let action = std::env::var("MARKET_FIXTURE_ACTION").unwrap_or_else(|_| "seed".into());
    if action.starts_with("seed") {
        let mut entries = vec![
            entry(E::Venue(w::Venue {
                id: "telegram".into(),
                name: "Telegram".into(),
            })),
            entry(E::Venue(w::Venue {
                id: "partner".into(),
                name: "Partner market".into(),
            })),
            entry(E::Currency(w::Currency {
                id: "GRAM".into(),
                name: "Gram".into(),
                decimals: 9,
            })),
            entry(E::Currency(w::Currency {
                id: "XTR".into(),
                name: "Stars".into(),
                decimals: 9,
            })),
            entry(E::Attribute(w::Attribute {
                id: "tg.model".into(),
                name: "Model".into(),
                value_type: w::AttributeType::Enum as i32,
                ..Default::default()
            })),
            entry(E::Attribute(w::Attribute {
                id: "tg.number".into(),
                name: "Number".into(),
                value_type: w::AttributeType::Integer as i32,
                ..Default::default()
            })),
            entry(E::AttributeOption(w::AttributeOption {
                id: "tg.gift.123.model.gold".into(),
                attribute_id: "tg.model".into(),
                name: "Gold".into(),
                ..Default::default()
            })),
            entry(E::AttributeOption(w::AttributeOption {
                id: "tg.gift.123.model.blue".into(),
                attribute_id: "tg.model".into(),
                name: "Blue".into(),
                ..Default::default()
            })),
        ];
        for (id, name) in [
            ("tg.gift.123", "Velvet Heart (test data)"),
            ("tg.gift.124", "Electric Skull (test data)"),
        ] {
            entries.push(entry(E::ItemType(w::ItemType {
                id: id.into(),
                name: name.into(),
                quantity_step: "1".into(),
                attributes: [
                    (
                        "tg.model".into(),
                        w::AttributeRule {
                            required: true,
                            allowed_values: Some(w::AllowedAttributeValues {
                                ids: vec![
                                    "tg.gift.123.model.gold".into(),
                                    "tg.gift.123.model.blue".into(),
                                ],
                            }),
                        },
                    ),
                    ("tg.number".into(), w::AttributeRule::default()),
                ]
                .into(),
                ..Default::default()
            })));
        }
        let books = ["partner", "telegram"]
            .into_iter()
            .flat_map(|venue| {
                ["tg.gift.123", "tg.gift.124"]
                    .into_iter()
                    .flat_map(move |item| {
                        ["GRAM", "XTR"].map(move |currency| w::BookKey {
                            venue_id: venue.into(),
                            item_type_id: item.into(),
                            currency_id: currency.into(),
                        })
                    })
            })
            .collect();
        entries.push(entry(E::Feed(w::Feed {
            id: "test.offers".into(),
            books,
            ..Default::default()
        })));
        feed.configure(entries).await?;
    }
    let observation = feed.observe().await?;
    let now = jiff::Timestamp::now().as_millisecond();
    if action == "delete" {
        feed.publish(&observation, vec![remove("partner", "ask-15:GRAM")])
            .await?;
        return Ok(());
    }
    if action == "update" {
        feed.publish(
            &observation,
            vec![upsert(w::Order {
                id: "ask-15:GRAM".into(),
                book: Some(w::BookKey {
                    venue_id: "partner".into(),
                    item_type_id: "tg.gift.123".into(),
                    currency_id: "GRAM".into(),
                }),
                mode: w::OrderMode::Ask as i32,
                price: "0.500000001".into(),
                quantity: "1".into(),
                item_id: Some("tg.123.15".into()),
                observed_at_ms: now,
                attributes: facts(15),
                ..Default::default()
            })],
        )
        .await?;
        return Ok(());
    }
    if action == "execute" || action == "backfill" {
        let backfill = action == "backfill";
        feed.publish(
            &observation,
            vec![upsert(w::Order {
                id: if backfill {
                    "old-execution".into()
                } else {
                    "ask-15:GRAM".into()
                },
                book: Some(w::BookKey {
                    venue_id: "partner".into(),
                    item_type_id: "tg.gift.123".into(),
                    currency_id: "GRAM".into(),
                }),
                mode: w::OrderMode::Executed as i32,
                executed_at_ms: Some(if backfill { now - 86_400_000 } else { now }),
                observed_at_ms: now,
                price: if backfill {
                    "0.1".into()
                } else {
                    "1.25".into()
                },
                quantity: "1".into(),
                attributes: facts(15),
                ..Default::default()
            })],
        )
        .await?;
        return Ok(());
    }
    let mut rows = Vec::new();
    for venue in ["partner", "telegram"] {
        for item in ["tg.gift.123", "tg.gift.124"] {
            for currency in ["GRAM", "XTR"] {
                for n in 1..=if item.ends_with("123") { 15 } else { 3 } {
                    if action == "seed-recover"
                        && venue == "partner"
                        && item == "tg.gift.123"
                        && currency == "GRAM"
                        && n == 15
                    {
                        continue;
                    }
                    let whole = if venue == "partner" { 16 - n } else { 31 - n };
                    rows.push(upsert(w::Order {
                        id: format!(
                            "{}-ask-{n:02}:{currency}",
                            if item.ends_with("123") { "" } else { "skull" }
                        )
                        .trim_start_matches('-')
                        .into(),
                        book: Some(w::BookKey {
                            venue_id: venue.into(),
                            item_type_id: item.into(),
                            currency_id: currency.into(),
                        }),
                        mode: w::OrderMode::Ask as i32,
                        price: if currency == "GRAM" {
                            format!("{whole}.25")
                        } else {
                            format!("{}", whole * 1000 + 250)
                        },
                        quantity: "1".into(),
                        observed_at_ms: now,
                        item_id: Some(format!(
                            "tg.{}.{}",
                            if item.ends_with("123") { 123 } else { 124 },
                            n
                        )),
                        attributes: facts(n),
                        ..Default::default()
                    }));
                }
                rows.push(upsert(w::Order {
                    id: format!("{}-bid:{currency}", item),
                    book: Some(w::BookKey {
                        venue_id: venue.into(),
                        item_type_id: item.into(),
                        currency_id: currency.into(),
                    }),
                    mode: w::OrderMode::Bid as i32,
                    price: if currency == "GRAM" {
                        "0.75".into()
                    } else {
                        "750".into()
                    },
                    quantity: "10".into(),
                    observed_at_ms: now,
                    attributes: [(
                        "tg.model".into(),
                        w::AttributePredicate {
                            predicate: Some(w::attribute_predicate::Predicate::OneOf(
                                w::AttributeValues {
                                    values: ["tg.gift.123.model.gold", "tg.gift.123.model.blue"]
                                        .map(|v| w::AttributeValue {
                                            value: Some(w::attribute_value::Value::OptionId(
                                                v.into(),
                                            )),
                                        })
                                        .into(),
                                },
                            )),
                        },
                    )]
                    .into(),
                    ..Default::default()
                }));
            }
        }
    }
    if action == "seed" {
        rows.push(upsert(w::Order {
            id: "execution-1".into(),
            book: Some(w::BookKey {
                venue_id: "partner".into(),
                item_type_id: "tg.gift.123".into(),
                currency_id: "GRAM".into(),
            }),
            mode: w::OrderMode::Executed as i32,
            executed_at_ms: Some(now - 1000),
            observed_at_ms: now,
            price: "0.75".into(),
            quantity: "1".into(),
            attributes: facts(1),
            ..Default::default()
        }));
    }
    feed.publish(&observation, rows).await?;
    Ok(())
}
