use crate::*;
use market_telegram::IncomingMessage;
fn message(id: i32, text: &str) -> IncomingMessage<'_> {
    IncomingMessage {
        private: true,
        outgoing: false,
        user_id: Some(7),
        message_id: id,
        text: Some(text),
    }
}
#[test]
fn floor_and_portfolio_arguments_are_typed_without_overriding_caller_identity() {
    let request = commands::parse(
        message(21, "/floor@MARKET_BOT Bunny Muffin | Red Alert | TON"),
        &["market_bot"],
    )
    .unwrap();
    assert_eq!(request.user_id, 7);
    let Command::Floor(query) = request.command else {
        panic!("expected floor")
    };
    assert_eq!(
        query.model,
        market_gifts::valuation::ModelLookup::named("Bunny Muffin", "Red Alert").unwrap()
    );
    assert_eq!(query.currency.as_str(), "GRAM");
    let request = commands::parse(message(22, "/portfolio TON"), &["market_bot"]).unwrap();
    assert_eq!(request.user_id, 7);
    let Command::Portfolio(currency) = request.command else {
        panic!("expected portfolio")
    };
    assert_eq!(currency.as_str(), "GRAM");
    for command in [
        "/floor bad",
        "/floor A | B | GRAM | XTR",
        "/portfolio GRAM user_id=999",
    ] {
        let request = commands::parse(message(23, command), &["market_bot"]).unwrap();
        assert!(matches!(request.command, Command::Invalid(_)));
        assert_eq!(request.user_id, 7);
    }
}
#[test]
fn floor_commands_reuse_domain_name_limits_and_keep_currency_shorthand_local() {
    use market_gifts::valuation::ModelLookup;
    for name in ["x".repeat(256), "💎".repeat(64)] {
        let text = format!("/floor  Collection | {name} | xtr ");
        let request = commands::parse(message(1, &text), &["market_bot"]).unwrap();
        let Command::Floor(query) = request.command else {
            panic!("expected floor")
        };
        assert_eq!(query.model, ModelLookup::named("Collection", name).unwrap());
        assert_eq!(query.currency.as_str(), "XTR");
    }
    for name in ["".to_owned(), "x".repeat(257), "💎".repeat(65)] {
        let text = format!("/floor Collection | {name}");
        let request = commands::parse(message(1, &text), &["market_bot"]).unwrap();
        assert!(matches!(
            request.command,
            Command::Invalid(CommandKind::Floor)
        ));
    }
    for text in [
        "/floor Collection | Model",
        "/floor Collection | Model | ton",
        "/floor Collection | Model | ",
    ] {
        let request = commands::parse(message(1, text), &["market_bot"]).unwrap();
        let Command::Floor(query) = request.command else {
            panic!("expected floor")
        };
        assert_eq!(query.currency.as_str(), "GRAM");
    }
}

