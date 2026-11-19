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
            messages.insert(name.into(), json!({
                "name":name,
                "title":name,
                "summary": summary(name),
                "contentType":"application/json",
                "correlationId":{"location":"$message.payload#/id", "description":"Unique operation ID. Empty on connection-level errors."},
                "payload":{"$ref":format!("#/components/schemas/{root}/oneOf/{index}")},
                "examples": examples(name)?
            }));
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
        "command_error" => "Rejected command; the existing operation continues unchanged.",
        "result" => "Typed request result; valuation uses the same response as HTTP.",
        "cancel" => "Cancel a pending request or subscription. No acknowledgement is sent.",
        "orders.watch" => "Watch a fixed ordered range with an atomic initial snapshot.",
        "orders.snapshot" => "Initialize or reset range state atomically.",
        "orders.changed" => "Apply removals and upserts atomically after checking base_sequence.",
        "watch_catalog" => {
            "Watch catalog content revisions for up to eight collections, without polling."
        }
        "catalog" => {
            "Current catalog content revisions. Compare with HTTP ETags to revalidate changed entries."
        }
        "status" => "The subscription is connecting.",
        "sync" => {
            "Update freshness and revision without replacing orders. The server has checked the subscribed range."
        }
        "error" => {
            "A request, subscription or connection-level error. Inspect code and operation ID."
        }
        _ => "Protocol message.",
    }
}
fn examples(name: &str) -> serde_json::Result<Vec<Value>> {
    let sync = SyncStatus {
        ready: true,
        state: SyncState::Live,
        updated_at: None,
    };
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
                "currency_id":"GRAM","sync":sync,
                "evidence":{"collection_samples":3,"model_samples":3,"backdrop_samples":3,"model_backdrop_samples":1,"minimum_multiplier_samples":3}
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
                    revision: "opaque-revision",
                    evaluated_at: "2026-11-18T12:00:00Z".parse().expect("example timestamp"),
                    display: data.display(),
                    data: &data,
                },
            }))?
        }
        "orders.watch" => serde_json::to_value(Command::WatchOrders {
            id: "orders.1".into(),
            params: Box::new(serde_json::from_value(
                json!({"query":{"filters":{"collection_id":"tg.gift.123","currency_id":"GRAM"},"source":"asks","sort":"price_asc","position":null,"limit":150,"direction":"around"}}),
            )?),
        })?,
        "orders.snapshot" => serde_json::to_value(ServerFrame::OrdersSnapshot {
            id: "orders.1".into(),
            sequence: 0,
            reset: false,
            data: OrderReadResponse {
                result: OrderResultResponse {
                    data: vec![],
                    range: &market_gifts::OrderRange {
                        start: None,
                        end: None,
                    },
                },
                snapshot: None,
                sync: &sync,
                revision: "opaque-revision",
            },
        })?,
        "orders.changed" => serde_json::to_value(ServerFrame::OrdersChanged {
            id: "orders.1".into(),
            sequence: 1,
            base_sequence: 0,
            upserts: vec![],
            removed: vec![],
            sync: &sync,
            revision: "opaque-revision-2",
        })?,
        "watch_catalog" => serde_json::to_value(Command::WatchCatalog {
            id: "catalog".into(),
            collection_ids: vec!["tg.gift.123".into()],
        })?,
        "catalog" => serde_json::to_value(ServerFrame::Catalog {
            id: "catalog".into(),
            versions: CatalogVersions {
                directory: "opaque-directory-content-revision".into(),
                collections: [(
                    "tg.gift.123".into(),
                    Some("opaque-collection-content-revision".into()),
                )]
                .into(),
            },
        })?,
        "status" => serde_json::to_value(ServerFrame::Status {
            id: "offers".into(),
            state: SubscriptionStatus::Connecting,
        })?,
        "sync" => serde_json::to_value(ServerFrame::Sync {
            id: "offers".into(),
            sync,
            revision: "opaque-book-revision".into(),
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
    Ok(vec![json!({"name":name,"payload":value})])
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
        for command in ["watch_catalog", "orders.watch", "request", "cancel"] {
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
            4
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
