use super::*;
use w::{attribute_predicate::Predicate as P, attribute_value::Value as V, entity::Entity as E};
fn value(id: &str) -> w::AttributeValue {
    w::AttributeValue {
        value: Some(V::OptionId(id.into())),
    }
}
fn choices(ids: &[&str]) -> w::AttributePredicate {
    w::AttributePredicate {
        predicate: Some(if ids.len() == 1 {
            P::Eq(value(ids[0]))
        } else {
            P::OneOf(w::AttributeValues {
                values: ids.iter().map(|id| value(id)).collect(),
            })
        }),
    }
}
fn numbers(min: i64, max: i64) -> w::AttributePredicate {
    w::AttributePredicate {
        predicate: Some(P::Range(w::AttributeRange {
            min: Some(w::AttributeValue {
                value: Some(V::Integer(min)),
            }),
            max: Some(w::AttributeValue {
                value: Some(V::Integer(max)),
            }),
        })),
    }
}
fn data() -> w::Page {
    let mut data = page("1");
    data.cursor = "bid-filter-initial".into();
    for entity in &mut data.entities {
        if let Some(E::ItemType(item)) = &mut entity.entity {
            for id in ["tg.model", "tg.backdrop", "tg.pattern", "tg.number"] {
                item.attributes.insert(id.into(), Default::default());
            }
        }
    }
    for (attribute, ids) in [
        ("tg.model", vec!["gold", "silver"]),
        ("tg.backdrop", vec!["blue", "red"]),
        ("tg.pattern", vec!["star"]),
    ] {
        data.entities.push(w::Entity {
            entity: Some(E::Attribute(w::Attribute {
                id: attribute.into(),
                name: attribute.into(),
                value_type: 1,
                ..Default::default()
            })),
        });
        for id in ids {
            data.entities.push(w::Entity {
                entity: Some(E::AttributeOption(w::AttributeOption {
                    id: id.into(),
                    name: id.into(),
                    attribute_id: attribute.into(),
                    ..Default::default()
                })),
            });
        }
    }
    data.entities.push(w::Entity {
        entity: Some(E::Attribute(w::Attribute {
            id: "tg.number".into(),
            name: "Number".into(),
            value_type: 2,
            ..Default::default()
        })),
    });
    for (id, price, models, backdrop, number) in [
        ("any", "100", vec![], None, None),
        ("gold", "90", vec!["gold"], None, None),
        ("both", "80", vec!["gold", "silver"], None, None),
        ("silver", "70", vec!["silver"], None, None),
        (
            "range",
            "60",
            vec!["gold", "silver"],
            Some("blue"),
            Some((10, 20)),
        ),
        ("red", "50", vec!["gold", "silver"], Some("red"), None),
    ] {
        let mut row = w::Order {
            id: id.into(),
            book: Some(book()),
            mode: 2,
            price: price.into(),
            quantity: "1".into(),
            ..Default::default()
        };
        if !models.is_empty() {
            row.attributes.insert("tg.model".into(), choices(&models));
        }
        if let Some(backdrop) = backdrop {
            row.attributes
                .insert("tg.backdrop".into(), choices(&[backdrop]));
        }
        if let Some((min, max)) = number {
            row.attributes.insert("tg.number".into(), numbers(min, max));
        }
        data.entities.push(w::Entity {
            entity: Some(E::Order(row)),
        });
    }
    data.entities.push(w::Entity {
        entity: Some(E::Order(w::Order {
            id: "complex".into(),
            book: Some(book()),
            mode: 2,
            price: "40".into(),
            quantity: "1".into(),
            attributes: [(
                "tg.model".into(),
                w::AttributePredicate {
                    predicate: Some(P::AllOf(w::AttributePredicates {
                        predicates: vec![choices(&["gold"])],
                    })),
                },
            )]
            .into(),
            ..Default::default()
        })),
    });
    data
}
fn bid_query(mode: &str) -> serde_json::Value {
    serde_json::json!({"source":"bids","bid_match":mode,"limit":100,"filters":{"collection_id":"tg.gift.123","currency_id":"GRAM","models":["gold","silver"]}})
}
fn row_ids(result: &serde_json::Value) -> Vec<String> {
    result["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["id"].as_str().unwrap().to_owned())
        .collect()
}
type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
async fn frame(socket: &mut Socket, id: &str, kind: &str) -> serde_json::Value {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let tokio_tungstenite::tungstenite::Message::Text(text) =
                socket.next().await.unwrap().unwrap()
            {
                let frame: serde_json::Value = serde_json::from_str(&text).unwrap();
                if frame["id"] == id {
                    assert_ne!(frame["type"], "error", "{frame}");
                    if frame["type"] == kind {
                        break frame;
                    }
                }
            }
        }
    })
    .await
    .unwrap()
}
async fn send(socket: &mut Socket, body: serde_json::Value) {
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            body.to_string().into(),
        ))
        .await
        .unwrap();
}

