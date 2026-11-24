use super::*;
#[derive(Clone)]
struct Inventory {
    calls: Arc<AtomicUsize>,
    release: Arc<tokio::sync::Semaphore>,
}
impl market_portfolio::InventoryProvider for Inventory {
    fn page(
        &self,
        _: market_portfolio::InventoryPageRequest,
    ) -> impl market_invocations::Invocation<
        Output = market_portfolio::InventoryPage,
        Error = market_portfolio::InventoryError,
    > {
        let this = self.clone();
        market_invocations::operation(
            market_invocations::Policy {
                limits: vec![],
                replay: market_invocations::Replay::Safe,
                attempt_timeout: Duration::from_secs(5),
                max_attempts: 1,
            },
            move || {
                let this = this.clone();
                async move {
                    this.calls.fetch_add(1, Ordering::SeqCst);
                    this.release.acquire().await.unwrap().forget();
                    Ok(market_portfolio::InventoryPage {
                        reported_total: Some(0),
                        ..Default::default()
                    })
                }
            },
        )
    }
}
#[actix_web::test]
async fn portfolio_rest_and_ws_share_inventory_progress_and_reconnect_cache() {
    let (state, runtime, peer, stop) = setup().await;
    let without = test::init_service(
        App::new()
            .app_data(web::Data::new(state.clone()))
            .configure(configure),
    )
    .await;
    assert_eq!(
        test::call_service(
            &without,
            test::TestRequest::get()
                .uri("/portfolios/7?currency_id=GRAM")
                .to_request()
        )
        .await
        .status(),
        503
    );
    let calls = Arc::new(AtomicUsize::new(0));
    let release = Arc::new(tokio::sync::Semaphore::new(0));
    let invocations = market_invocations::Runtime::start(Default::default()).unwrap();
    let portfolio = market_portfolio::PortfolioService::start(
        Inventory {
            calls: calls.clone(),
            release: release.clone(),
        },
        runtime.valuation.clone(),
        Default::default(),
        invocations.handle(),
    )
    .await
    .unwrap();
    let state = state.with_portfolio(portfolio.handle.clone());
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(state))
            .configure(configure),
    )
    .await;
    let response = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/portfolios/7?currency_id=GRAM")
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
    let idle: serde_json::Value = test::read_body_json(response).await;
    assert_eq!(idle["progress"]["stage"], "idle");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/portfolios/0?currency_id=GRAM")
                .to_request()
        )
        .await
        .status(),
        400
    );
    let web_dir = peer._directory.path().join("portfolio-web");
    std::fs::create_dir_all(&web_dir).unwrap();
    std::fs::write(web_dir.join("index.html"), "<html></html>").unwrap();
    let server = api_server(&runtime, web_dir).with_portfolio(portfolio.handle.clone());
    let address = server.local_addr().unwrap();
    let api_stop = CancellationToken::new();
    let job = tokio::spawn(server.run(api_stop.child_token()));
    let (mut socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}/ws"))
        .await
        .unwrap();
    let params = serde_json::json!({"user_id":7,"currency_id":"GRAM"});
    bid_filters::send(
        &mut socket,
        serde_json::json!({"type":"portfolio.watch","id":"p1","params":params}),
    )
    .await;
    let pending = bid_filters::frame(&mut socket, "p1", "portfolio.snapshot").await;
    assert!(matches!(
        pending["data"]["progress"]["stage"].as_str(),
        Some("queued" | "loading_inventory")
    ));
    let response = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/portfolios/7/refresh")
            .set_json(serde_json::json!({"currency_id":"GRAM"}))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), 200);
    release.add_permits(1);
    let ready = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let frame = bid_filters::frame(&mut socket, "p1", "portfolio.snapshot").await;
            if frame["data"]["progress"]["stage"] == "ready" {
                break frame["data"].clone();
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(ready["inventory"]["count"], 0);
    assert_eq!(ready["valuation"]["total"], "0");
    bid_filters::send(&mut socket, serde_json::json!({"type":"cancel","id":"p1"})).await;
    bid_filters::send(
        &mut socket,
        serde_json::json!({"type":"portfolio.watch","id":"p2","params":params}),
    )
    .await;
    let reopened = bid_filters::frame(&mut socket, "p2", "portfolio.snapshot").await;
    assert_eq!(reopened["data"]["valuation"], ready["valuation"]);
    bid_filters::send(&mut socket,serde_json::json!({"type":"request","id":"query","method":"portfolio.query","params":params})).await;
    let finite = bid_filters::frame(&mut socket, "query", "result").await;
    assert_eq!(finite["method"], "portfolio.query");
    assert_eq!(finite["data"]["valuation"], ready["valuation"]);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/portfolios/7/refresh")
                .set_json(serde_json::json!({"currency_id":"GRAM"}))
                .to_request()
        )
        .await
        .status(),
        429
    );
    socket.close(None).await.unwrap();
    api_stop.cancel();
    job.await.unwrap().unwrap();
    portfolio.task.shutdown().await.unwrap();
    invocations.shutdown().await;
    stop.cancel();
    runtime.shutdown().await.unwrap();
}
