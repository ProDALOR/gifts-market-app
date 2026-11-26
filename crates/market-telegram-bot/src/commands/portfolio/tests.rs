use super::render::{checked_time, display_amount, sort_controls};
use super::request::{SORTS, parse_sort};
use super::*;
use crate::commands::rich::COLUMNS;
fn render(state: &PortfolioState, request: &PortfolioRequest, icons: &crate::ReportIcons) -> Reply {
    super::render(
        state,
        request,
        icons,
        &PortfolioReadCache::default(),
        &tokio_util::sync::CancellationToken::new(),
    )
}
fn sample(count: usize) -> PortfolioState {
    use market_gifts::{TelegramDocumentId, TelegramGiftDetails, TelegramGiftId};
    let items = (1..=count)
        .map(|n| market_valuation::InventoryValue {
            gift: market_portfolio::InventoryGift {
                gift: TelegramGiftDetails {
                    id: n.to_string().try_into().unwrap(),
                    slug: format!("ResistanceDog-{n}").try_into().unwrap(),
                    number: (n as u32).try_into().unwrap(),
                    collection: TelegramGiftId::new(123).unwrap(),
                    model: TelegramDocumentId::new(n as i64 + 1000).unwrap(),
                    backdrop: None,
                    pattern: None,
                },
                relation: market_portfolio::GiftRelation::Owned,
            },
            resolved: None,
            estimate: Some("2".into()),
            error: None,
        })
        .collect();
    PortfolioState {
        user_id: UserId::new(123).unwrap(),
        refresh_id: 1,
        progress: market_portfolio::PortfolioProgress {
            stage: PortfolioStage::Ready,
            ..Default::default()
        },
        inventory: None,
        version: Some(format!("{count:032x}").try_into().unwrap()),
        refresh_after: None,
        error: None,
        valuation: Some(std::sync::Arc::new(market_valuation::InventoryValuation {
            currency_id: "GRAM".into(),
            evaluated_at: jiff::Timestamp::now(),
            stale: false,
            total: Some((count * 2).to_string().try_into().unwrap()),
            priced: count,
            unpriced: 0,
            items,
        })),
    }
}
#[test]
fn grids_fill_three_columns_and_clamp_pagination_without_losing_gifts() {
    let icons = crate::ReportIcons::default();
    for count in [0, 1, 29, 30, 31, 61] {
        let state = sample(count);
        for requested in [0, usize::MAX] {
            let request = PortfolioRequest {
                user_id: UserId::new(123).unwrap(),
                currency_id: BotCurrency::Gram,
                action: PortfolioAction::Open,
                page: requested,
                sort: PortfolioSort::PositionAsc,
                mode: PortfolioMode::Private,
                layout: GridMode::Linked,
            };
            let reply = render(&state, &request, &icons).bounded();
            let html = reply.rich_html.unwrap();
            let page = requested.min(count.div_ceil(PAGE_SIZE).max(1) - 1);
            let size = (count - page * PAGE_SIZE).min(PAGE_SIZE);
            assert_eq!(html.matches("https://t.me/nft/").count(), size);
            assert_eq!(html.matches("<tr>").count(), size.div_ceil(COLUMNS));
            assert_eq!(
                html.matches("<td").count(),
                size.div_ceil(COLUMNS) * COLUMNS
            );
            assert_eq!(reply.emoji_documents.len(), size);
            if size > 0 {
                assert!(html.contains(&format!(
                    "https://t.me/nft/ResistanceDog-{}",
                    page * PAGE_SIZE + 1
                )));
                assert!(html.contains("\"><tg-emoji emoji-id=\""));
                assert!(html.contains("</tg-emoji> <b>2</b> <tg-emoji"));
                assert_eq!(html.matches("</a></td>").count(), size);
                assert!(!html.contains("ResistanceD…"));
                assert!(!html.contains("<br>"));
                assert!(!html.contains("<p></p>"));
            } else {
                assert!(!html.contains("<table"));
            }
            assert!(html.len() < 32768);
        }
    }
}
#[test]
fn compact_reports_keep_owner_access_currency_and_one_count() {
    let icons = crate::ReportIcons::default();
    let reply = render(&sample(31), &callback("p:123:GRAM:v:0").unwrap(), &icons);
    let html = reply.rich_html.unwrap();
    let header = html.split("<tg-button-row>").next().unwrap();
    assert!(header.contains("tg://user?id=123\">Portfolio</a>"));
    assert!(!header.contains("Portfolio · 123"));
    assert!(!header.contains("GRAM"));
    assert!(!header.contains("31 gifts"));
    assert!(!html.contains("0 unpriced"));
    assert_eq!(html.matches("1–30 of 31").count(), 1);
    assert!(reply.text.starts_with("Portfolio\nEstimate 62 GRAM"));

    // Currency comes from the report; preserve unknown units when no icon is available.
    let mut state = sample(0);
    std::sync::Arc::get_mut(state.valuation.as_mut().unwrap())
        .unwrap()
        .currency_id = "USD".into();
    let unknown = render(&state, &callback("p:123:GRAM:v:0").unwrap(), &icons);
    assert!(unknown.rich_html.unwrap().contains("<h2>0 USD</h2>"));
    assert!(unknown.text.contains("0 gifts"));
}

