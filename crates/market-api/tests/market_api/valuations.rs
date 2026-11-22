use super::*;
#[actix_web::test]
async fn floor_reads_and_watches_share_collection_scoped_snapshots() {
    let (state, runtime, peer, stop) = setup().await;
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(state))
            .configure(configure),
    )
    .await;
    let query = serde_json::json!({"currency_id":"GRAM","strategies":["catalog_floor"],"target":{"kind":"catalog","collection_ids":["tg.gift.123"],"details":{"collection_id":"tg.gift.123","pair":null}}});
    let response = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/valuations/query")
            .set_json(&query)
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
    let rest: serde_json::Value = test::read_body_json(response).await;
    assert_eq!(
        rest["results"][0]["data"]["collections"][0]["floor"]["value"],
        "1.000000001"
    );
    for field in ["revision", "sync", "cursor"] {
        assert!(rest.get(field).is_none());
    }
    let web_dir = peer._directory.path().join("web");
    std::fs::create_dir_all(&web_dir).unwrap();
    std::fs::write(web_dir.join("index.html"), "<html></html>").unwrap();
    let server = api_server(&runtime, web_dir);
    let address = server.local_addr().unwrap();
    let api_stop = CancellationToken::new();
    let job = tokio::spawn(server.run(api_stop.child_token()));
    let (mut socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}/ws"))
        .await
        .unwrap();
    bid_filters::send(
        &mut socket,
        serde_json::json!({"type":"valuation.watch","id":"prices","params":query}),
    )
    .await;
    let ws = bid_filters::frame(&mut socket, "prices", "valuation.snapshot").await["data"].clone();
    assert!(rest.get("evaluated_at").unwrap().is_string());
    assert!(rest["results"][0]["data"]["collections"][0]["evaluated_at"].is_string());
    assert_eq!(rest["target"], ws["target"]);
    assert_eq!(rest["results"], ws["results"]);
    peer.page.send_replace(page("2"));
    // Exercise the real timer in HTTP worker runtimes, not the fixture clock.
    let changed = bid_filters::frame_with_timeout(
        &mut socket,
        "prices",
        "valuation.snapshot",
        Duration::from_secs(65),
    )
    .await;
    assert_eq!(
        changed["data"]["results"][0]["data"]["collections"][0]["floor"]["value"],
        "2"
    );
    bid_filters::send(
        &mut socket,
        serde_json::json!({"type":"cancel","id":"prices"}),
    )
    .await;
    let mut invalid = query.clone();
    invalid["target"]["collection_ids"] = serde_json::json!(["tg.gift.123", "tg.gift.123"]);
    assert_eq!(
        test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/valuations/query")
                .set_json(&invalid)
                .to_request()
        )
        .await
        .status(),
        400
    );
    bid_filters::send(
        &mut socket,
        serde_json::json!({"type":"valuation.watch","id":"prices","params":invalid}),
    )
    .await;
    assert_eq!(
        bid_filters::frame(&mut socket, "prices", "error").await["error"]["code"],
        "invalid_query"
    );
    socket.close(None).await.unwrap();
    api_stop.cancel();
    job.await.unwrap().unwrap();
    stop.cancel();
    runtime.shutdown().await.unwrap();
}
#[actix_web::test]
async fn target_strategies_share_rest_and_websocket_results_and_keep_partial_failures() {
    let (state, runtime, peer, stop) = setup().await;
    let mut data = page("3");
    data.cursor = "target-valuations".into();
    let base = data
        .entities
        .iter()
        .find_map(|entity| match &entity.entity {
            Some(w::entity::Entity::Order(row)) => Some(row.clone()),
            _ => None,
        })
        .unwrap();
    for i in 0..3 {
        let mut row = base.clone();
        row.id = format!("execution-{i}");
        row.item_id = Some(format!("executed-gift-{i}"));
        row.mode = 3;
        row.price = ((i + 1) * 10).to_string();
        row.executed_at_ms = Some(
            "2026-01-10T00:00:00Z"
                .parse::<jiff::Timestamp>()
                .unwrap()
                .as_millisecond()
                + i as i64,
        );
        data.entities.push(w::Entity {
            entity: Some(w::entity::Entity::Order(row)),
        });
    }
    peer.page.send_replace(data);
    tokio::time::timeout(Duration::from_secs(3), async {
        while runtime.reads.handle().reader().cursor().unwrap() != "target-valuations" {
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
    let q = serde_json::json!({"target":{"kind":"selection","collection_id":"tg.gift.123","criteria":{}},"currency_id":"GRAM","strategies":["ask_floor","execution_mean","execution_median","gift_floor"],"period":{"from":"2026-01-01T00:00:00Z","until":"2026-02-01T00:00:00Z"}});
    let response = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/valuations/query")
            .set_json(&q)
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
    let mut rest: serde_json::Value = test::read_body_json(response).await;
    assert_eq!(rest["results"][0]["data"]["value"], "3");
    for index in [1, 2] {
        assert_eq!(rest["results"][index]["data"]["price"]["value"], "20");
        assert_eq!(rest["results"][index]["data"]["evidence"]["gifts"], 3);
    }
    assert_eq!(rest["results"][3]["error"]["code"], "unsupported_target");
    assert!(rest.get("revision").is_none());
    assert!(rest.get("sync").is_none());
    let web_dir = peer._directory.path().join("web");
    std::fs::create_dir_all(&web_dir).unwrap();
    std::fs::write(web_dir.join("index.html"), "<html></html>").unwrap();
    let server = api_server(&runtime, web_dir);
    let address = server.local_addr().unwrap();
    let api_stop = CancellationToken::new();
    let job = tokio::spawn(server.run(api_stop.child_token()));
    let (mut socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}/ws"))
        .await
        .unwrap();
    bid_filters::send(
        &mut socket,
        serde_json::json!({"type":"request","id":"target-1","method":"valuation.query","params":q}),
    )
    .await;
    let mut ws = bid_filters::frame(&mut socket, "target-1", "result").await["data"].clone();
    rest.as_object_mut().unwrap().remove("evaluated_at");
    ws.as_object_mut().unwrap().remove("evaluated_at");
    assert_eq!(rest, ws);
    bid_filters::send(
        &mut socket,
        serde_json::json!({"type":"valuation.watch","id":"target-live","params":q}),
    )
    .await;
    let mut initial =
        bid_filters::frame(&mut socket, "target-live", "valuation.snapshot").await["data"].clone();
    initial.as_object_mut().unwrap().remove("evaluated_at");
    assert_eq!(rest, initial);
    let mut changed = peer.page.borrow().clone();
    changed.cursor = "history-backfill".into();
    let mut execution = base;
    execution.id = "execution-3".into();
    execution.item_id = Some("executed-gift-3".into());
    execution.mode = 3;
    execution.price = "100".into();
    execution.executed_at_ms = Some(
        "2026-01-15T00:00:00Z"
            .parse::<jiff::Timestamp>()
            .unwrap()
            .as_millisecond(),
    );
    changed.entities.push(w::Entity {
        entity: Some(w::entity::Entity::Order(execution)),
    });
    peer.page.send_replace(changed);
    let updated = bid_filters::frame_with_timeout(
        &mut socket,
        "target-live",
        "valuation.snapshot",
        Duration::from_secs(65),
    )
    .await;
    assert_eq!(
        updated["data"]["results"][1]["data"]["price"]["value"],
        "40"
    );
    assert_eq!(
        updated["data"]["results"][2]["data"]["price"]["value"],
        "25"
    );
    assert_eq!(
        updated["data"]["results"][3]["error"]["code"],
        "unsupported_target"
    );
    bid_filters::send(
        &mut socket,
        serde_json::json!({"type":"cancel","id":"target-live"}),
    )
    .await;
    let mut invalid = q.clone();
    invalid.as_object_mut().unwrap().remove("period");
    assert_eq!(
        test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/valuations/query")
                .set_json(&invalid)
                .to_request()
        )
        .await
        .status(),
        400
    );
    socket.close(None).await.unwrap();
    api_stop.cancel();
    job.await.unwrap().unwrap();
    stop.cancel();
    runtime.shutdown().await.unwrap();
}
