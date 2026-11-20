//! AsyncAPI wraps the same serde/utoipa message types used by the socket handler.
use crate::{models::*, schema::ApiDoc};
use serde_json::{Map, Value, json};
#[cfg(test)]
use std::collections::BTreeSet;
use utoipa::OpenApi;

/// Public, transport-only specification. No runtime identity or upstream configuration.
pub fn document() -> serde_json::Result<Value> {
    let openapi = serde_json::to_value(ApiDoc::openapi())?;
    let source = openapi["components"]["schemas"]
        .as_object()
        .expect("generated schemas");
    let mut schemas = Map::new();
    let mut pending = vec!["Command".to_owned(), "ServerFrame".to_owned()];
    while let Some(name) = pending.pop() {
        if schemas.contains_key(&name) {
            continue;
        }
        let schema = source
            .get(&name)
            .expect("generated reference resolves")
            .clone();
        references(&schema, &mut pending);
        schemas.insert(name, schema);
    }
    let mut messages = Map::new();
    let mut channel_messages = Map::new();
    let mut operations = Map::new();
    for (root, action, operation) in [
        ("Command", "receive", "receiveCommands"),
        ("ServerFrame", "send", "sendUpdates"),
    ] {
        let mut refs = vec![];
        for (index, schema) in schemas[root]["oneOf"]
            .as_array()
            .expect("tagged message enum")
            .iter()
            .enumerate()
        {
            let name = message_tag(schema).expect("serde message tag");
            let mut message = json!({
                "name":name,
                "title":name,
                "summary": summary(name),
                "contentType":"application/json",
                "payload":{"$ref":format!("#/components/schemas/{root}/oneOf/{index}")},
                "examples": examples(name)?
            });
            if name != "market.status" {
                message["correlationId"] = json!({"location":"$message.payload#/id","description":"Unique operation ID. Empty on command rejections without a valid identity."});
            }
            messages.insert(name.into(), message);
            channel_messages.insert(
                name.into(),
                json!({"$ref":format!("#/components/messages/{name}")}),
            );
            refs.push(json!({"$ref":format!("#/channels/market/messages/{name}")}));
        }
        operations.insert(
            operation.into(),
            json!({"action":action,"channel":{"$ref":"#/channels/market"},"messages":refs}),
        );
    }
    Ok(json!({
        "asyncapi":"3.0.0",
        "info":{
            "title":"Gift market WebSocket API",
            "version":env!("CARGO_PKG_VERSION"),
            "description": include_str!("websocket/protocol.md")
        },
        "defaultContentType":"application/json",
        "channels":{"market":{"address":"/ws","description":"Connect to /ws on this application's origin using ws or wss. Operations are described from the server's perspective.","messages":channel_messages}},
        "operations":operations,
        "components":{"schemas":schemas,"messages":messages}
    }))
}
fn message_tag(schema: &Value) -> Option<&str> {
    schema["properties"]["type"]["enum"][0]
        .as_str()
        .or_else(|| schema["allOf"].as_array()?.iter().find_map(message_tag))
}
fn references(value: &Value, pending: &mut Vec<String>) {
    match value {
        Value::Object(fields) => {
            if let Some(name) = fields
                .get("$ref")
                .and_then(Value::as_str)
                .and_then(|reference| reference.strip_prefix("#/components/schemas/"))
            {
                pending.push(name.split('/').next().expect("schema name").to_owned());
            }
            for value in fields.values() {
                references(value, pending);
            }
        }
        Value::Array(values) => {
            for value in values {
                references(value, pending);
            }
        }
        _ => {}
    }
}
fn summary(name: &str) -> &'static str {
    match name {
        "request" => "Run one typed API request. One result or error completes the operation.",
        "command_error" => {
            "Rejected command; active operations continue unchanged. ID is empty when the command has no valid identity."
        }
        "result" => "Typed request result; valuation uses the same response as HTTP.",
        "cancel" => "Cancel a pending request or subscription. No acknowledgement is sent.",
        "orders.watch" => "Read up to limit rows, then watch the initialized coverage atomically.",
        "orders.snapshot" => {
            "Initialize or reset state atomically; the half-open coverage end also advances pagination."
        }
        "orders.changed" => {
            "Apply removals and upserts atomically after checking sequence continuity."
        }
        "catalog.watch" => {
            "Watch catalog content revisions for up to eight collections, without polling."
        }
        "market.values.watch" => {
            "Watch batched current collection and selected trait ask floors, refreshed on relevant changes or expiry."
        }
        "market.values" => {
            "Atomic current-ask projection in the requested currency. Missing evidence has a typed reason."
        }
        "catalog.versions" => {
            "Compact opaque catalog content tokens. Compare with unquoted HTTP ETags to revalidate changed entries."
        }
        "market.status" => {
            "Market availability: available, delayed or unavailable, independent of query lifetimes."
        }
        "error" => "A terminal request or subscription error, correlated by operation ID.",
        _ => "Protocol message.",
    }
}
fn examples(name: &str) -> serde_json::Result<Vec<Value>> {
    let value = match name {
        "request" => serde_json::to_value(Command::Request(ApiRequest::Valuation {
            id: "request-1".into(),
            params: Box::new(serde_json::from_value(
                json!({"gift":"Gift-123","currency_id":"GRAM"}),
            )?),
        }))?,
        "cancel" => serde_json::to_value(Command::Cancel {
            id: "request-1".into(),
        })?,
        "result" => {
            let mut data = json!({
                "slug":null,"gift":{"id":"gift-123","collection_id":"tg.gift.123","number":123,"traits":{"model":"model","backdrop":"backdrop","pattern":null}},
                "currency_id":"GRAM",
                "evidence":{"collection":{"listings":3,"gifts":3},"model":{"listings":3,"gifts":3},"backdrop":{"listings":3,"gifts":3},"model_backdrop":{"listings":1,"gifts":1},"minimum_multiplier_gifts":3}
            });
            for field in [
                "collection_floor",
                "model_floor",
                "backdrop_floor",
                "model_backdrop_floor",
                "model_multiplier",
                "backdrop_multiplier",
                "final_mark",
            ] {
                data[field] = json!({"value":"1"});
            }
            let data: market_valuation::GiftFloorValuation = serde_json::from_value(data)?;
            serde_json::to_value(ServerFrame::Result(ApiResult::Valuation {
                id: "request-1".into(),
                data: GiftValuationResponse {
                    strategy: "gift_floor",
                    stale: false,
                    evaluated_at: "2026-11-18T12:00:00Z".parse().expect("example timestamp"),
                    display: data.display(),
                    data: &data,
                },
            }))?
        }
        "orders.watch" => serde_json::to_value(Command::WatchOrders {
            id: "orders.1".into(),
            params: Box::new(serde_json::from_value(
                json!({"filters":{"collection_id":"tg.gift.123","currency_id":"GRAM"},"source":"asks","sort":"price_asc","range":{"start":null,"end":null},"limit":100}),
            )?),
        })?,
        "orders.snapshot" => serde_json::to_value(ServerFrame::OrdersSnapshot {
            id: "orders.1".into(),
            sequence: 0,
            data: OrderSnapshotResponse {
                data: vec![],
                range: crate::orders::OrderRangeResponse {
                    start: None,
                    end: None,
                },
            },
        })?,
        "orders.changed" => serde_json::to_value(ServerFrame::OrdersChanged {
            id: "orders.1".into(),
            sequence: 1,
            upserts: vec![],
            removed: vec![],
        })?,
        "catalog.watch" => serde_json::to_value(Command::WatchCatalog {
            id: "catalog".into(),
            collection_ids: vec!["tg.gift.123".into()],
        })?,
        "market.values.watch" => serde_json::to_value(Command::WatchMarketValues {
            id: "prices".into(),
            params: Box::new(serde_json::from_value(
                json!({"currency_id":"GRAM","collection_ids":["tg.gift.123"],"traits_for":"tg.gift.123","pair":null}),
            )?),
        })?,
        "market.values" => {
            let data = serde_json::from_value(
                json!({"currency_id":"GRAM","evaluated_at":"2026-11-18T12:00:00Z","stale":false,"collections":[{"collection_id":"tg.gift.123","floor":{"price":{"value":"3"},"evidence":{"listings":3,"gifts":3}}}],"traits":{"collection_id":"tg.gift.123","models":{},"backdrops":{}}}),
            )?;
            serde_json::to_value(ServerFrame::MarketValues {
                id: "prices".into(),
                data: &data,
            })?
        }
        "catalog.versions" => serde_json::to_value(ServerFrame::Catalog {
            id: "catalog".into(),
            versions: crate::catalog::versions(CatalogVersions {
                directory: "directory".into(),
                collections: [("tg.gift.123".into(), Some("collection".into()))].into(),
            }),
        })?,
        "market.status" => serde_json::to_value(ServerFrame::MarketStatus {
            status: MarketStatus::Available,
        })?,
        "command_error" => serde_json::to_value(ServerFrame::CommandError {
            id: "offers".into(),
            error: crate::error::ApiError::invalid("duplicate operation ID"),
        })?,
        "error" => serde_json::to_value(ServerFrame::Error {
            id: "offers".into(),
            error: crate::error::ApiError::invalid("Invalid subscription query"),
        })?,
        _ => return Ok(vec![]),
    };
    let mut examples = vec![json!({"name":name,"payload":value})];
    if name == "request" {
        let request = Command::Request(ApiRequest::TargetValuation {
            id: "target.1".into(),
            params: Box::new(serde_json::from_value(
                json!({"target":{"kind":"collection","collection_id":"tg.gift.123"},"currency_id":"GRAM","strategies":["ask_floor","execution_mean","execution_median"],"period":{"from":"2026-01-01T00:00:00Z","until":"2026-02-01T00:00:00Z"}}),
            )?),
        });
        examples.push(json!({"name":"target-valuations","payload":serde_json::to_value(request)?}));
    }
    if name == "orders.watch" {
        let bid = Command::WatchOrders {
            id: "bids.1".into(),
            params: Box::new(serde_json::from_value(
                json!({"source":"bids","bid_match":"all","filters":{"collection_id":"tg.gift.123","currency_id":"GRAM","models":["gold","silver"]},"limit":80}),
            )?),
        };
        examples.push(json!({"name":"bids.all","payload":serde_json::to_value(bid)?}));
    }
    Ok(examples)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn websocket_schemas_and_examples_are_derived_from_the_public_wire_contract() {
        let spec = document().unwrap();
        let openapi = serde_json::to_value(ApiDoc::openapi()).unwrap();
        for (name, schema) in spec["components"]["schemas"].as_object().unwrap() {
            assert_eq!(schema, &openapi["components"]["schemas"][name]);
        }
        let mut refs = vec![];
        references(&spec, &mut refs);
        let names: BTreeSet<_> = spec["components"]["schemas"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        assert!(refs.iter().all(|name| names.contains(name)));
        assert!(!names.contains("GiftCatalog"));
        assert!(!names.contains("Session"));
        let versions = &spec["components"]["messages"]["catalog.versions"]["examples"][0]["payload"]
            ["versions"];
        assert_eq!(versions["directory"], crate::catalog::revision("directory"));
        assert_eq!(
            versions["collections"]["tg.gift.123"],
            crate::catalog::revision("collection")
        );
        for command in ["catalog.watch", "orders.watch", "request", "cancel"] {
            serde_json::from_value::<Command>(
                spec["components"]["messages"][command]["examples"][0]["payload"].clone(),
            )
            .unwrap();
        }
        assert_eq!(
            spec["operations"]["receiveCommands"]["messages"]
                .as_array()
                .unwrap()
                .len(),
            5
        );
        assert_eq!(
            spec["operations"]["sendUpdates"]["messages"]
                .as_array()
                .unwrap()
                .len(),
            8
        );
    }
}

#[cfg(test)]
mod http_tests {
    use actix_web::{App, test};
    #[actix_web::test]
    async fn specifications_work_without_market_state_frontend_or_telegram_configuration() {
        let app = test::init_service(App::new().configure(crate::configure)).await;
        for path in ["/openapi.json", "/asyncapi.json"] {
            let response =
                test::call_service(&app, test::TestRequest::get().uri(path).to_request()).await;
            assert_eq!(response.status(), 200);
            assert_eq!(
                response.headers().get("content-type").unwrap(),
                "application/json"
            );
            let body: serde_json::Value = test::read_body_json(response).await;
            assert_eq!(body["info"]["version"], env!("CARGO_PKG_VERSION"));
            if path == "/asyncapi.json" {
                assert_eq!(body["asyncapi"], "3.0.0");
            } else {
                for item in body["paths"].as_object().unwrap().values() {
                    for operation in item.as_object().unwrap().values() {
                        if let Some(tags) = operation.get("tags").and_then(|tags| tags.as_array()) {
                            assert!(tags.iter().all(|tag| !tag.as_str().unwrap().contains("::")));
                        }
                    }
                }
            }
        }
        assert_eq!(
            test::call_service(&app, test::TestRequest::get().uri("/docs").to_request())
                .await
                .status(),
            404
        );
    }
}
