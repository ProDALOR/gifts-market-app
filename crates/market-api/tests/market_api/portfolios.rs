use super::*;
#[derive(Clone)]
struct Inventory {
    calls: Arc<AtomicUsize>,
    count: usize,
    release: Arc<tokio::sync::Semaphore>,
}
impl market_portfolio::InventoryProvider for Inventory {
    fn page(
        &self,
        request: market_portfolio::InventoryPageRequest,
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
                let request = request.clone();
                async move {
                    this.calls.fetch_add(1, Ordering::SeqCst);
                    this.release.acquire().await.unwrap().forget();
                    let offset: usize = request.cursor.as_deref().unwrap_or("0").parse().unwrap();
                    let end = (offset + request.limit).min(this.count);
                    Ok(market_portfolio::InventoryPage {
                        gifts: (offset..end)
                            .map(|i| {
                                let number = (this.count - i) as u32;
                                market_portfolio::InventoryGift {
                                    gift: market_gifts::TelegramGiftDetails {
                                        id: format!("gift-{number}").try_into().unwrap(),
                                        slug: format!("Gift-{number}").try_into().unwrap(),
                                        number: number.try_into().unwrap(),
                                        collection: market_gifts::TelegramGiftId::new(1).unwrap(),
                                        model: market_gifts::TelegramDocumentId::new(2).unwrap(),
                                        backdrop: None,
                                        pattern: None,
                                    },
                                    relation: market_portfolio::GiftRelation::Owned,
                                }
                            })
                            .collect(),
                        scanned: end - offset,
                        reported_total: Some(this.count),
                        next_cursor: (end < this.count).then(|| end.to_string()),
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
            count: 0,
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
    for user in [
        "0",
        "-1",
        "9007199254740992",
        "not-a-user",
        "18446744073709551616",
    ] {
        for request in [
            test::TestRequest::get().uri(&format!("/portfolios/{user}?currency_id=GRAM")),
            test::TestRequest::post()
                .uri(&format!("/portfolios/{user}/refresh"))
                .set_json(serde_json::json!({"currency_id":"GRAM"})),
        ] {
            let response = test::call_service(&app, request.to_request()).await;
            assert_eq!(response.status(), 400, "{user}");
            let error: serde_json::Value = test::read_body_json(response).await;
            assert_eq!(error["error"]["code"], "invalid_query");
        }
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "invalid users never reach inventory"
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
    assert!(pending["data"].get("refresh_after").is_none());
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
    let checked: jiff::Timestamp = ready["inventory"]["checked_at"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let next: jiff::Timestamp = ready["refresh_after"].as_str().unwrap().parse().unwrap();
    assert!(
        (299_000..=301_000).contains(&(next.as_millisecond() - checked.as_millisecond())),
        "the public recheck deadline starts five minutes after completion"
    );
    bid_filters::send(&mut socket, serde_json::json!({"type":"cancel","id":"p1"})).await;
    bid_filters::send(
        &mut socket,
        serde_json::json!({"type":"portfolio.watch","id":"p2","params":params}),
    )
    .await;
    let reopened = bid_filters::frame(&mut socket, "p2", "portfolio.snapshot").await;
    assert_eq!(reopened["data"]["valuation"], ready["valuation"]);
    assert_eq!(reopened["data"]["refresh_after"], ready["refresh_after"]);
    bid_filters::send(&mut socket,serde_json::json!({"type":"request","id":"query","method":"portfolio.query","params":params})).await;
    let finite = bid_filters::frame(&mut socket, "query", "result").await;
    assert_eq!(finite["method"], "portfolio.query");
    assert_eq!(
        finite["data"]["selection"]["total"],
        ready["valuation"]["total"]
    );
    assert_eq!(finite["data"]["version"], ready["valuation"]["version"]);
    assert_eq!(finite["data"]["items"], serde_json::json!([]));
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

#[derive(Clone)]
struct Values;
impl market_portfolio::ValuationProvider for Values {
    async fn value(
        &self,
        gifts: Arc<Vec<market_portfolio::InventoryGift>>,
        currency: market_gifts::CurrencyId,
        progress: tokio::sync::watch::Sender<usize>,
    ) -> Result<Arc<market_valuation::InventoryValuation>, market_valuation::ValuationError> {
        let mut total = market_gifts::Amount::from("0").decimal();
        let items = gifts
            .iter()
            .map(|gift| {
                let estimate =
                    market_gifts::Amount::new(format!("{}.01", gift.gift.number)).unwrap();
                total += estimate.decimal();
                market_valuation::InventoryValue {
                    gift: gift.clone(),
                    resolved: Some(market_gifts::CollectibleGift {
                        id: gift.gift.id.clone(),
                        collection_id: "gift".into(),
                        number: Some(gift.gift.number),
                        traits: market_gifts::GiftTraits {
                            model: Some(if gift.gift.number.get() % 2 == 0 {
                                "m".into()
                            } else {
                                "otherm".into()
                            }),
                            backdrop: None,
                            pattern: None,
                        },
                    }),
                    estimate: Some(estimate),
                    error: None,
                }
            })
            .collect();
        progress.send_replace(gifts.len());
        Ok(Arc::new(market_valuation::InventoryValuation {
            currency_id: currency,
            evaluated_at: jiff::Timestamp::now(),
            stale: false,
            total: Some(total.try_into().unwrap()),
            priced: gifts.len(),
            unpriced: 0,
            items,
        }))
    }
}
#[actix_web::test]
async fn large_portfolio_watch_stays_small_and_rest_ws_pages_share_global_filters() {
    let (state, runtime, peer, stop) = setup().await;
    let calls = Arc::new(AtomicUsize::new(0));
    let invocations = market_invocations::Runtime::start(Default::default()).unwrap();
    let portfolio = market_portfolio::PortfolioService::start(
        Inventory {
            count: 6000,
            calls: calls.clone(),
            release: Arc::new(tokio::sync::Semaphore::new(100)),
        },
        Values,
        Default::default(),
        invocations.handle(),
    )
    .await
    .unwrap();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(
                state.with_portfolio(portfolio.handle.clone()),
            ))
            .configure(configure),
    )
    .await;
    let web_dir = peer._directory.path().join("portfolio-page-web");
    std::fs::create_dir_all(&web_dir).unwrap();
    std::fs::write(web_dir.join("index.html"), "<html></html>").unwrap();
    let server = api_server(&runtime, web_dir).with_portfolio(portfolio.handle.clone());
    let address = server.local_addr().unwrap();
    let api_stop = CancellationToken::new();
    let job = tokio::spawn(server.run(api_stop.child_token()));
    let (mut socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}/ws"))
        .await
        .unwrap();
    bid_filters::send(&mut socket, serde_json::json!({"type":"portfolio.watch","id":"p","params":{"user_id":7,"currency_id":"GRAM"}})).await;
    let ready = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let frame = bid_filters::frame(&mut socket, "p", "portfolio.snapshot").await;
            if frame["data"]["progress"]["stage"] == "ready" {
                break frame["data"].clone();
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(ready["valuation"]["count"], 6000);
    assert!(ready["valuation"].get("items").is_none());
    assert!(
        serde_json::to_vec(&ready).unwrap().len() < 2048,
        "watch size does not grow with gift rows"
    );
    let version = ready["valuation"]["version"].clone();
    let query = serde_json::json!({"user_id":7,"currency_id":"GRAM","sort":"position_asc","version":version,"offset":100});
    let response = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/portfolios/query")
            .set_json(&query)
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), 200);
    let rest: serde_json::Value = test::read_body_json(response).await;
    assert_eq!(rest["items"].as_array().unwrap().len(), 100);
    assert_eq!(rest["items"][0]["gift"]["gift"]["number"], 5900);
    assert_eq!(rest["selection"]["count"], 6000);
    bid_filters::send(
        &mut socket,
        serde_json::json!({"type":"request","id":"page","method":"portfolio.query","params":query}),
    )
    .await;
    assert_eq!(
        bid_filters::frame(&mut socket, "page", "result").await["data"],
        rest
    );
    let filtered = serde_json::json!({"user_id":7,"currency_id":"GRAM","filters":{"models":["otherm"],"number":1}});
    let response = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/portfolios/query")
            .set_json(&filtered)
            .to_request(),
    )
    .await;
    let data: serde_json::Value = test::read_body_json(response).await;
    assert_eq!(data["selection"]["count"], 1);
    assert_eq!(data["selection"]["total"], "1.01");
    let mut invalid = query.clone();
    invalid["limit"] = 101.into();
    assert_eq!(
        test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/portfolios/query")
                .set_json(&invalid)
                .to_request()
        )
        .await
        .status(),
        400
    );
    let mut stale = query.clone();
    stale["version"] = serde_json::json!("00000000000000000000000000000000");
    assert_eq!(
        test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/portfolios/query")
                .set_json(&stale)
                .to_request()
        )
        .await
        .status(),
        409
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        60,
        "pagination never rechecks Telegram inventory"
    );
    socket.close(None).await.unwrap();
    api_stop.cancel();
    job.await.unwrap().unwrap();
    portfolio.task.shutdown().await.unwrap();
    invocations.shutdown().await;
    stop.cancel();
    runtime.shutdown().await.unwrap();
}