use market_portfolio::{InventoryError, InventoryPage, InventoryPageRequest, InventoryProvider};
use market_telegram::{BotError, MessageHandler};
#[derive(Clone)]
struct Source(InventoryError);
impl InventoryProvider for Source {
    fn page(
        &self,
        _: InventoryPageRequest,
    ) -> impl market_invocations::Invocation<Output = InventoryPage, Error = InventoryError> {
        let error = self.0.clone();
        market_invocations::operation(
            market_invocations::Policy {
                limits: vec![],
                replay: market_invocations::Replay::Never,
                attempt_timeout: std::time::Duration::from_secs(1),
                max_attempts: 1,
            },
            move || {
                let error = error.clone();
                async move {
                    Err(market_invocations::AttemptError {
                        error,
                        retry: market_invocations::Retry::Never,
                    })
                }
            },
        )
    }
}
async fn services(
    data: market_actors::MarketDataHandle,
    error: InventoryError,
) -> (
    market_invocations::Runtime,
    market_valuation::Service<
        market_gifts::valuation::ValuationRequest,
        market_gifts::valuation::Valuation,
        market_actors::error::DataError,
    >,
    market_portfolio::PortfolioService,
) {
    let invocations = market_invocations::Runtime::start(Default::default()).unwrap();
    let valuation = market_valuation::Service::start(
        market_actors::valuation::ModelFloorStrategy::new(data),
        Default::default(),
    )
    .unwrap();
    let portfolio = market_portfolio::PortfolioService::start(
        Source(error),
        valuation.handle.clone(),
        Default::default(),
        invocations.handle(),
    )
    .await
    .unwrap();
    (invocations, valuation, portfolio)
}
#[tokio::test]
async fn bot_status_reads_the_shared_actor_and_portfolio_permission_failure_is_a_user_reply() {
    let directory = tempfile::tempdir().unwrap();
    let writer = market_replica::ReplicaWriter::open(directory.path(), "test").unwrap();
    let data = market_actors::MarketDataService::start(writer.reader(), 1, 1)
        .await
        .unwrap();
    let (_invocations, _valuation, portfolio) =
        services(data.handle.clone(), InventoryError::Forbidden).await;
    let handler = MarketCommands::new(
        data.handle.clone(),
        portfolio.handle.clone(),
        _valuation.handle.clone(),
    );
    let names = vec!["market_bot".to_owned(), "market_alias".to_owned()];
    let request = handler
        .parse(message(1, "/portfolio@MARKET_ALIAS"), &names)
        .unwrap();
    let reply = handler.handle(request).await.unwrap();
    assert!(reply.text.contains("Telegram did not allow"));
    assert!(!reply.text.contains("estimate: 0"));
    let request = handler.parse(message(2, "/status"), &names).unwrap();
    assert_eq!(
        handler.handle(request).await.unwrap().text,
        "Market data: starting. Preparing market data. Please check again shortly."
    );
    let stop = tokio_util::sync::CancellationToken::new();
    stop.cancel();
    portfolio.task.shutdown().await.unwrap();
    _valuation.task.shutdown().await.unwrap();
    _invocations.shutdown().await;
    data.task.run(stop).await.unwrap();
}
#[tokio::test]
async fn application_handler_owns_menu_alias_routing_launch_and_usage_without_mtproto() {
    let handler = commands::LaunchCommands;
    assert_eq!(
        handler
            .commands()
            .iter()
            .map(|command| command.name)
            .collect::<Vec<_>>(),
        ["start", "market", "help", "status", "floor", "portfolio"]
    );
    assert!(
        handler
            .commands()
            .iter()
            .all(|command| !command.description.is_empty())
    );
    assert_eq!(handler.launch_label(), "Open market");
    let names = vec!["market_bot".to_owned(), "second_bot".to_owned()];
    for text in ["/start payload", "/market@SECOND_BOT", "/help"] {
        let request = handler.parse(message(1, text), &names).unwrap();
        assert!(!handler.handle(request).await.unwrap().text.is_empty());
    }
    for text in ["/start@inactive_bot", "hello"] {
        assert!(handler.parse(message(1, text), &names).is_none());
    }
    let mut group = message(1, "/help");
    group.private = false;
    assert!(handler.parse(group, &names).is_none());
    let mut outgoing = message(1, "/help");
    outgoing.outgoing = true;
    assert!(handler.parse(outgoing, &names).is_none());
    let request = handler.parse(message(1, "/floor bad"), &names).unwrap();
    assert!(
        handler
            .handle(request)
            .await
            .unwrap()
            .text
            .starts_with("Use /floor")
    );
}
#[tokio::test]
async fn bot_service_cancellation_opens_no_telegram_storage_and_keeps_market_data_available() {
    let directory = tempfile::tempdir().unwrap();
    let writer =
        market_replica::ReplicaWriter::open(&directory.path().join("replica"), "test").unwrap();
    let data = market_actors::MarketDataService::start(writer.reader(), 1, 1)
        .await
        .unwrap();
    let worker = market_telegram::TelegramWorker::new(market_telegram::BotConfig {
        credentials: market_telegram::BotCredentials::new(
            "42:private_test_secret".into(),
            12345,
            "0123456789abcdef0123456789abcdef".into(),
        )
        .unwrap(),
        public_url: None,
        data_dir: directory.path().join("telegram"),
    })
    .unwrap();
    let stop = tokio_util::sync::CancellationToken::new();
    stop.cancel();
    let (_invocations, _valuation, portfolio) =
        services(data.handle.clone(), InventoryError::Forbidden).await;
    BotService::new(
        worker,
        data.handle.clone(),
        portfolio.handle.clone(),
        _valuation.handle.clone(),
    )
    .run(stop.clone())
    .await
    .unwrap();
    assert!(!directory.path().join("telegram").exists());
    assert!(data.handle.status().await.is_ok());
    portfolio.task.shutdown().await.unwrap();
    _valuation.task.shutdown().await.unwrap();
    _invocations.shutdown().await;
    data.task.run(stop).await.unwrap();
}
#[tokio::test]
async fn portfolio_authorization_failure_remains_fatal_for_the_transport() {
    let directory = tempfile::tempdir().unwrap();
    let writer = market_replica::ReplicaWriter::open(directory.path(), "test").unwrap();
    let data = market_actors::MarketDataService::start(writer.reader(), 1, 1)
        .await
        .unwrap();
    let (_invocations, _valuation, portfolio) =
        services(data.handle.clone(), InventoryError::Authorization).await;
    let handler = MarketCommands::new(
        data.handle.clone(),
        portfolio.handle.clone(),
        _valuation.handle.clone(),
    );
    let request = handler
        .parse(message(1, "/portfolio"), &["market_bot".into()])
        .unwrap();
    assert_eq!(handler.handle(request).await, Err(BotError::Authorization));
    let stop = tokio_util::sync::CancellationToken::new();
    stop.cancel();
    portfolio.task.shutdown().await.unwrap();
    _valuation.task.shutdown().await.unwrap();
    _invocations.shutdown().await;
    data.task.run(stop).await.unwrap();
}

