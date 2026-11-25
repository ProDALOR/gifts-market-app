use super::*;
fn fixture() -> w::Page {
    use w::entity::Entity as E;
    let mut data = page("1");
    data.cursor = "aliases".into();
    data.entities
        .retain(|v| matches!(&v.entity, Some(E::Venue(_) | E::Currency(_))));
    let mut push = |entity| {
        data.entities.push(w::Entity {
            entity: Some(entity),
        })
    };
    for kind in ["tg.model", "tg.backdrop", "tg.pattern"] {
        push(E::Attribute(w::Attribute {
            id: kind.into(),
            name: kind.into(),
            value_type: 1,
            ..Default::default()
        }));
    }
    for (suffix, slug) in [("1", "One"), ("2", "Two")] {
        let mut attributes = std::collections::BTreeMap::new();
        for (kind, prefix, name, alias) in [
            ("tg.model", "m", "Dogfather", "101"),
            ("tg.backdrop", "b", "Black", "42"),
            ("tg.pattern", "p", "Crown", "77"),
        ] {
            let id = format!("{prefix}{suffix}");
            attributes.insert(
                kind.into(),
                w::AttributeRule {
                    allowed_values: Some(w::AllowedAttributeValues {
                        ids: vec![id.clone()],
                    }),
                    ..Default::default()
                },
            );
            push(E::AttributeOption(w::AttributeOption {
                id,
                name: name.into(),
                attribute_id: kind.into(),
                aliases: vec![w::ExternalId {
                    namespace: "tg".into(),
                    id: alias.into(),
                }],
                ..Default::default()
            }));
        }
        push(E::ItemType(w::ItemType {
            id: format!("c{suffix}"),
            name: slug.into(),
            quantity_step: "1".into(),
            attributes,
            aliases: vec![w::ExternalId {
                namespace: "tg.slug".into(),
                id: slug.into(),
            }],
            ..Default::default()
        }));
        for (prefix, mode, price) in [
            (
                "ask",
                w::OrderMode::Ask,
                if suffix == "1" { "4" } else { "2" },
            ),
            (
                "bid",
                w::OrderMode::Bid,
                if suffix == "1" { "8" } else { "9" },
            ),
            (
                "sale",
                w::OrderMode::Executed,
                if suffix == "1" { "3" } else { "1" },
            ),
        ] {
            let mut order = w::Order {
                id: format!("{prefix}{suffix}"),
                book: Some(w::BookKey {
                    venue_id: "test".into(),
                    item_type_id: format!("c{suffix}"),
                    currency_id: "GRAM".into(),
                }),
                mode: mode as i32,
                price: price.into(),
                quantity: "1".into(),
                observed_at_ms: 1800000000000,
                executed_at_ms: (mode == w::OrderMode::Executed)
                    .then_some(1800000000000 + if suffix == "1" { 100 } else { 200 }),
                ..Default::default()
            };
            for (kind, prefix) in if mode == w::OrderMode::Bid {
                vec![("tg.backdrop", "b")]
            } else {
                vec![("tg.model", "m"), ("tg.backdrop", "b"), ("tg.pattern", "p")]
            } {
                order.attributes.insert(
                    kind.into(),
                    w::AttributePredicate {
                        predicate: Some(w::attribute_predicate::Predicate::Eq(w::AttributeValue {
                            value: Some(w::attribute_value::Value::OptionId(format!(
                                "{prefix}{suffix}"
                            ))),
                        })),
                    },
                );
            }
            push(E::Order(order));
        }
    }
    data
}
fn query(source: &str, collection: Option<&str>) -> serde_json::Value {
    serde_json::json!({"filters":{"collection_id":collection,"currency_id":"GRAM","venue_id":null,"models":[],"backdrops":["@tg:42"],"patterns":[],"number":null,"price":null},"source":source,"bid_match":if source=="bids"{"all"}else{"any"},"limit":100})
}
#[actix_web::test]
async fn aliases_share_catalog_rest_ws_matching_and_global_history_ordering() {
    let (state, runtime, peer, stop) = setup().await;
    peer.page.send_replace(fixture());
    checkpoint(&runtime, "aliases").await;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(state.clone()))
            .configure(configure),
    )
    .await;
    let directory: serde_json::Value =
        test::call_and_read_body_json(&app, test::TestRequest::get().uri("/catalog").to_request())
            .await;
    assert_eq!(directory["backdrops"].as_array().unwrap().len(), 1);
    assert_eq!(directory["patterns"].as_array().unwrap().len(), 1);
    assert!(directory["backdrops"][0]["aliases"].is_array());
    assert!(directory["backdrops"][0].get("rarity").is_none());
    let collection: serde_json::Value = test::call_and_read_body_json(
        &app,
        test::TestRequest::get()
            .uri("/collections/%40tg.slug%3AOne")
            .to_request(),
    )
    .await;
    assert_eq!(collection["collection"]["id"], "c1");
    assert!(
        collection["models"][0].get("aliases").is_none(),
        "numeric source IDs stay inside the adapter"
    );
    assert_eq!(
        collection["collection"]["aliases"][0],
        serde_json::json!({"namespace":"slug","id":"One"})
    );
    assert_eq!(directory["backdrops"][0]["aliases"][0]["namespace"], "name");
    let web_dir = peer._directory.path().join("web");
    std::fs::create_dir_all(&web_dir).unwrap();
    std::fs::write(web_dir.join("index.html"), "fixture").unwrap();
    let server = api_server(&runtime, web_dir);
    let address = server.local_addr().unwrap();
    let api_stop = CancellationToken::new();
    let job = tokio::spawn(server.run(api_stop.child_token()));
    let (mut socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}/ws"))
        .await
        .unwrap();
    for (source, expected) in [
        ("asks", vec!["ask2", "ask1"]),
        ("bids", vec!["bid2", "bid1"]),
        ("history", vec!["sale2", "sale1"]),
    ] {
        let params = query(source, None);
        let response = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/orders/query")
                .set_json(&params)
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), 200, "{source}");
        let rest: serde_json::Value = test::read_body_json(response).await;
        let ids = rest["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["id"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(ids, expected);
        bid_filters::send(&mut socket,serde_json::json!({"type":"request","method":"orders.query","id":source,"params":params})).await;
        let ws = bid_filters::frame(&mut socket, source, "result").await;
        assert_eq!(ws["data"], rest["data"]);
        // Frozen continuations retain one globally ordered page across collection boundaries.
        let mut page = params.clone();
        page["limit"] = 1.into();
        page["view"] = serde_json::json!({"kind":"snapshot"});
        let first: serde_json::Value = test::call_and_read_body_json(
            &app,
            test::TestRequest::post()
                .uri("/orders/query")
                .set_json(&page)
                .to_request(),
        )
        .await;
        assert_eq!(first["data"][0]["id"], expected[0]);
        page["range"] = serde_json::json!({"start":first["range"]["end"]});
        page["view"] = serde_json::json!({"kind":"snapshot","id":first["snapshot"]});
        let next: serde_json::Value = test::call_and_read_body_json(
            &app,
            test::TestRequest::post()
                .uri("/orders/query")
                .set_json(&page)
                .to_request(),
        )
        .await;
        assert_eq!(next["data"][0]["id"], expected[1]);
    }
    for selector in ["@slug:One", "@tg.slug:One"] {
        let mut scoped = query("asks", Some(selector));
        scoped["filters"]["backdrops"] = serde_json::json!(["@name:Black"]);
        scoped["filters"]["models"] = serde_json::json!(["@name:dogfather"]);
        let response = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/orders/query")
                .set_json(&scoped)
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);
        let response: serde_json::Value = test::read_body_json(response).await;
        assert_eq!(response["data"][0]["id"], "ask1");
    }
    let mut scoped = query("asks", Some("@tg.slug:One"));
    scoped["filters"]["models"] = serde_json::json!(["@name:Dogfather"]);
    scoped["filters"]["patterns"] = serde_json::json!(["@tg:77"]);
    let value: serde_json::Value = test::call_and_read_body_json(
        &app,
        test::TestRequest::post()
            .uri("/orders/query")
            .set_json(&scoped)
            .to_request(),
    )
    .await;
    assert_eq!(value["data"].as_array().unwrap().len(), 1);
    assert_eq!(value["data"][0]["id"], "ask1");
    scoped["filters"]["collection_id"] = serde_json::Value::Null;
    assert_eq!(
        test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/orders/query")
                .set_json(&scoped)
                .to_request()
        )
        .await
        .status(),
        400
    );
    let valuation = serde_json::json!({"currency_id":"GRAM","strategies":["ask_floor"],"target":{"kind":"selection","collection_id":"@tg.slug:One","criteria":{"models":["@name:Dogfather"],"backdrops":["@tg:42"]}}});
    let value: serde_json::Value = test::call_and_read_body_json(
        &app,
        test::TestRequest::post()
            .uri("/valuations/query")
            .set_json(&valuation)
            .to_request(),
    )
    .await;
    assert_eq!(value["target"]["collection_id"], "c1");
    assert_eq!(value["results"][0]["data"]["value"], "4");
    // Every valuation input resolves through the same typed alias boundary, with native output.
    for target in [
        serde_json::json!({"kind":"gift_attributes","collection_id":"@tg.slug:One","traits":{"model":"@name:Dogfather","backdrop":"@tg:42","pattern":"@tg:77"}}),
        serde_json::json!({"kind":"catalog","collection_ids":["@tg.slug:One"],"details":{"collection_id":"@tg.slug:One","pair":{"model_id":"@name:Dogfather","backdrop_id":"@tg:42"}}}),
    ] {
        let request = serde_json::json!({"currency_id":"GRAM","strategies":[if target["kind"] == "catalog" {"catalog_floor"} else {"gift_floor"}],"target":target});
        let response = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/valuations/query")
                .set_json(&request)
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), 200);
        let report: serde_json::Value = test::read_body_json(response).await;
        let encoded_target = report["target"].to_string();
        assert!(!encoded_target.contains('@'), "{report}");
        assert!(encoded_target.contains("c1"));
        bid_filters::send(&mut socket, serde_json::json!({"type":"request","method":"valuation.query","id":"typed-value","params":request})).await;
        let ws = bid_filters::frame(&mut socket, "typed-value", "result").await;
        assert_eq!(ws["data"]["target"], report["target"]);
    }
    let mut malformed = scoped.clone();
    malformed["filters"]["collection_id"] = "@malformed".into();
    assert_eq!(
        test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/orders/query")
                .set_json(&malformed)
                .to_request()
        )
        .await
        .status(),
        400
    );
    let (mut catalog_socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}/ws"))
        .await
        .unwrap();
    bid_filters::send(&mut catalog_socket,serde_json::json!({"type":"catalog.watch","id":"alias-catalog","collection_ids":["@tg.slug:One"]})).await;
    let manifest =
        bid_filters::frame(&mut catalog_socket, "alias-catalog", "catalog.versions").await;
    assert!(manifest["versions"]["collections"]["@tg.slug:One"].is_string());
    let watch = query("asks", Some("@slug:One"));
    bid_filters::send(
        &mut socket,
        serde_json::json!({"type":"orders.watch","id":"alias-live","params":watch}),
    )
    .await;
    let initial = bid_filters::frame(&mut socket, "alias-live", "orders.snapshot").await;
    assert_eq!(initial["data"][0]["id"], "ask1");
    let mut frozen = watch.clone();
    frozen["view"] = serde_json::json!({"kind":"snapshot"});
    let frozen_result: serde_json::Value = test::call_and_read_body_json(
        &app,
        test::TestRequest::post()
            .uri("/orders/query")
            .set_json(&frozen)
            .to_request(),
    )
    .await;
    let mut changed = fixture();
    changed.cursor = "retargeted-alias".into();
    for entity in &mut changed.entities {
        if let Some(w::entity::Entity::ItemType(item)) = &mut entity.entity {
            item.aliases[0].id = if item.id == "c1" { "Two" } else { "One" }.into();
        }
    }
    peer.page.send_replace(changed);
    checkpoint(&runtime, "retargeted-alias").await;
    let changed_manifest =
        bid_filters::frame(&mut catalog_socket, "alias-catalog", "catalog.versions").await;
    assert_ne!(
        changed_manifest["versions"]["collections"]["@tg.slug:One"],
        manifest["versions"]["collections"]["@tg.slug:One"]
    );
    let update = bid_filters::frame(&mut socket, "alias-live", "orders.changed").await;
    assert!(
        update["removed"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["id"] == "ask1")
    );
    assert!(
        update["upserts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["id"] == "ask2")
    );
    frozen["view"] = serde_json::json!({"kind":"snapshot","id":frozen_result["snapshot"]});
    let retained: serde_json::Value = test::call_and_read_body_json(
        &app,
        test::TestRequest::post()
            .uri("/orders/query")
            .set_json(&frozen)
            .to_request(),
    )
    .await;
    assert_eq!(
        retained["data"][0]["id"], "ask1",
        "aliases resolve against the retained catalog, not the current catalog"
    );
    advance_valuation_refresh().await;
    let updated: serde_json::Value = test::call_and_read_body_json(
        &app,
        test::TestRequest::post()
            .uri("/valuations/query")
            .set_json(&valuation)
            .to_request(),
    )
    .await;
    assert_eq!(updated["target"]["collection_id"], "c2");
    assert_eq!(updated["results"][0]["data"]["value"], "2");
    bid_filters::send(
        &mut socket,
        serde_json::json!({"type":"cancel","id":"alias-live"}),
    )
    .await;
    catalog_socket.close(None).await.unwrap();
    socket.close(None).await.unwrap();
    api_stop.cancel();
    job.await.unwrap().unwrap();
    stop.cancel();
    runtime.shutdown().await.unwrap();
}
