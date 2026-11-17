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

use market_telegram::{BotError, GiftSource, InventoryError, MessageHandler, VisibleInventory};
struct ForbiddenGifts;
impl GiftSource for ForbiddenGifts {
    async fn inventory(&self) -> Result<VisibleInventory, InventoryError> {
        Err(InventoryError::Forbidden)
    }
}
#[tokio::test]
async fn bot_status_reads_the_shared_actor_and_portfolio_permission_failure_is_a_user_reply() {
    let directory = tempfile::tempdir().unwrap();
    let writer = market_replica::ReplicaWriter::open(directory.path(), "test").unwrap();
    let data = market_actors::MarketDataService::start(writer.reader(), 1, 1)
        .await
        .unwrap();
    let handler = MarketCommands::new(data.handle.clone());
    let names = vec!["market_bot".to_owned(), "market_alias".to_owned()];
    let request = handler
        .parse(message(1, "/portfolio@MARKET_ALIAS"), &names)
        .unwrap();
    let reply = handler.handle(request, &ForbiddenGifts).await.unwrap();
    assert!(reply.text.contains("Telegram did not allow"));
    assert!(!reply.text.contains("estimate: 0"));
    let request = handler.parse(message(2, "/status"), &names).unwrap();
    assert_eq!(
        handler.handle(request, &ForbiddenGifts).await.unwrap().text,
        "Market data: starting. Preparing market data. Please check again shortly."
    );
    let stop = tokio_util::sync::CancellationToken::new();
    stop.cancel();
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
        assert!(
            !handler
                .handle(request, &ForbiddenGifts)
                .await
                .unwrap()
                .text
                .is_empty()
        );
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
            .handle(request, &ForbiddenGifts)
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
    BotService::new(worker, data.handle.clone())
        .run(stop.clone())
        .await
        .unwrap();
    assert!(!directory.path().join("telegram").exists());
    assert!(data.handle.status().await.is_ok());
    data.task.run(stop).await.unwrap();
}
#[tokio::test]
async fn portfolio_authorization_failure_remains_fatal_for_the_transport() {
    struct Revoked;
    impl GiftSource for Revoked {
        async fn inventory(&self) -> Result<VisibleInventory, InventoryError> {
            Err(InventoryError::Authorization)
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let writer = market_replica::ReplicaWriter::open(directory.path(), "test").unwrap();
    let data = market_actors::MarketDataService::start(writer.reader(), 1, 1)
        .await
        .unwrap();
    let handler = MarketCommands::new(data.handle.clone());
    let request = handler
        .parse(message(1, "/portfolio"), &["market_bot".into()])
        .unwrap();
    assert_eq!(
        handler.handle(request, &Revoked).await,
        Err(BotError::Authorization)
    );
    let stop = tokio_util::sync::CancellationToken::new();
    stop.cancel();
    data.task.run(stop).await.unwrap();
}