#[test]
fn gallery_preserves_sort_currency_and_sharing_with_twelve_gifts_per_page() {
    let request = request::parse_args(
        "123 stars sort=price_desc view=gallery",
        456,
        PortfolioMode::Inline,
    )
    .unwrap();
    assert_eq!(request.layout, GridMode::Gallery);
    assert_eq!(request.page_size(), 12);
    let token = request.data(1, PortfolioAction::Navigate, request.sort);
    assert!(token.len() <= 64);
    let next = callback(&token).unwrap();
    assert_eq!(next.layout, GridMode::Gallery);
    assert_eq!(next.page, 1);
    assert_eq!(next.currency_id, BotCurrency::Stars);
    assert_eq!(next.sort, request.sort);
    assert_eq!(
        request::inline(&request.inline_query(), 456).unwrap(),
        request
    );
    assert_eq!(callback("p:123:GRAM:v:0").unwrap().layout, GridMode::Linked);
    assert!(callback("p:123:GRAM:v:0:pa:unknown").is_none());
    assert!(request::parse_args("view=gallery view=linked", 123, PortfolioMode::Private).is_none());

    let mut page = next;
    page.currency_id = BotCurrency::Gram;
    page.sort = PortfolioSort::PositionAsc;
    let reply = render(&sample(31), &page, &crate::ReportIcons::default());
    let html = reply.rich_html.unwrap();
    assert_eq!(html.matches("https://t.me/nft/").count(), 12);
    assert!(html.contains("ResistanceDog-13"));
    assert!(html.contains("ResistanceDog-24"));
    assert!(!html.contains("ResistanceDog-25"));
    assert!(html.contains("13–24 of 31"));
}
#[test]
fn warning_details_survive_compaction_and_errors_are_not_duplicated() {
    for stage in [PortfolioStage::Ready, PortfolioStage::Failed] {
        let mut state = sample(1);
        state.progress.stage = stage;
        state.error = Some("Refresh <failed>".into());
        let report = std::sync::Arc::get_mut(state.valuation.as_mut().unwrap()).unwrap();
        report.total = None;
        report.items[0].estimate = None;
        report.priced = 0;
        report.unpriced = 1;
        report.stale = true;
        state.inventory = Some(market_portfolio::InventorySummary {
            checked_at: "2025-11-24T03:06:00Z".parse().unwrap(),
            count: 1,
            scanned: 1,
            regular: 0,
            skipped: 0,
            complete: false,
        });
        let reply = render(
            &state,
            &callback("p:123:GRAM:v:0").unwrap(),
            &crate::ReportIcons::default(),
        );
        let html = reply.rich_html.unwrap();
        for warning in ["1 unpriced", "Partial inventory", "Market updates delayed"] {
            assert!(html.contains(warning));
            assert!(reply.text.contains(warning));
        }
        assert_eq!(html.matches("Refresh &lt;failed&gt;").count(), 1);
        assert_eq!(reply.text.matches("Refresh <failed>").count(), 1);
        assert!(reply.text.contains("Estimate — GRAM"));
        assert_eq!(html.matches("<footer>").count(), 1);
        assert!(html.contains("1–1 of 1 · Checked "));
        assert!(!html.contains("Gifts checked"));
    }
}
#[test]
fn amounts_and_check_times_keep_rounding_and_date_boundaries() {
    for (input, expected) in [
        ("0", "0"),
        ("0.5", "0.5"),
        ("999.995", "1 000"),
        ("1234.50", "1 234.5"),
        ("1234567.126", "1 234 567.13"),
    ] {
        assert_eq!(
            display_amount(&market_gifts::Amount::new(input).unwrap()),
            expected
        );
    }
    let now = "2026-11-25T03:07:00Z".parse().unwrap();
    for (input, expected) in [
        ("2026-11-25T03:06:00Z", "03:06 UTC"),
        ("2026-11-24T23:59:59Z", "24 Nov 23:59 UTC"),
        ("2025-11-25T03:06:00Z", "25 Nov 2025 03:06 UTC"),
    ] {
        assert_eq!(checked_time(input.parse().unwrap(), now), expected);
    }
}
#[tokio::test]
#[ignore = "sends one real portfolio preview; requires explicit bot ID, recipient and captured portfolio"]
async fn send_live_portfolio_grid_preview() {
    use grammers_session::types::{PeerAuth, PeerId, PeerRef};
    use market_telegram_client::{
        BotCredentials, Connection,
        transport::{Message, TelegramActions, Transport},
    };
    let credentials = BotCredentials::new(
        std::env::var("TELEGRAM_BOT_TOKEN").unwrap(),
        std::env::var("TELEGRAM_API_ID").unwrap().parse().unwrap(),
        std::env::var("TELEGRAM_API_HASH").unwrap(),
    )
    .unwrap();
    let expected = std::env::var("MARKET_TEST_BOT_ID")
        .unwrap()
        .parse::<u64>()
        .unwrap();
    let recipient = std::env::var("MARKET_TEST_RECIPIENT")
        .unwrap()
        .parse::<u64>()
        .unwrap();
    assert_eq!(credentials.bot_id(), expected);
    let captured: serde_json::Value = serde_json::from_slice(
        &std::fs::read(std::env::var("MARKET_TEST_PORTFOLIO").unwrap()).unwrap(),
    )
    .unwrap();
    let mut raw = captured["state"].clone();
    raw["valuation"]["items"] = captured["page"]["items"].clone();
    raw["version"] = captured["page"]["version"].clone();
    let state: PortfolioState = serde_json::from_value(raw).unwrap();
    assert_eq!(state.user_id, UserId::new(recipient).unwrap());
    let request = PortfolioRequest {
        user_id: UserId::new(recipient).unwrap(),
        currency_id: BotCurrency::Gram,
        action: PortfolioAction::Open,
        page: 0,
        sort: std::env::var("MARKET_TEST_PORTFOLIO_SORT")
            .ok()
            .map(|value| parse_sort(&value).unwrap())
            .unwrap_or(PortfolioSort::PositionAsc),
        mode: PortfolioMode::Private,
        layout: GridMode::Linked,
    };
    let reply = render(&state, &request, &crate::ReportIcons::default()).bounded();
    if let Ok(path) = std::env::var("MARKET_TEST_REPLY_OUTPUT") {
        if path.ends_with(".json") {
            std::fs::write(
                path,
                serde_json::to_vec(&serde_json::json!({
                    "text": reply.text, "title": reply.title, "summary": reply.summary,
                    "rich_html": reply.rich_html, "emoji_documents": reply.emoji_documents,
                }))
                .unwrap(),
            )
            .unwrap();
        } else {
            std::fs::write(path, reply.rich_html.as_ref().unwrap()).unwrap();
        }
    }
    let mut reply = super::super::tests::static_preview(reply);
    for row in &mut reply.buttons {
        row.retain(|button| !matches!(button, ReplyButton::Callback { .. }));
    }
    reply.buttons.retain(|row| !row.is_empty());
    let scheduler = market_invocations::Runtime::start(Default::default()).unwrap();
    let directory = std::path::PathBuf::from(
        std::env::var("MARKET_TEST_SESSION_DIR")
            .expect("reuse an authorized session directory for live previews"),
    );
    let connection = Connection::open(credentials, &directory, scheduler.handle())
        .await
        .unwrap();
    let actions = connection.transport.actions();
    let user = actions.identity().await.unwrap();
    assert_eq!(user.id, expected);
    let event = market_telegram_client::interactions::Event::Message(Message {
        peer: PeerRef {
            id: PeerId::user(recipient as i64).unwrap(),
            auth: PeerAuth::from_hash(0),
        },
        id: super::super::tests::preview_message_id(),
        outgoing: false,
        text: String::new(),
    });
    let result = actions.send_progress(&event, &reply, None).await;
    connection.close().await.unwrap();
    scheduler.shutdown().await;
    let Some(market_telegram_client::interactions::Event::Callback {
        target: market_telegram_client::interactions::CallbackTarget::Message(sent),
        ..
    }) = result.unwrap()
    else {
        panic!("Telegram did not return the portfolio preview message identity");
    };
    eprintln!(
        "Sent 30-gift portfolio preview via @{} to authorized recipient {}; Telegram message_id={}",
        user.username, recipient, sent.id
    );
}
#[test]
fn callbacks_are_bounded_and_do_not_retain_any_transport_credentials() {
    let request = callback("p:123:GRAM:r:2").unwrap();
    assert_eq!(request.user_id.get(), 123);
    assert_eq!(request.action, PortfolioAction::Refresh);
    assert_eq!(request.page, 2);
    for invalid in [
        "p:0:GRAM:v:0",
        "p:1:GRAM:r:1000",
        "p:1:GRAM:other:0",
        "p:1:GRAM:v:0:extra",
    ] {
        assert!(callback(invalid).is_none(), "{invalid}");
    }
}
#[test]
fn sorts_round_trip_through_pages_refresh_and_sharing() {
    for (sort, name, _) in SORTS {
        let mut request =
            parse_args(&format!("123 XTR sort={name}"), 7, PortfolioMode::Private).unwrap();
        assert_eq!(request.sort, sort);
        for refresh in [false, true] {
            let data = request.data(
                2,
                if refresh {
                    PortfolioAction::Refresh
                } else {
                    PortfolioAction::Navigate
                },
                sort,
            );
            assert!(data.len() <= 64);
            let parsed = callback(&data).unwrap();
            assert_eq!(
                (
                    parsed.user_id.get(),
                    parsed.currency_id.as_str(),
                    parsed.sort,
                    parsed.page,
                    parsed.action == PortfolioAction::Refresh
                ),
                (123, "XTR", sort, 2, refresh)
            );
        }
        request.mode = PortfolioMode::Inline;
        let parsed = inline(&request.inline_query(), 99).unwrap();
        assert_eq!(
            (
                parsed.user_id.get(),
                parsed.sort,
                parsed.currency_id.as_str(),
                parsed.mode
            ),
            (123, sort, "XTR", PortfolioMode::Inline)
        );
        for button in sort_controls(&request).split("data=\"").skip(1) {
            assert_eq!(callback(button.split('"').next().unwrap()).unwrap().page, 0);
        }
    }
    assert_eq!(
        callback("p:123:GRAM:v:2").unwrap().sort,
        PortfolioSort::PositionAsc
    );
    for invalid in [
        "p:1:GRAM:v:0:bad",
        "p:1:GRAM:v:0:vd:extra",
        "p:1:GRAM:r:1000:pa",
    ] {
        assert!(callback(invalid).is_none());
    }
    for invalid in [
        "sort=bad",
        "sort=price_asc sort=price_desc",
        "1 2",
        "-1",
        "0",
        "1 GRAM XTR",
    ] {
        assert!(
            parse_args(invalid, 7, PortfolioMode::Private).is_none(),
            "{invalid}"
        );
    }
}
#[test]
fn shared_sorting_keeps_unpriced_last_and_every_price_inside_its_gift_link() {
    let mut state = sample(4);
    let report = std::sync::Arc::get_mut(state.valuation.as_mut().unwrap()).unwrap();
    let original = std::mem::take(&mut report.items);
    report.items = [2, 0, 3, 1]
        .into_iter()
        .map(|i| original[i].clone())
        .collect();
    for item in &mut report.items {
        item.estimate = match item.gift.gift.number.get() {
            1 => Some("10".into()),
            3 => None,
            _ => Some("30".into()),
        };
    }
    report.priced = 3;
    report.unpriced = 1;
    report.total = Some("70".into());
    let cache = PortfolioReadCache::default();
    for (sort, expected) in [
        (PortfolioSort::PositionAsc, [3, 1, 4, 2]),
        (PortfolioSort::PositionDesc, [2, 4, 1, 3]),
        (PortfolioSort::NumberAsc, [1, 2, 3, 4]),
        (PortfolioSort::NumberDesc, [4, 3, 2, 1]),
        (PortfolioSort::ValueAsc, [1, 2, 4, 3]),
        (PortfolioSort::ValueDesc, [2, 4, 1, 3]),
    ] {
        let mut request = callback("p:123:GRAM:v:0").unwrap();
        request.sort = sort;
        let reply = super::render(
            &state,
            &request,
            &crate::ReportIcons::default(),
            &cache,
            &tokio_util::sync::CancellationToken::new(),
        );
        let html = reply.rich_html.unwrap();
        let actual = html
            .split("https://t.me/nft/ResistanceDog-")
            .skip(1)
            .map(|cell| cell.split('"').next().unwrap().parse::<u32>().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(actual, expected, "{sort:?}");
        for cell in html.split("<td><a href=").skip(1) {
            let linked = cell.split("</a>").next().unwrap();
            assert!(linked.contains("<b>"));
            assert!(linked.contains(&format!(
                "emoji-id=\"{}\"",
                crate::ReportIcon::Gram.definition().document.get()
            )));
        }
    }
}
#[test]
fn cancelled_sorted_reads_do_not_build_a_reply_and_inline_has_no_refresh_button() {
    let state = sample(31);
    let cache = PortfolioReadCache::default();
    let stop = tokio_util::sync::CancellationToken::new();
    stop.cancel();
    let mut request = callback("p:123:GRAM:v:0:vd").unwrap();
    let cancelled = super::render(
        &state,
        &request,
        &crate::ReportIcons::default(),
        &cache,
        &stop,
    );
    assert!(cancelled.rich_html.is_none());
    request.mode = PortfolioMode::Inline;
    let reply = render(&state, &request, &crate::ReportIcons::default());
    assert!(
        reply
            .buttons
            .iter()
            .flatten()
            .all(|b| !matches!(b, ReplyButton::Callback { .. }))
    );
    assert!(reply.buttons.iter().flatten().any(|b| matches!(b, ReplyButton::SwitchInline { query, .. } if query.contains("sort=price_desc"))));
    assert_eq!(reply.title.as_deref(), Some("Portfolio"));
}
#[test]
fn visible_unverified_and_hosted_gifts_remain_in_reports_and_totals() {
    use market_gifts::{TelegramDocumentId, TelegramGiftDetails, TelegramGiftId};
    use market_portfolio::{GiftRelation, InventoryGift};
    let items = [
        GiftRelation::Owned,
        GiftRelation::Unverified,
        GiftRelation::Hosted,
    ]
    .into_iter()
    .enumerate()
    .map(|(index, relation)| {
        let number = index as u32 + 1;
        market_valuation::InventoryValue {
            gift: InventoryGift {
                gift: TelegramGiftDetails {
                    id: number.to_string().try_into().unwrap(),
                    slug: format!("Gift-{number}").try_into().unwrap(),
                    number: number.try_into().unwrap(),
                    collection: TelegramGiftId::new(123).unwrap(),
                    model: TelegramDocumentId::new(456).unwrap(),
                    backdrop: None,
                    pattern: None,
                },
                relation,
            },
            resolved: None,
            estimate: Some("2".into()),
            error: None,
        }
    })
    .collect();
    let state = PortfolioState {
        version: Some(
            "00000000000000000000000000000001"
                .to_owned()
                .try_into()
                .unwrap(),
        ),
        user_id: UserId::new(123).unwrap(),
        refresh_id: 1,
        progress: market_portfolio::PortfolioProgress {
            stage: PortfolioStage::Ready,
            ..Default::default()
        },
        inventory: None,
        refresh_after: None,
        error: None,
        valuation: Some(std::sync::Arc::new(market_valuation::InventoryValuation {
            currency_id: "GRAM".into(),
            evaluated_at: jiff::Timestamp::now(),
            stale: false,
            total: Some("6".into()),
            priced: 3,
            unpriced: 0,
            items,
        })),
    };
    let reply = render(
        &state,
        &callback("p:123:GRAM:v:0").unwrap(),
        &crate::ReportIcons::default(),
    );
    assert!(reply.text.contains("Estimate 6 GRAM"));
    for number in 1..=3 {
        assert!(reply.text.contains(&format!("Gift-{number} · 2")));
    }
    assert!(!reply.text.contains("On-chain"));
    assert!(!reply.text.contains("Hosted"));
    let html = reply.rich_html.unwrap();
    assert_eq!(html.matches("https://t.me/nft/").count(), 3);
    assert!(!html.contains("On-chain"));
    assert!(!html.contains("Hosted"));
}
#[test]
fn reports_expose_partial_inventory_and_plain_text_fallback() {
    let request = callback("p:123:GRAM:v:0").unwrap();
    let state = PortfolioState {
        version: Some(
            "00000000000000000000000000000001"
                .to_owned()
                .try_into()
                .unwrap(),
        ),
        user_id: UserId::new(123).unwrap(),
        refresh_id: 1,
        progress: market_portfolio::PortfolioProgress {
            stage: PortfolioStage::Ready,
            ..Default::default()
        },
        inventory: Some(market_portfolio::InventorySummary {
            checked_at: jiff::Timestamp::now(),
            count: 0,
            scanned: 0,
            regular: 0,
            skipped: 0,
            complete: false,
        }),
        valuation: Some(std::sync::Arc::new(market_valuation::InventoryValuation {
            currency_id: "GRAM".into(),
            evaluated_at: jiff::Timestamp::now(),
            stale: false,
            total: None,
            priced: 0,
            unpriced: 1,
            items: vec![],
        })),
        refresh_after: None,
        error: None,
    };
    let reply = render(&state, &request, &crate::ReportIcons::default());
    assert!(reply.text.contains("Estimate — GRAM"));
    assert!(
        reply
            .rich_html
            .as_ref()
            .unwrap()
            .contains("Partial inventory")
    );
    assert!(
        reply
            .buttons
            .iter()
            .flatten()
            .any(|b| matches!(b,ReplyButton::Preview{payload,..} if payload.starts_with("p_")))
    );
}

#[test]
fn recovery_preserves_callbacks_and_identifies_the_requested_subject_and_currency() {
    let request = inline("portfolio 456 stars sort=price_desc", 7).unwrap();
    let reply = recovery(&request, None);
    assert_eq!(reply.kind, market_telegram::ReplyKind::Message);
    assert!(reply.text.contains("/portfolio 456 XTR"));
    assert!(reply.summary.unwrap().contains("/portfolio 456 XTR"));
    for mode in [PortfolioMode::Private, PortfolioMode::Inline] {
        let request = PortfolioRequest {
            mode,
            action: PortfolioAction::Navigate,
            ..request.clone()
        };
        let reply = recovery(&request, None);
        assert_eq!(reply.kind, market_telegram::ReplyKind::Notice);
        assert!(reply.text.contains("/portfolio 456 XTR"));
        assert!(reply.rich_html.is_none());
        assert!(reply.buttons.is_empty());
        let busy = recovery(&request, Some("Portfolio capacity reached".into()));
        assert_eq!(busy.kind, market_telegram::ReplyKind::Notice);
        assert_eq!(busy.text, "Portfolio capacity reached");
    }
    let failure = recovery(&request, Some("Portfolio capacity reached".into()));
    assert_eq!(failure.title.as_deref(), Some("Portfolio unavailable"));
    assert!(!failure.summary.unwrap().contains("Check privately"));
}

#[test]
fn legacy_callbacks_and_currency_aliases_normalize_to_one_bounded_codec() {
    for word in ["GRAM", "gram", "Ton", "XTR", "xtr", "Stars"] {
        let request = parse_args(
            &format!("9007199254740991 {word} sort=number_desc"),
            7,
            PortfolioMode::Private,
        )
        .unwrap();
        for refresh in [false, true] {
            let data = request.data(
                999,
                if refresh {
                    PortfolioAction::Refresh
                } else {
                    PortfolioAction::Navigate
                },
                request.sort,
            );
            assert!(data.len() <= 64);
            let decoded = callback(&data).unwrap();
            assert_eq!(decoded.user_id, request.user_id);
            assert_eq!(decoded.currency_id, request.currency_id);
            assert_eq!(decoded.page, 999);
            assert_eq!(decoded.sort, request.sort);
        }
    }
    for old in ["p:123:GRAM:v:0", "p:123:TON:v:0", "p:123:XTR:v:0"] {
        let request = callback(old).unwrap();
        assert_eq!(request.action, PortfolioAction::Navigate);
        assert_eq!(request.sort, PortfolioSort::PositionAsc);
    }
    for word in ["USD", "GRAM:bad", "sort:price_desc", &"x".repeat(256)] {
        assert!(parse_args(word, 7, PortfolioMode::Private).is_none());
        assert!(callback(&format!("p:123:{word}:v:0:pa")).is_none());
    }
}

#[tokio::test]
async fn private_and_inline_navigation_use_cached_reports_during_a_running_refresh() {
    use market_portfolio::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio_util::sync::CancellationToken;
    #[derive(Clone, Default)]
    struct Source(Arc<AtomicUsize>);
    impl InventoryProvider for Source {
        fn page(
            &self,
            _: InventoryPageRequest,
        ) -> impl market_invocations::Invocation<Output = InventoryPage, Error = InventoryError>
        {
            let count = self.0.clone();
            market_invocations::operation(
                market_invocations::Policy {
                    limits: vec![market_invocations::RateLimit {
                        key: market_invocations::RateKey {
                            owner: 1,
                            group: "test",
                            resource: None,
                        },
                        interval: Duration::from_millis(1),
                        concurrency: 1,
                    }],
                    replay: market_invocations::Replay::Safe,
                    attempt_timeout: Duration::from_secs(1),
                    max_attempts: 1,
                },
                move || {
                    count.fetch_add(1, Ordering::SeqCst);
                    async {
                        Ok(InventoryPage {
                            gifts: vec![
                                sample(1).valuation.as_ref().unwrap().items[0].gift.clone(),
                            ],
                            scanned: 1,
                            reported_total: Some(1),
                            ..Default::default()
                        })
                    }
                },
            )
        }
    }
    #[derive(Clone, Default)]
    struct Quotes {
        calls: Arc<AtomicUsize>,
        release: CancellationToken,
    }
    impl ValuationProvider for Quotes {
        async fn value(
            &self,
            _: Arc<Vec<InventoryGift>>,
            _: market_gifts::CurrencyId,
            _: tokio::sync::watch::Sender<usize>,
        ) -> Result<Arc<InventoryValuation>, market_valuation::ValuationError> {
            if self.calls.fetch_add(1, Ordering::SeqCst) > 0 {
                self.release.cancelled().await;
            }
            Ok(sample(1).valuation.unwrap())
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let writer = market_replica::ReplicaWriter::open(dir.path(), "test").unwrap();
    let reads =
        market_replica::ReadRuntime::new(writer.reader(), 4, CancellationToken::new()).unwrap();
    let data = market_actors::MarketDataService::start(reads.handle(), 4)
        .await
        .unwrap();
    let (valuation, task) =
        market_valuation::ValuationHandle::start(&data.arbiter(), reads.handle())
            .await
            .unwrap();
    let scheduler = market_invocations::Runtime::start(Default::default()).unwrap();
    let source = Source::default();
    let quotes = Quotes::default();
    let service = PortfolioService::start(
        source.clone(),
        quotes.clone(),
        PortfolioConfig {
            refresh_interval: Duration::from_millis(5),
            ..Default::default()
        },
        scheduler.handle(),
    )
    .await
    .unwrap();
    let commands =
        MarketCommands::new(data.handle.clone(), valuation).with_portfolio(service.handle.clone());
    let request = parse_args("123 GRAM", 7, PortfolioMode::Private).unwrap();
    let first = commands.portfolio_reply(request.clone(), None).await;
    assert!(first.text.contains("Estimate 2 GRAM"));
    tokio::time::sleep(Duration::from_millis(15)).await;
    service
        .handle
        .query(PortfolioQuery {
            user_id: request.user_id,
            currency_id: "GRAM".into(),
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while quotes.calls.load(Ordering::SeqCst) != 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    for mode in [PortfolioMode::Private, PortfolioMode::Inline] {
        let navigation = PortfolioRequest {
            action: PortfolioAction::Navigate,
            mode,
            ..request.clone()
        };
        let reply = tokio::time::timeout(
            Duration::from_secs(1),
            commands.portfolio_reply(navigation, None),
        )
        .await
        .expect("navigation must not wait for the running valuation");
        assert!(reply.text.contains("Estimate 2 GRAM"));
        assert!(reply.rich_html.unwrap().contains("ResistanceDog-1"));
    }
    let missing = inline("portfolio 456 XTR", 7).unwrap();
    let reply = commands.portfolio_reply(missing, None).await;
    assert!(reply.text.contains("/portfolio 456 XTR"));
    let cached = service
        .handle
        .cached(PortfolioQuery {
            user_id: request.user_id,
            currency_id: "GRAM".into(),
        })
        .await
        .unwrap();
    let sharing = crate::Sharing::new(Arc::new(commands.clone()), Default::default());
    let query = serde_json::from_value(serde_json::json!({
        "user_id": request.user_id, "currency_id":"GRAM", "filters":{},
        "sort":"position_asc", "offset":0, "limit":30, "version":cached.version,
    }))
    .unwrap();
    let shared = tokio::time::timeout(Duration::from_secs(1), sharing.portfolio(7, query))
        .await
        .expect("preparing a share must not join or start the running valuation")
        .unwrap();
    assert!(shared.prepared.is_none());
    let id = u64::from_str_radix(shared.inline_query.strip_prefix("s:").unwrap(), 16).unwrap();
    assert!(
        commands
            .shared_reply(7, id)
            .await
            .text
            .contains("Estimate 2 GRAM")
    );
    let stale = serde_json::from_value(serde_json::json!({
        "user_id": request.user_id, "currency_id":"GRAM", "filters":{},
        "version":"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
    }))
    .unwrap();
    assert!(matches!(
        sharing.portfolio(7, stale).await,
        Err(crate::ShareError::RestartRead)
    ));
    let missing =
        serde_json::from_value(serde_json::json!({ "user_id":456, "currency_id":"GRAM" })).unwrap();
    assert!(matches!(
        sharing.portfolio(7, missing).await,
        Err(crate::ShareError::Unavailable(_))
    ));
    let gallery_request = PortfolioRequest {
        layout: GridMode::Gallery,
        action: PortfolioAction::Preview,
        ..request.clone()
    };
    let gallery = tokio::time::timeout(
        Duration::from_secs(1),
        commands.handle(CommandRequest {
            command: Command::Portfolio(gallery_request.clone()),
            user_id: 7,
            message_id: 0,
            inline: false,
        }),
    )
    .await
    .expect("preview reads only cached valuation")
    .unwrap();
    assert!(matches!(
        gallery.artwork,
        Some(market_media::Artwork::Gallery(_))
    ));
    let legacy = PortfolioRequest {
        page: 1,
        action: PortfolioAction::Navigate,
        ..gallery_request
    };
    let browsed = commands.portfolio_reply(legacy, None).await;
    assert!(
        browsed.artwork.is_none(),
        "legacy gallery pagination returns to linked browsing without image generation"
    );
    assert_eq!(source.0.load(Ordering::SeqCst), 1);
    assert_eq!(quotes.calls.load(Ordering::SeqCst), 2);
    service.task.shutdown().await.unwrap();
    let navigation = PortfolioRequest {
        action: PortfolioAction::Navigate,
        ..request
    };
    assert_eq!(
        commands.portfolio_reply(navigation, None).await.kind,
        market_telegram::ReplyKind::Notice
    );
    scheduler.shutdown().await;
    task.shutdown().await.unwrap();
    let stop = CancellationToken::new();
    stop.cancel();
    data.task.run(stop).await.unwrap();
    reads.shutdown().await;
}

#[test]
fn shared_pages_keep_filters_sorting_offsets_and_warnings_without_unfiltered_navigation() {
    let state = sample(61);
    let reads = PortfolioReadCache::default();
    let stop = tokio_util::sync::CancellationToken::new();
    let query = ResolvedPortfolioReadQuery {
        user_id: state.user_id,
        currency_id: "GRAM".into(),
        filters: market_portfolio::ResolvedPortfolioFilters {
            number: Some(37u32.try_into().unwrap()),
            ..Default::default()
        },
        sort: PortfolioSort::PositionDesc,
        offset: 0,
        limit: 30,
        version: state.version.clone(),
    };
    let page = reads.read(&state, query.clone(), || Ok(())).unwrap();
    assert_eq!(page.selection.count, 1);
    let request = PortfolioRequest {
        user_id: state.user_id,
        currency_id: BotCurrency::Gram,
        action: PortfolioAction::Navigate,
        page: 0,
        sort: query.sort,
        mode: PortfolioMode::Inline,
        layout: GridMode::Linked,
    };
    let reply = super::render::render_page(
        &state,
        &request,
        &Default::default(),
        &reads,
        &stop,
        Some((page, 0)),
    );
    let html = reply.rich_html.unwrap();
    assert_eq!(html.matches("https://t.me/nft/").count(), 1);
    assert!(html.contains("ResistanceDog-37"));
    assert!(html.contains("<h2>2 "));
    assert!(html.contains("1–1 of 1"));
    assert!(
        !html.contains("p:123:"),
        "filter-dropping sort/navigation callbacks must not be offered"
    );
    let mut stale = query;
    stale.version = Some(
        "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
            .to_owned()
            .try_into()
            .unwrap(),
    );
    assert!(matches!(
        reads.read(&state, stale, || Ok(())),
        Err(market_portfolio::PortfolioError::RestartRead)
    ));
}

#[test]
fn gallery_snapshots_have_no_sort_or_paging_and_browse_returns_to_the_same_linked_page() {
    let state = sample(100);
    let request = PortfolioRequest {
        user_id: state.user_id,
        currency_id: BotCurrency::Gram,
        action: PortfolioAction::Preview,
        page: 3,
        sort: PortfolioSort::PositionAsc,
        mode: PortfolioMode::Private,
        layout: GridMode::Gallery,
    };
    let reply = render(&state, &request, &Default::default());
    let html = reply.rich_html.unwrap();
    assert!(html.contains(">Browse</tg-button>"));
    assert!(reply.text.contains("37–48 of 100"));
    assert_eq!(html.matches("https://t.me/nft/").count(), 12);
    for label in ["→", "←", "Position ↑", "Price ↓", "Gift # ↑"] {
        assert!(!html.contains(&format!(">{label}</tg-button>")));
    }
    assert!(
        !reply
            .buttons
            .iter()
            .flatten()
            .any(|b| matches!(b, ReplyButton::Callback { .. }))
    );
    let end = html.find(">Browse</tg-button>").unwrap();
    let button = &html[html[..end].rfind("<tg-button ").unwrap()..end];
    let data = button
        .split("data=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    let target = callback(data).unwrap();
    assert_eq!(target.layout, GridMode::Linked);
    assert_eq!(target.page, 1);
    let linked = render(&state, &target, &Default::default());
    let html = linked.rich_html.unwrap();
    let end = html.find(">Gallery</tg-button>").unwrap();
    let button = &html[html[..end].rfind("<tg-button ").unwrap()..end];
    let data = button
        .split("data=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    let preview = callback(data).unwrap();
    assert_eq!(preview.action, PortfolioAction::Preview);
    assert_eq!(preview.page, 3);
}
