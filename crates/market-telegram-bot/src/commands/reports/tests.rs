use super::*;
use market_telegram::{InlineMessageId, ReplyKind};

fn event(user_id: u64, data: impl Into<Vec<u8>>) -> Event {
    Event::Callback {
        id: 1,
        user_id,
        data: data.into(),
        target: CallbackTarget::Inline(InlineMessageId::Wide {
            dc_id: 4,
            owner_id: 999,
            id: 2,
            access_hash: 3,
        }),
    }
}
fn controls(reply: &Reply) -> Vec<ReportCallback> {
    reply
        .rich_html
        .as_ref()
        .unwrap()
        .split(rich::CALLBACK_PREFIX)
        .skip(1)
        .map(|s| s.split('"').next().unwrap().parse().unwrap())
        .collect()
}
fn gift() -> views::View {
    super::super::tests::report_view()
}
fn gift_reply(reports: &Reports) -> Reply {
    let mut view = gift();
    view.state.tab = Tab::Sales;
    views::render(reports, &view, None, None)
}
#[test]
fn one_owned_record_serves_all_buttons_without_nested_tokens() {
    let reports = Reports::default();
    let raw = gift_reply(&reports);
    let destinations = controls(&raw);
    assert!(destinations.len() > 5);
    assert!(destinations.iter().all(|c| c.id == destinations[0].id));
    assert_eq!(
        reports.0.len(),
        1,
        "rendering buttons stores one view, not action destinations"
    );
    let bound = reports.reply(7, raw.clone());
    assert_eq!(
        reports.0.len(),
        1,
        "ownership attaches to the existing report"
    );
    assert_eq!(reports.reply(7, bound.clone()), bound);
    for callback in controls(&bound) {
        assert_eq!(
            reports
                .parse(&event(7, callback.to_string()), &[])
                .unwrap()
                .command,
            Command::Report(callback)
        );
        assert_eq!(
            reports
                .parse(&event(8, callback.to_string()), &[])
                .unwrap()
                .command,
            Command::CallbackDenied
        );
    }
    assert_eq!(reports.0.len(), 1);
    let entry = reports.0.get(destinations[0].id).unwrap();
    assert!(
        entry.responses.0.try_lock().unwrap().is_empty(),
        "unused buttons allocate no reply slots"
    );
    let shared = reports.reply(8, raw);
    let other = controls(&shared);
    assert_eq!(
        reports.0.len(),
        2,
        "a new sender gets an independent owner record"
    );
    assert_ne!(other[0].id, destinations[0].id);
    assert!(Arc::ptr_eq(
        &entry.state,
        &reports.0.get(other[0].id).unwrap().state
    ));
    assert_eq!(
        reports
            .parse(&event(7, other[0].to_string()), &[])
            .unwrap()
            .command,
        Command::CallbackDenied
    );
    assert_eq!(
        reports
            .parse(&event(8, other[0].to_string()), &[])
            .unwrap()
            .command,
        Command::Report(other[0])
    );
    let altered = ReportCallback::new(
        destinations[0].id,
        ReportAction::Market(browse::Destination {
            page: 0,
            mode: GridMode::Linked,
            refresh: false,
        }),
    );
    assert_eq!(
        reports
            .parse(&event(7, altered.to_string()), &[])
            .unwrap()
            .command,
        Command::CallbackExpired
    );
}
#[test]
fn portfolio_buttons_share_one_target_and_keep_absolute_pages_and_private_refresh() {
    use grammers_session::types::{PeerAuth, PeerId, PeerRef};
    let reports = Reports::default();
    let raw = Reply {
        rich_html: Some(
            [
                "p:123:GRAM:v:0:pa",
                "p:123:GRAM:v:1:vd",
                "p:123:GRAM:g:3:pa:g",
            ]
            .iter()
            .map(|data| rich::button("Go", Some(data), false))
            .collect(),
        ),
        buttons: vec![vec![
            ReplyButton::Callback {
                text: "Refresh".into(),
                data: b"p:123:GRAM:r:0:pa".to_vec(),
            },
            ReplyButton::Share {
                text: "Share".into(),
                destination: market_telegram::ShareLink::Inline("portfolio 123".into()),
            },
        ]],
        ..Reply::text("Portfolio")
    };
    let bound = reports.reply(7, raw.clone());
    let callbacks = controls(&bound);
    assert_eq!(reports.0.len(), 1);
    assert!(callbacks.iter().all(|c| c.id == callbacks[0].id));
    assert_eq!(
        bound.buttons[0][1], raw.buttons[0][1],
        "sharing remains public"
    );
    let expected = [
        (0, market_portfolio::PortfolioSort::PositionAsc),
        (1, market_portfolio::PortfolioSort::ValueDesc),
        (3, market_portfolio::PortfolioSort::PositionAsc),
    ];
    for (callback, (page, sort)) in callbacks.iter().zip(expected) {
        let request = reports.parse(&event(7, callback.to_string()), &[]).unwrap();
        let Command::Portfolio(query) = request.command else {
            panic!()
        };
        assert_eq!(
            (query.user_id.get(), query.page, query.sort, query.mode),
            (123, page, sort, PortfolioMode::Inline)
        );
        assert_eq!(
            reports
                .parse(&event(8, callback.to_string()), &[])
                .unwrap()
                .command,
            Command::CallbackDenied
        );
    }
    let ReplyButton::Callback { data, .. } = &bound.buttons[0][0] else {
        panic!()
    };
    let mut private = event(7, data.clone());
    let Event::Callback { target, .. } = &mut private else {
        panic!()
    };
    *target = CallbackTarget::Message(market_telegram::transport::Message {
        peer: PeerRef {
            id: PeerId::user(7).unwrap(),
            auth: PeerAuth::from_hash(123),
        },
        id: 1,
        outgoing: true,
        text: String::new(),
    });
    let Command::Portfolio(query) = reports.parse(&private, &[]).unwrap().command else {
        panic!()
    };
    assert_eq!(
        (query.mode, query.action),
        (PortfolioMode::Private, PortfolioAction::Refresh)
    );
    let Command::Portfolio(query) = reports.parse(&event(7, data.clone()), &[]).unwrap().command
    else {
        panic!()
    };
    assert_eq!(
        (query.mode, query.action),
        (PortfolioMode::Inline, PortfolioAction::Navigate)
    );
    let Event::Callback { data: raw, .. } = &mut private else {
        panic!()
    };
    *raw = b"p:123:GRAM:r:0:pa".to_vec();
    assert!(
        matches!(
            reports.parse(&private, &[]).unwrap().command,
            Command::Portfolio(_)
        ),
        "old private callbacks remain valid"
    );
    let Event::Callback { target, .. } = &mut private else {
        panic!()
    };
    *target = CallbackTarget::Message(market_telegram::transport::Message {
        peer: PeerRef {
            id: PeerId::chat(9).unwrap(),
            auth: PeerAuth::from_hash(123),
        },
        id: 1,
        outgoing: true,
        text: String::new(),
    });
    assert_eq!(
        reports.parse(&private, &[]).unwrap().command,
        Command::CallbackExpired,
        "raw callbacks cannot bypass ownership in groups"
    );
    let Event::Callback {
        data: bound_data,
        user_id,
        ..
    } = &mut private
    else {
        panic!()
    };
    *bound_data = data.clone();
    *user_id = 8;
    assert_eq!(
        reports.parse(&private, &[]).unwrap().command,
        Command::CallbackDenied
    );
}
#[test]
fn legacy_inline_tokens_unowned_reports_eviction_and_restart_fail_closed() {
    let reports = Reports(Cache::new("r", 1));
    let unowned = gift_reply(&reports);
    let first = controls(&unowned)[0];
    assert_eq!(
        reports
            .parse(&event(7, first.to_string()), &[])
            .unwrap()
            .command,
        Command::CallbackExpired
    );
    reports.reply(7, unowned);
    assert!(matches!(
        reports
            .parse(&event(7, first.to_string()), &[])
            .unwrap()
            .command,
        Command::Report(_)
    ));
    gift_reply(&reports);
    assert_eq!(
        reports
            .parse(&event(7, first.to_string()), &[])
            .unwrap()
            .command,
        Command::CallbackExpired
    );
    assert_eq!(
        Reports::default()
            .parse(&event(7, first.to_string()), &[])
            .unwrap()
            .command,
        Command::CallbackExpired
    );
    for raw in [
        "i:0000000000000000",
        "v:0000000000000000",
        "m:0000000000000000",
        "p:123:GRAM:v:1:pa",
    ] {
        assert_eq!(
            reports.parse(&event(7, raw), &[]).unwrap().command,
            Command::CallbackExpired
        );
    }
}
#[test]
fn callback_codec_round_trips_typed_destinations_within_the_transport_limit() {
    for tab in [Tab::Floors, Tab::Sales] {
        for scope in Scope::ALL {
            for page in 1..=history::MAX_PAGES {
                for mode in [GridMode::Linked, GridMode::Gallery] {
                    for refresh in [false, true] {
                        let callback = ReportCallback::new(
                            u64::MAX,
                            ReportAction::Gift(views::Destination {
                                tab,
                                scope,
                                page,
                                mode,
                                refresh,
                            }),
                        );
                        let data = callback.to_string();
                        assert!(data.len() <= 64);
                        assert_eq!(data.parse::<ReportCallback>(), Ok(callback));
                    }
                }
            }
        }
    }
    for data in [
        "r:0000000000000001:g:s:p:0:l:0",
        "r:0000000000000001:g:s:p:11:l:0",
        "r:0000000000000001:g:s:p:1:x:0",
        "r:0000000000000001:m:10:l:0",
        "r:0000000000000001:p:pa:1000:l:v",
        "r:0000000000000001:p:garbage:1:l:v",
        "r:0000000000000001:m:0:l:0:extra",
        "r:1:m:0:l:0",
    ] {
        assert!(data.parse::<ReportCallback>().is_err(), "{data}");
    }
    for page in [0, 9] {
        let callback = ReportCallback::new(
            1,
            ReportAction::Market(browse::Destination {
                page,
                mode: GridMode::Linked,
                refresh: false,
            }),
        );
        assert_eq!(callback.to_string().parse(), Ok(callback));
    }
    for sort in [
        market_portfolio::PortfolioSort::PositionAsc,
        market_portfolio::PortfolioSort::PositionDesc,
        market_portfolio::PortfolioSort::ValueAsc,
        market_portfolio::PortfolioSort::ValueDesc,
        market_portfolio::PortfolioSort::NumberAsc,
        market_portfolio::PortfolioSort::NumberDesc,
    ] {
        let callback = ReportCallback::new(
            u64::MAX,
            ReportAction::Portfolio {
                page: 999,
                sort,
                layout: GridMode::Gallery,
                action: PortfolioAction::Preview,
            },
        );
        assert!(callback.to_string().len() <= 64);
        assert_eq!(callback.to_string().parse(), Ok(callback));
    }
}
#[tokio::test]
async fn requested_destination_replies_coalesce_and_cancellation_leaves_a_retryable_slot() {
    let responses = Responses::default();
    let action = ReportAction::Market(browse::Destination {
        page: 1,
        mode: GridMode::Linked,
        refresh: false,
    });
    let slot = responses.slot(action).await;
    assert!(Arc::ptr_eq(&slot, &responses.slot(action).await));
    let (started, ready) = tokio::sync::oneshot::channel();
    let working = slot.clone();
    let task = tokio::spawn(async move {
        working
            .get_or_init(|| async {
                started.send(()).unwrap();
                std::future::pending::<Reply>().await
            })
            .await
            .clone()
    });
    ready.await.unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(slot.get().is_none());
    assert!(
        slot.get_or_try_init(|| async { Err::<Reply, _>(Reply::notice("Retry")) })
            .await
            .is_err()
    );
    assert!(slot.get().is_none());
    let calls = std::sync::atomic::AtomicUsize::new(0);
    let build = || async {
        calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        tokio::task::yield_now().await;
        Reply::text("Page 2")
    };
    let (first, second) = tokio::join!(slot.get_or_init(build), slot.get_or_init(build));
    assert_eq!(first, second);
    assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 1);
    assert_eq!(first.kind, ReplyKind::Message);
    for page in 0..10 {
        responses
            .slot(ReportAction::Portfolio {
                page,
                sort: market_portfolio::PortfolioSort::PositionAsc,
                layout: GridMode::Linked,
                action: PortfolioAction::Navigate,
            })
            .await;
    }
    assert_eq!(responses.0.lock().await.len(), 4);
}