#[tokio::test]
async fn portfolio_commands_share_pagination_cache_and_revision_consistent_marks() {
    use market_gifts::{SyncState, SyncStatus, inventory::*, valuation::*};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    #[derive(Clone, Default)]
    struct Pages(Arc<AtomicUsize>);
    impl InventoryProvider for Pages {
        fn page(
            &self,
            request: InventoryPageRequest,
        ) -> impl market_invocations::Invocation<Output = InventoryPage, Error = InventoryError>
        {
            let calls = self.0.clone();
            market_invocations::operation(
                market_invocations::Policy {
                    limits: vec![],
                    replay: market_invocations::Replay::Safe,
                    attempt_timeout: std::time::Duration::from_secs(1),
                    max_attempts: 1,
                },
                move || {
                    let calls = calls.clone();
                    let request = request.clone();
                    async move {
                        assert_eq!(request.user.get(), 7);
                        calls.fetch_add(1, Ordering::Relaxed);
                        let start = request
                            .cursor
                            .as_deref()
                            .map(|s| s.strip_prefix("offset:").unwrap().parse::<usize>().unwrap())
                            .unwrap_or(0);
                        let end = (start + request.limit).min(745);
                        let gifts = (start..end)
                            .filter(|id| *id >= 45)
                            .map(|id| InventoryGift {
                                id: format!("gift-{id}").try_into().unwrap(),
                                collection_id: market_gifts::TelegramGiftId::new(1).unwrap(),
                                model_document_id: market_gifts::TelegramDocumentId::new(2)
                                    .unwrap(),
                                relation: GiftRelation::Owned,
                            })
                            .collect::<Vec<_>>();
                        Ok(InventoryPage {
                            regular_gifts: end - start - gifts.len(),
                            gifts,
                            scanned: end - start,
                            reported_total: Some(745),
                            next_cursor: (end < 745).then(|| format!("offset:{end}")),
                            ..Default::default()
                        })
                    }
                },
            )
        }
    }
    struct Floors {
        changes: tokio::sync::watch::Receiver<u64>,
        calls: Arc<AtomicUsize>,
    }
    impl market_valuation::Strategy for Floors {
        type Key = ValuationRequest;
        type Value = Valuation;
        type Error = market_actors::error::DataError;
        fn changes(&self) -> tokio::sync::watch::Receiver<u64> {
            self.changes.clone()
        }
        async fn evaluate(
            &self,
            request: ValuationRequest,
        ) -> Result<market_valuation::Evaluation<Valuation>, Self::Error> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            let revision = *self.changes.borrow();
            let price = revision + 2;
            let count: u32 = request.positions.iter().map(|p| p.quantity).sum();
            let value = Valuation {
                rows: request
                    .positions
                    .into_iter()
                    .map(|p| ModelValue {
                        identity: None,
                        collection: "Gift".into(),
                        model: "Model".into(),
                        quantity: p.quantity,
                        floor: Some(price.to_string().try_into().unwrap()),
                        value: Some(
                            (u64::from(p.quantity) * price)
                                .to_string()
                                .try_into()
                                .unwrap(),
                        ),
                        venue_id: None,
                        unpriced: None,
                    })
                    .collect(),
                total: (u64::from(count) * price).to_string().try_into().unwrap(),
                priced: count,
                unpriced: 0,
                currency_id: request.currency_id,
                cursor: revision.to_string(),
                sync: SyncStatus {
                    ready: true,
                    state: SyncState::Live,
                    updated_at: None,
                },
            };
            Ok(market_valuation::Evaluation {
                revision: value.cursor.clone(),
                coverage: market_valuation::Coverage {
                    priced: count,
                    unpriced: 0,
                },
                value,
            })
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let writer = market_replica::ReplicaWriter::open(directory.path(), "test").unwrap();
    let data = market_actors::MarketDataService::start(writer.reader(), 1, 1)
        .await
        .unwrap();
    let invocations = market_invocations::Runtime::start(Default::default()).unwrap();
    let (changed, changes) = tokio::sync::watch::channel(0);
    let quotes = Arc::new(AtomicUsize::new(0));
    let valuation = market_valuation::Service::start(
        Floors {
            changes,
            calls: quotes.clone(),
        },
        Default::default(),
    )
    .unwrap();
    let pages = Pages::default();
    let portfolio = market_portfolio::PortfolioService::start(
        pages.clone(),
        valuation.handle.clone(),
        Default::default(),
        invocations.handle(),
    )
    .await
    .unwrap();
    let handler = MarketCommands::new(
        data.handle.clone(),
        portfolio.handle.clone(),
        valuation.handle.clone(),
    );
    let request = handler
        .parse(message(1, "/portfolio"), &["market_bot".into()])
        .unwrap();
    let (first, duplicate) = tokio::join!(
        handler.handle(request.clone()),
        handler.handle(request.clone())
    );
    for reply in [first.unwrap(), duplicate.unwrap()] {
        assert!(reply.text.contains("745 of 745 across 8 page(s)"));
        assert!(reply.text.contains("Model-floor estimate: 1400 GRAM"));
        assert!(reply.text.contains("Priced collectibles: 700"));
        assert!(
            reply
                .text
                .contains("Regular, non-collectible gifts skipped: 45")
        );
    }
    assert_eq!(pages.0.load(Ordering::Relaxed), 8);
    assert_eq!(quotes.load(Ordering::Relaxed), 1);
    changed.send_replace(1);
    let updated = handler.handle(request).await.unwrap();
    assert!(updated.text.contains("Model-floor estimate: 2100 GRAM"));
    assert_eq!(pages.0.load(Ordering::Relaxed), 8);
    portfolio.task.shutdown().await.unwrap();
    valuation.task.shutdown().await.unwrap();
    invocations.shutdown().await;
    let stop = tokio_util::sync::CancellationToken::new();
    stop.cancel();
    data.task.run(stop).await.unwrap();
}