#[actix_web::test]
async fn bid_matching_is_shared_by_rest_websocket_pagination_and_live_updates() {
    let (state, runtime, peer, stop) = setup().await;
    let mut data = data();
    let cursor = data.cursor.clone();
    peer.page.send_replace(data.clone());
    tokio::time::timeout(Duration::from_secs(3), async {
        while runtime.reads.handle().reader().cursor().unwrap() != cursor {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(state))
            .configure(configure),
    )
    .await;
    let web_dir = peer._directory.path().join("web");
    std::fs::create_dir_all(&web_dir).unwrap();
    std::fs::write(web_dir.join("index.html"), "<html></html>").unwrap();
    let server = api_server(&runtime, web_dir);
    let address = server.local_addr().unwrap();
    let api_stop = CancellationToken::new();
    let api_task = tokio::spawn(server.run(api_stop.child_token()));
    let (mut socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}/ws"))
        .await
        .unwrap();
    for (mode, expected) in [
        ("any", vec!["any", "gold", "both", "silver", "range", "red"]),
        ("all", vec!["both"]),
    ] {
        let q = bid_query(mode);
        let rest: serde_json::Value = test::call_and_read_body_json(
            &app,
            test::TestRequest::post()
                .uri("/orders/query")
                .set_json(&q)
                .to_request(),
        )
        .await;
        assert_eq!(row_ids(&rest), expected);
        send(
            &mut socket,
            serde_json::json!({"type":"request","id":mode,"method":"orders.query","params":q}),
        )
        .await;
        let ws = frame(&mut socket, mode, "result").await;
        assert_eq!(ws["data"], rest["data"]);
        assert_eq!(ws["range"], rest["range"]);
        let mut frozen = q.clone();
        frozen["view"] = serde_json::json!({"kind":"snapshot"});
        let frozen: serde_json::Value = test::call_and_read_body_json(
            &app,
            test::TestRequest::post()
                .uri("/orders/query")
                .set_json(&frozen)
                .to_request(),
        )
        .await;
        assert_eq!(frozen["data"], rest["data"]);
        let release = test::call_service(
            &app,
            test::TestRequest::delete()
                .uri("/orders/query")
                .set_json(serde_json::json!({"snapshot":frozen["snapshot"]}))
                .to_request(),
        )
        .await;
        assert!(release.status().is_success());
        for sort in [
            "identity",
            "price_asc",
            "price_desc",
            "time_asc",
            "time_desc",
            "number_asc",
            "number_desc",
        ] {
            let mut paged = q.clone();
            paged["limit"] = 2.into();
            paged["sort"] = sort.into();
            let mut seen = vec![];
            loop {
                let page: serde_json::Value = test::call_and_read_body_json(
                    &app,
                    test::TestRequest::post()
                        .uri("/orders/query")
                        .set_json(&paged)
                        .to_request(),
                )
                .await;
                seen.extend(row_ids(&page));
                if page["range"]["end"].is_null() {
                    break;
                }
                paged["range"] = serde_json::json!({"start":page["range"]["end"],"end":null});
            }
            seen.sort();
            let mut wanted = expected.clone();
            wanted.sort();
            assert_eq!(seen, wanted, "{mode} {sort}");
        }
        let mut numbered = q.clone();
        numbered["filters"]["number"] = serde_json::json!({"min":15,"max":25});
        let page: serde_json::Value = test::call_and_read_body_json(
            &app,
            test::TestRequest::post()
                .uri("/orders/query")
                .set_json(&numbered)
                .to_request(),
        )
        .await;
        assert_eq!(
            row_ids(&page),
            if mode == "any" {
                vec!["any", "gold", "both", "silver", "range", "red"]
            } else {
                vec![]
            }
        );
        let expected_numbers: std::collections::BTreeSet<_> = row_ids(&page).into_iter().collect();
        for sort in ["number_asc", "number_desc"] {
            numbered["sort"] = sort.into();
            let page: serde_json::Value = test::call_and_read_body_json(
                &app,
                test::TestRequest::post()
                    .uri("/orders/query")
                    .set_json(&numbered)
                    .to_request(),
            )
            .await;
            assert_eq!(
                row_ids(&page)
                    .into_iter()
                    .collect::<std::collections::BTreeSet<_>>(),
                expected_numbers,
                "Unrestricted bids must survive number-sort index bounds"
            );
        }
        let mut empty = q;
        empty["filters"] =
            serde_json::json!({"collection_id":"tg.gift.123","currency_id":"GRAM","number":{}});
        let page: serde_json::Value = test::call_and_read_body_json(
            &app,
            test::TestRequest::post()
                .uri("/orders/query")
                .set_json(&empty)
                .to_request(),
        )
        .await;
        if mode == "any" {
            assert_eq!(page["data"].as_array().unwrap().len(), 7);
            assert!(
                page["data"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|row| row["id"] == "complex" && row["target"]["kind"] == "unsupported")
            );
        } else {
            assert_eq!(row_ids(&page), ["any"]);
        }
    }
    // Empty selections mean absent restrictions in All, not wildcards.
    // Both trait values and number bounds must match, regardless of predicate encoding.
    for (filters, expected) in [
        (serde_json::json!({"models":["gold"]}), vec!["gold"]),
        (
            serde_json::json!({"models":["silver","gold","gold"],"backdrops":[],"patterns":[]}),
            vec!["both"],
        ),
        (
            serde_json::json!({"models":["gold","silver"],"backdrops":["red"]}),
            vec!["red"],
        ),
        (
            serde_json::json!({"models":["gold","silver"],"backdrops":["blue"]}),
            vec![],
        ),
        (
            serde_json::json!({"models":["gold","silver"],"backdrops":["blue"],"number":{"min":10,"max":20}}),
            vec!["range"],
        ),
        (
            serde_json::json!({"models":["gold","silver"],"backdrops":["blue"],"number":{"min":11,"max":20}}),
            vec![],
        ),
        (
            serde_json::json!({"models":[],"backdrops":["blue"]}),
            vec![],
        ),
        (
            serde_json::json!({"models":["gold","silver"],"patterns":["star"]}),
            vec![],
        ),
    ] {
        let mut q = bid_query("all");
        q["filters"] = filters;
        q["filters"]["collection_id"] = "tg.gift.123".into();
        q["filters"]["currency_id"] = "GRAM".into();
        let rest: serde_json::Value = test::call_and_read_body_json(
            &app,
            test::TestRequest::post()
                .uri("/orders/query")
                .set_json(&q)
                .to_request(),
        )
        .await;
        assert_eq!(row_ids(&rest), expected, "{q}");
        send(
            &mut socket,
            serde_json::json!({"type":"request","id":"exact","method":"orders.query","params":q}),
        )
        .await;
        assert_eq!(
            row_ids(&frame(&mut socket, "exact", "result").await),
            expected
        );
    }
    for (source, mode) in [("asks", "all"), ("history", "all"), ("bids", "invalid")] {
        let mut invalid = bid_query(mode);
        invalid["source"] = source.into();
        let response = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/orders/query")
                .set_json(&invalid)
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), actix_web::http::StatusCode::BAD_REQUEST);
    }
    // Empty Any and All selections must not share a feed: one browses all bids,
    // the other matches only completely unrestricted conditions.
    for (mode, id, expected) in [("any", "empty-any", 7), ("all", "empty-all", 1)] {
        let mut q = bid_query(mode);
        q["filters"] = serde_json::json!({"collection_id":"tg.gift.123","currency_id":"GRAM"});
        send(
            &mut socket,
            serde_json::json!({"type":"orders.watch","id":id,"params":q}),
        )
        .await;
        let initial = frame(&mut socket, id, "orders.snapshot").await;
        assert_eq!(row_ids(&initial).len(), expected);
        if mode == "all" {
            assert_eq!(row_ids(&initial), ["any"]);
        }
    }
    for id in ["empty-any", "empty-all"] {
        send(&mut socket, serde_json::json!({"type":"cancel","id":id})).await;
    }
    send(
        &mut socket,
        serde_json::json!({"type":"orders.watch","id":"any-live","params":bid_query("any")}),
    )
    .await;
    assert_eq!(
        row_ids(&frame(&mut socket, "any-live", "orders.snapshot").await),
        ["any", "gold", "both", "silver", "range", "red"]
    );
    send(
        &mut socket,
        serde_json::json!({"type":"orders.watch","id":"live","params":bid_query("all")}),
    )
    .await;
    assert_eq!(
        row_ids(&frame(&mut socket, "live", "orders.snapshot").await),
        ["both"]
    );
    // The identical scope/range has different local options and must not join Any's producer.
    send(
        &mut socket,
        serde_json::json!({"type":"orders.watch","id":"all-join","params":bid_query("all")}),
    )
    .await;
    assert_eq!(
        row_ids(&frame(&mut socket, "all-join", "orders.snapshot").await),
        ["both"]
    );
    send(
        &mut socket,
        serde_json::json!({"type":"cancel","id":"any-live"}),
    )
    .await;
    send(
        &mut socket,
        serde_json::json!({"type":"cancel","id":"all-join"}),
    )
    .await;
    for entity in &mut data.entities {
        if let Some(E::Order(row)) = &mut entity.entity {
            if row.id == "gold" {
                row.attributes
                    .insert("tg.model".into(), choices(&["gold", "silver"]));
            }
            if row.id == "both" {
                row.attributes
                    .insert("tg.backdrop".into(), choices(&["red"]));
            }
        }
    }
    data.cursor = "bid-filter-update".into();
    peer.page.send_replace(data);
    let update = frame(&mut socket, "live", "orders.changed").await;
    assert!(
        update["upserts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["id"] == "gold")
    );
    assert!(
        update["removed"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["id"] == "both")
    );
    let schema: serde_json::Value =
        serde_json::from_str(&market_api::schema_json().unwrap()).unwrap();
    assert_eq!(
        schema["components"]["schemas"]["BidMatch"]["enum"],
        serde_json::json!(["any", "all"])
    );
    assert!(
        schema["components"]["schemas"]["GiftOrderQuery"]["properties"]
            .get("bid_match")
            .is_some()
    );
    socket.close(None).await.unwrap();
    api_stop.cancel();
    api_task.await.unwrap().unwrap();
    stop.cancel();
    runtime.shutdown().await.unwrap();
}
