use super::*;
use market_gifts::*;
use market_telegram::{CallbackTarget, Event, InlineMessageId, ReplyButton};
use std::sync::Arc;
pub(super) fn view() -> Arc<report::Report> {
    let mut value = render::tests::fixture();
    value.target = market_valuation::ResolvedTarget::Gift {
        gift: CollectibleGift {
            id: "gift".into(),
            collection_id: "collection".into(),
            number: Some(GiftNumber::new(123).unwrap()),
            traits: GiftTraits {
                model: Some("m".into()),
                backdrop: Some("b".into()),
                pattern: Some("p".into()),
            },
        },
        slug: Some(GiftSlug::new("Gift-123").unwrap()),
    };
    Arc::new(report::Report {
        query: ValueRequest {
            gift: GiftSlug::new("Gift-123").unwrap(),
            currency_id: "GRAM".into(),
        },
        value: Arc::new(value),
        floors: report::FloorQuotes::default(),
        artwork: None,
        icons: Default::default(),
        catalog: Some(GiftCollectionCatalog {
            collection: GiftCollection {
                aliases: Default::default(),
                id: "collection".into(),
                name: "Gift <unsafe> & friends".into(),
                slug: Some(CollectionSlug::new("Gift").unwrap()),
                preview: None,
            },
            models: Arc::new(vec![GiftModel {
                aliases: Default::default(),
                id: "m".into(),
                name: "<b>Model</b>".into(),
                media: None,
                rarity: None,
            }]),
            backdrops: Arc::new(vec![]),
            patterns: Arc::new(vec![]),
        }),
    })
}
pub(super) fn report_view() -> views::View {
    views::View::initial(view()).unwrap()
}
fn callback(html: &str, label: &str) -> ReportCallback {
    let end = html.find(&format!(">{label}</tg-button>")).unwrap();
    let start = html[..end].rfind("<tg-button ").unwrap();
    let token = html[start..end]
        .split("data=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    token.parse().unwrap()
}
#[test]
fn entry_points_share_value_requests_without_trusting_payload_identities() {
    let inline = Event::Inline {
        id: 1,
        user_id: 7,
        query: "https://t.me/nft/Gift-123 TON".into(),
        offset: String::new(),
    };
    let guest = Event::Guest {
        id: 2,
        user_id: 7,
        text: "@market_bot please value this in TON".into(),
        references: vec!["Look: https://t.me/nft/Gift-123.".into()],
    };
    let aliases = vec!["market_bot".into()];
    let a = interactions::parse_event(&inline, &aliases).unwrap();
    let b = interactions::parse_event(&guest, &aliases).unwrap();
    assert_eq!(a, b);
    let mut invalid = inline.clone();
    if let Event::Inline { query, .. } = &mut invalid {
        *query = "Gift-123 Other-1".into();
    }
    assert!(interactions::parse_event(&invalid, &aliases).is_none());
    if let Event::Inline { offset, .. } = &mut invalid {
        *offset = "next".into();
    }
    assert!(interactions::parse_event(&invalid, &aliases).is_none());
    for text in ["@market_bot Gift-123 Other-1", "@market_bot nonsense"] {
        let event = Event::Guest {
            id: 2,
            user_id: 7,
            text: text.into(),
            references: vec![],
        };
        assert!(matches!(
            interactions::parse_event(&event, &aliases).unwrap().command,
            Command::GiftHelp
        ));
    }
    let event = Event::Guest {
        id: 2,
        user_id: 7,
        text: "@market_bot Gift-123".into(),
        references: vec!["Other-1".into()],
    };
    assert!(
        matches!(
            interactions::parse_event(&event, &aliases).unwrap().command,
            Command::Gift(_)
        ),
        "explicit gift takes precedence over reply context"
    );
}
#[test]
fn rich_report_escapes_catalog_content_and_links_use_the_existing_deep_link_codec() {
    let report = view();
    let reply = report.render().bounded();
    let payload = reply
        .buttons
        .iter()
        .flatten()
        .find_map(|b| match b {
            ReplyButton::OpenApp { payload, .. } => Some(payload.clone()),
            _ => None,
        })
        .unwrap();
    let html = reply.rich_html.unwrap();
    assert!(html.contains("Gift &lt;unsafe&gt; &amp; friends"));
    assert!(html.contains("&lt;b&gt;Model&lt;/b&gt;"));
    assert!(html.contains("<b>Estimate</b>"));
    assert!(!html.contains("UTC"));
    assert!(!reply.text.contains("UTC"));
    for removed in [
        "How calculated",
        "<details>",
        "tg-math-block",
        "multiplier",
        "premium",
        "unique observed gifts",
    ] {
        assert!(!html.contains(removed), "{removed}");
        assert!(!reply.text.contains(removed), "{removed}");
    }
    assert!(html.contains("href=\"https://t.me/nft/Gift-123\""));
    assert!(
        !reply
            .buttons
            .iter()
            .flatten()
            .any(|button| matches!(button, ReplyButton::Url { .. })),
        "gift link lives in the title"
    );
    assert_eq!(
        market_links::Target::from_launch(&payload).unwrap(),
        market_links::Target::Gift {
            slug: GiftSlug::new("Gift-123").unwrap(),
            currency_id: "GRAM".into()
        }
    );
    assert!(payload.len() <= 512);
}
#[test]
fn covers_use_the_selected_backdrop_and_resolved_model_and_symbol_documents() {
    let mut report = view();
    let report = Arc::get_mut(&mut report).unwrap();
    report.catalog.as_mut().unwrap().backdrops = Arc::new(vec![GiftBackdrop {
        aliases: Default::default(),
        id: "b".into(),
        name: "Backdrop".into(),
        rarity: None,
        palette: Some(BackdropPalette {
            center: "#123456".to_owned().try_into().unwrap(),
            edge: "#abcdef".to_owned().try_into().unwrap(),
            pattern: "#654321".to_owned().try_into().unwrap(),
        }),
    }]);
    report.set_artwork(
        TelegramDocumentId::new(11).unwrap(),
        Some(TelegramDocumentId::new(22).unwrap()),
    );
    let market_media::Artwork::Gift(cover) = report.render().artwork.unwrap() else {
        panic!("gift cover")
    };
    assert_eq!(cover.model.get(), 11);
    assert_eq!(cover.pattern.unwrap().get(), 22);
    assert_eq!(
        cover.palette,
        Some(market_media::CoverPalette {
            center: 0x123456,
            edge: 0xabcdef,
            pattern: 0x654321
        })
    );
}
#[test]
fn price_formatting_is_consistent_in_rich_plain_and_inline_summaries() {
    use market_valuation::*;
    for (input, expected) in [
        ("11.40", "11.4"),
        ("1000.000", "1 000"),
        ("1234.5678", "1 234.57"),
        ("0.004", "0"),
    ] {
        let mut report = view();
        let report = Arc::get_mut(&mut report).unwrap();
        let value = Arc::get_mut(&mut report.value).unwrap();
        let StrategyOutcome::Value(StrategyValuation::GiftFloor(floor)) = &mut value.results[0]
        else {
            panic!()
        };
        floor.final_mark = ValuationMetric::Value {
            value: Amount::new(input).unwrap(),
        };
        let reply = report.render().bounded();
        assert!(reply.text.contains(&format!("Estimate: {expected} GRAM")));
        assert!(reply.summary.unwrap().contains(&format!("{expected} GRAM")));
        let html = reply.rich_html.unwrap();
        assert!(html.contains(&format!("<h1>{expected} <tg-emoji")));
        assert!(!html.contains("Bot preview test"));
        assert!(
            !html.contains("💎 GRAM"),
            "custom emoji alternatives must not become standalone icons"
        );
    }
}
#[test]
fn report_icons_keep_custom_emoji_fallbacks_and_one_artwork_reference() {
    use crate::{ReportIcon, ReportIcons};
    let mut shared_report = view();
    let report = Arc::get_mut(&mut shared_report).unwrap();
    report.artwork = Some(market_media::GiftCover {
        model: market_media::DocumentId::new(777).unwrap(),
        pattern: None,
        palette: None,
    });
    report.icons = Arc::new(
        ReportIcons::default()
            .with_custom(
                ReportIcon::Model,
                TelegramDocumentId::new(123).unwrap(),
                "🧸",
            )
            .unwrap(),
    );
    let reply = report.render().bounded();
    let html = reply.rich_html.unwrap();
    assert!(html.contains("<tg-emoji emoji-id=\"123\">🧸</tg-emoji> &lt;b&gt;Model&lt;/b&gt;"));
    assert!(reply.text.contains("🧸 <b>Model</b>"));
    assert!(!reply.text.contains("tg-emoji"));
    assert_eq!(reply.artwork, report.artwork.map(Into::into));
    assert!(
        !html.contains("<img"),
        "only the transport adds the single large artwork"
    );
    let views = cache::Cache::new("t", 256);
    let token = views.insert(views::View::initial(shared_report.clone()).unwrap());
    let id = u64::from_str_radix(std::str::from_utf8(&token[2..]).unwrap(), 16).unwrap();
    let restored = views.get(id).unwrap();
    assert!(Arc::ptr_eq(&shared_report.icons, &restored.report.icons));
    assert!(
        restored
            .report
            .render()
            .rich_html
            .unwrap()
            .contains("<tg-emoji emoji-id=\"123\">🧸</tg-emoji>")
    );
}
#[test]
fn callback_cache_is_bounded_and_tokens_expire_without_sliding_their_deadline() {
    let views = cache::Cache::new("t", 256);
    let token = views.insert(report_view());
    assert!(token.len() <= 64);
    let event = Event::Callback {
        id: 1,
        user_id: 42,
        data: token,
        target: CallbackTarget::Inline(InlineMessageId::Standard {
            dc_id: 2,
            id: 1,
            access_hash: 1,
        }),
    };
    let Event::Callback { data, .. } = event else {
        panic!()
    };
    let id = interactions::callback_id(std::str::from_utf8(&data[2..]).unwrap()).unwrap();
    assert!(views.get(id).is_some());
    for _ in 0..256 {
        views.insert(report_view());
    }
    assert!(views.get(id).is_none());
    assert!(
        interactions::parse_event(
            &Event::Callback {
                id: 1,
                user_id: 42,
                data: b"v:1;user_id=9".to_vec(),
                target: CallbackTarget::Inline(InlineMessageId::Standard {
                    dc_id: 2,
                    id: 1,
                    access_hash: 1
                })
            },
            &[]
        )
        .is_none()
    );
}
#[test]
fn similar_sales_are_bounded_executions_with_explicit_matching_and_fixed_period() {
    use history::{SalesState, Scope};
    let report = view();
    let gift = report.gift().unwrap();
    let mut request = SalesState {
        scope: Scope::Pair,
        until: report.value.evaluated_at,
        starts: vec![],
    };
    let query = request.query(gift, "GRAM".into()).unwrap();
    assert_eq!(query.source, OrderSource::History);
    assert_eq!(query.limit, history::PAGE_SIZE);
    assert_eq!(query.sort, Some(GiftSort::TimeDesc));
    assert_eq!(query.filters.models, vec![ModelId::from("m")]);
    assert_eq!(query.filters.backdrops, vec![BackdropId::from("b")]);
    assert!(query.filters.patterns.is_empty());
    assert_eq!(query.period.unwrap().until, Some(report.value.evaluated_at));
    request.scope = Scope::Collection;
    let query = request.query(gift, "GRAM".into()).unwrap();
    assert!(query.filters.models.is_empty() && query.filters.backdrops.is_empty());
    request.starts = vec![OrderKey::try_from("00".to_owned()).unwrap(); 10];
    assert!(request.query(gift, "GRAM".into()).is_err());
}

#[actix::test]
async fn tabs_dual_floors_pagination_and_refresh_share_reports_and_keep_view_state() {
    use market_hub_protocol::wire as w;
    let dir = tempfile::tempdir().unwrap();
    let mut writer = market_replica::ReplicaWriter::open(dir.path(), "test").unwrap();
    let entity = |entity| w::Change {
        change: Some(w::change::Change::Upsert(w::Entity {
            entity: Some(entity),
        })),
    };
    let mut rows = vec![
        entity(w::entity::Entity::Currency(w::Currency {
            id: "GRAM".into(),
            name: "TON".into(),
            decimals: 9,
        })),
        entity(w::entity::Entity::Currency(w::Currency {
            id: "XTR".into(),
            name: "Stars".into(),
            decimals: 0,
        })),
        entity(w::entity::Entity::Venue(w::Venue {
            id: "v".into(),
            name: "Venue".into(),
        })),
        entity(w::entity::Entity::ItemType(w::ItemType {
            id: "collection".into(),
            name: "Gift".into(),
            quantity_step: "1".into(),
            aliases: vec![w::ExternalId {
                namespace: "tg.slug".into(),
                id: "Gift".into(),
            }],
            attributes: [
                (
                    "tg.model".into(),
                    w::AttributeRule {
                        required: false,
                        allowed_values: Some(w::AllowedAttributeValues {
                            ids: vec!["m".into()],
                        }),
                    },
                ),
                (
                    "tg.backdrop".into(),
                    w::AttributeRule {
                        required: false,
                        allowed_values: Some(w::AllowedAttributeValues {
                            ids: vec!["b".into()],
                        }),
                    },
                ),
                ("tg.number".into(), w::AttributeRule::default()),
            ]
            .into(),
        })),
    ];
    for (kind, id) in [("tg.model", "m"), ("tg.backdrop", "b")] {
        rows.push(entity(w::entity::Entity::Attribute(w::Attribute {
            id: kind.into(),
            name: kind.into(),
            value_type: w::AttributeType::Enum as i32,
            ..Default::default()
        })));
        rows.push(entity(w::entity::Entity::AttributeOption(
            w::AttributeOption {
                id: id.into(),
                attribute_id: kind.into(),
                name: id.into(),
                ..Default::default()
            },
        )));
    }
    rows.push(entity(w::entity::Entity::Attribute(w::Attribute {
        id: "tg.number".into(),
        name: "Number".into(),
        value_type: w::AttributeType::Integer as i32,
        ..Default::default()
    })));
    let now = jiff::Timestamp::now().as_millisecond();
    for i in 0..7 {
        let predicate = |value| w::AttributePredicate {
            predicate: Some(w::attribute_predicate::Predicate::Eq(w::AttributeValue {
                value: Some(value),
            })),
        };
        rows.push(entity(w::entity::Entity::Order(w::Order {
            id: format!("o{i}"),
            book: Some(w::BookKey {
                venue_id: "v".into(),
                item_type_id: "collection".into(),
                currency_id: "GRAM".into(),
            }),
            mode: if i == 0 {
                w::OrderMode::Ask
            } else {
                w::OrderMode::Executed
            } as i32,
            price: format!("{}", 10 + i),
            quantity: "1".into(),
            item_id: Some(format!("g{i}")),
            observed_at_ms: now - 1000,
            executed_at_ms: (i > 0).then_some(now - i * 1000),
            attributes: [
                (
                    "tg.model".into(),
                    predicate(w::attribute_value::Value::OptionId("m".into())),
                ),
                (
                    "tg.backdrop".into(),
                    predicate(w::attribute_value::Value::OptionId("b".into())),
                ),
                (
                    "tg.number".into(),
                    predicate(w::attribute_value::Value::Integer(123 + i)),
                ),
            ]
            .into(),
            ..Default::default()
        })));
    }
    let mut stars_ask = rows
        .iter()
        .find_map(|row| match row.change.as_ref()? {
            w::change::Change::Upsert(entity) => match entity.entity.as_ref()? {
                w::entity::Entity::Order(order) if order.mode == w::OrderMode::Ask as i32 => {
                    Some(order.clone())
                }
                _ => None,
            },
            _ => None,
        })
        .unwrap();
    stars_ask.id = "stars-ask".into();
    stars_ask.book.as_mut().unwrap().currency_id = "XTR".into();
    stars_ask.price = "1500".into();
    rows.push(entity(w::entity::Entity::Order(stars_ask)));
    let mut exact_sale = rows
        .iter()
        .find_map(|row| match row.change.as_ref()? {
            w::change::Change::Upsert(entity) => match entity.entity.as_ref()? {
                w::entity::Entity::Order(order) if order.id == "o0" => Some(order.clone()),
                _ => None,
            },
            _ => None,
        })
        .unwrap();
    exact_sale.id = "exact-sale".into();
    exact_sale.mode = w::OrderMode::Executed as i32;
    exact_sale.executed_at_ms = Some(now - 500);
    exact_sale.meta.insert(
        "url".into(),
        "https://example.com/sale?gift=123&venue=v".into(),
    );
    rows.push(entity(w::entity::Entity::Order(exact_sale)));
    for n in 201..=214 {
        let mut ask = rows
            .iter()
            .find_map(|change| match change.change.as_ref()? {
                w::change::Change::Upsert(entity) => match entity.entity.as_ref()? {
                    w::entity::Entity::Order(order) if order.id == "o0" => Some(order.clone()),
                    _ => None,
                },
                _ => None,
            })
            .unwrap();
        ask.id = format!("extra-{n}");
        ask.item_id = Some(format!("gift-{n}"));
        ask.price = (n - 180).to_string();
        ask.attributes.insert(
            "tg.number".into(),
            w::AttributePredicate {
                predicate: Some(w::attribute_predicate::Predicate::Eq(w::AttributeValue {
                    value: Some(w::attribute_value::Value::Integer(n)),
                })),
            },
        );
        rows.push(entity(w::entity::Entity::Order(ask)));
    }
    let mut snapshot = writer.begin_snapshot().unwrap();
    snapshot.append(&rows).unwrap();
    writer
        .install_snapshot(snapshot.finish("ready").unwrap())
        .unwrap();
    let reads = market_replica::ReadRuntime::new(
        writer.reader(),
        4,
        tokio_util::sync::CancellationToken::new(),
    )
    .unwrap();
    let data = market_actors::MarketDataService::start(reads.handle(), 4)
        .await
        .unwrap();
    let (valuation, task) =
        market_valuation::ValuationHandle::start(&data.arbiter(), reads.handle())
            .await
            .unwrap();
    let commands = MarketCommands::new(data.handle.clone(), valuation).with_report_icons(
        crate::ReportIcons::default()
            .with_custom(
                crate::ReportIcon::Backdrop,
                TelegramDocumentId::new(123).unwrap(),
                "🎨",
            )
            .unwrap(),
    );
    let market = commands
        .browse(browse::parse("/market collection=@slug:Gift currency=stars").unwrap())
        .await;
    let market_html = market.rich_html.as_ref().unwrap();
    assert!(market_html.contains("<h3>Best asks · Gift</h3>"));
    assert!(market_html.contains("<table striped compact>"));
    assert_eq!(market_html.matches("<td>").count(), 3);
    assert!(market_html.contains("https://t.me/nft/Gift-123"));
    assert!(market_html.contains("<b>1 500</b>"));
    assert!(market_html.contains(&crate::ReportIcons::default().html(crate::ReportIcon::Stars)));
    assert!(market.text.contains("#123 · 1 500 XTR"));
    assert!(!market.buttons.is_empty());
    let market_page = commands
        .browse(browse::parse("/market collection=@slug:Gift").unwrap())
        .await;
    assert_eq!(
        market_page
            .rich_html
            .as_ref()
            .unwrap()
            .matches("https://t.me/nft/")
            .count(),
        12
    );
    let market_forward_id = callback(market_page.rich_html.as_ref().unwrap(), "→");
    let (market_next, duplicate) = tokio::join!(
        commands.report(market_forward_id),
        commands.report(market_forward_id)
    );
    assert_eq!(market_next, duplicate);
    let jump = ReportCallback::new(
        market_forward_id.id,
        reports::ReportAction::Market(browse::Destination {
            page: 9,
            mode: GridMode::Linked,
            refresh: false,
        }),
    );
    assert_eq!(
        commands.report(jump).await.kind,
        market_telegram::ReplyKind::Notice,
        "forged jumps cannot trigger a scan through unloaded pages"
    );
    assert!(market_next.text.contains("#212"));
    assert!(!market_next.text.contains("#211"));
    let market_back = commands
        .report(callback(market_next.rich_html.as_ref().unwrap(), "←"))
        .await;
    assert!(market_back.text.contains("#201"));
    let cached_market_next = callback(market_back.rich_html.as_ref().unwrap(), "→");
    let pending_market = commands
        .browse(browse::parse("/market collection=@slug:Gift").unwrap())
        .await;
    let pending_market_next = callback(pending_market.rich_html.as_ref().unwrap(), "→");
    let reply = commands
        .value(ValueRequest {
            gift: GiftSlug::new("Gift-123").unwrap(),
            currency_id: "GRAM".into(),
        })
        .await;
    let html = reply.rich_html.as_ref().unwrap();
    assert!(html.contains("Gift #123"));
    assert!(html.contains("<table compact>"));
    assert!(html.contains("<th align=\"right\">"));
    assert!(html.contains(&format!(
        "<tr><td>{} Collection</td><td align=\"right\">10</td><td align=\"right\">1 500</td></tr>",
        crate::ReportIcons::default().html(crate::ReportIcon::Collection)
    )));
    assert!(html.contains("Stars</th>"));
    assert!(!html.contains(">GRAM</tg-button>"));
    assert!(!html.contains(">Stars</tg-button>"));
    assert_eq!(
        html.matches("<tg-emoji emoji-id=\"123\">🎨</tg-emoji>")
            .count(),
        1,
        "attributes appear once in the table"
    );
    assert!(!html.contains("<td>1</td>"), "sample counts are omitted");
    for removed in ["How calculated", "Currency:", "multiplier", "premium"] {
        assert!(!reply.text.contains(removed));
    }
    assert!(!html.contains("<details"));
    assert!(
        !reply.text.contains("Recent sales"),
        "valuation starts without a history read"
    );
    assert!(html.contains("type=\"disabled\" style=\"primary\">Floors</tg-button>"));
    // Exercise the actual inline entry point and handler, rather than just the renderer.
    let inline_request = commands
        .parse_event(
            &Event::Inline {
                id: 100,
                user_id: 7,
                query: "Gift-123".into(),
                offset: String::new(),
            },
            &[],
        )
        .unwrap();
    assert!(inline_request.inline);
    let inline_reply = commands.handle(inline_request).await.unwrap();
    let protected_sales = callback(inline_reply.rich_html.as_ref().unwrap(), "Sales");
    let inline_event = |user_id| Event::Callback {
        id: 101,
        user_id,
        data: protected_sales.to_string().into_bytes(),
        target: CallbackTarget::Inline(InlineMessageId::Standard {
            dc_id: 2,
            id: 3,
            access_hash: 4,
        }),
    };
    let denied = commands.parse_event(&inline_event(8), &[]).unwrap();
    assert_eq!(denied.command, Command::CallbackDenied);
    let (updates, mut progress) = tokio::sync::watch::channel(Reply::default());
    let notice = commands
        .handle_with_progress(denied, updates)
        .await
        .unwrap();
    assert_eq!(notice.kind, market_telegram::ReplyKind::Notice);
    assert!(notice.text.contains("Only the person"));
    assert!(notice.rich_html.is_none() && notice.artwork.is_none() && notice.buttons.is_empty());
    assert!(
        progress.changed().await.is_err(),
        "rejected callbacks never emit progress edits"
    );
    let allowed = commands.parse_event(&inline_event(7), &[]).unwrap();
    let inline_sales = commands.handle(allowed).await.unwrap();
    let inline_html = inline_sales.rich_html.as_ref().unwrap();
    assert!(inline_html.contains("type=\"callback_data\" data=\"r:"));
    assert!(!inline_html.contains("type=\"callback_data\" data=\"v:"));
    assert!(
        inline_sales.text.contains("#124"),
        "creator can navigate the report"
    );
    assert!(
        inline_reply
            .rich_html
            .as_ref()
            .unwrap()
            .contains("type=\"disabled\" style=\"primary\">Floors"),
        "the original cached reply stays unchanged"
    );
    let sales_id = callback(html, "Sales");
    let (sales, duplicate) = tokio::join!(commands.report(sales_id), commands.report(sales_id));
    let sales_html = sales.rich_html.as_ref().unwrap();
    let jump = ReportCallback::new(
        sales_id.id,
        reports::ReportAction::Gift(views::Destination {
            tab: views::Tab::Sales,
            scope: history::Scope::Pair,
            page: 10,
            mode: GridMode::Linked,
            refresh: false,
        }),
    );
    assert_eq!(
        commands.report(jump).await.kind,
        market_telegram::ReplyKind::Notice,
        "sales require a known page boundary rather than scanning to an arbitrary page"
    );
    assert_eq!(
        sales.rich_html, duplicate.rich_html,
        "duplicate clicks reuse one rendered result"
    );
    assert!(
        !sales_html.contains("<th align=\"left\">Attribute</th>"),
        "sales replace the floor table"
    );
    assert!(
        !sales_html.contains("<details"),
        "there is no disclosure to collapse on navigation"
    );
    assert!(sales_html.contains("UTC"));
    assert_eq!(
        sales_html.matches("UTC").count(),
        1,
        "timezone appears once"
    );
    assert!(sales_html.contains("<br>"));
    assert!(sales_html.contains("<table striped compact>"));
    assert!(sales_html.contains("<b>10</b> <tg-emoji"));
    assert!(!sales_html.contains("<th>"));
    assert!(sales_html.contains("30 days · UTC"));
    assert!(!sales_html.contains("This gift"));
    assert!(!sales_html.contains("history may be incomplete"));
    assert!(sales_html.contains("<b>#123</b>"));
    assert!(sales_html.contains("https://example.com/sale?gift=123&amp;venue=v"));
    // Real typed ordering keys stay opaque to preview rendering. Captures are filtered,
    // sorted and bounded using the same period and selected traits as live reads.
    let source_view = commands.reports.gift_view(sales_id).unwrap();
    let query = source_view
        .state
        .sales
        .query(source_view.report.gift().unwrap(), "GRAM".into())
        .unwrap();
    let live = commands
        .market
        .query_orders(market_actors::input::query(query.clone()))
        .await
        .unwrap()
        .result;
    let mut captured = live.data.clone();
    let mut gallery_rows = live.data[..1].to_vec();
    let mut gift = gallery_rows[0].order.clone().into_data();
    let GiftTarget::Gift { gift: target } = &mut gift.target else {
        panic!("gift")
    };
    target.traits.pattern = Some("p".into());
    gallery_rows[0].order = GiftOrder::try_from(gift).unwrap();
    let mut bulk = gallery_rows[0].clone();
    let mut draft = bulk.order.into_data();
    draft.kind = GiftOrderKind::Bid;
    draft.executed_at = None;
    draft.quantity = 5;
    draft.book.currency_id = "XTR".into();
    draft.target = GiftTarget::Criteria {
        criteria: GiftCriteria {
            models: vec!["m".into(), "other".into()],
            ..Default::default()
        },
    };
    bulk.order = GiftOrder::try_from(draft).unwrap();
    gallery_rows.push(bulk);
    let mut catalog = source_view.report.catalog.clone().unwrap();
    catalog.backdrops = Arc::new(vec![GiftBackdrop {
        aliases: Default::default(),
        id: "b".into(),
        name: "Blue".into(),
        rarity: None,
        palette: Some(BackdropPalette {
            center: "#123456".to_owned().try_into().unwrap(),
            edge: "#654321".to_owned().try_into().unwrap(),
            pattern: "#112233".to_owned().try_into().unwrap(),
        }),
    }]);
    let market_media::Artwork::Gallery(gallery) = gallery::order_cells(
        &gallery_rows,
        &[("m".into(), TelegramDocumentId::new(11).unwrap())].into(),
        &[(catalog.collection.id.clone(), catalog)].into(),
        &[("p".into(), market_media::DocumentId::new(22).unwrap())].into(),
        2,
    ) else {
        panic!("gallery")
    };
    assert_eq!(gallery.columns, 2);
    assert_eq!(gallery.cells.len(), 2);
    let cover = gallery.cells[0].cover.unwrap();
    assert_eq!(cover.model.get(), 11);
    assert_eq!(cover.pattern.unwrap().get(), 22);
    assert_eq!(cover.palette.unwrap().center, 0x123456);
    assert_eq!(
        gallery.cells[1].cover, None,
        "ambiguous bids cannot invent artwork"
    );
    assert_eq!(gallery.cells[1].identity, "× 5");
    assert!(gallery.cells[1].price.ends_with(" XTR"));
    captured.reverse();
    for time in [
        query
            .period
            .as_ref()
            .unwrap()
            .from
            .unwrap()
            .checked_sub(jiff::SignedDuration::from_secs(1))
            .unwrap(),
        query.period.as_ref().unwrap().until.unwrap(),
    ] {
        let mut row = live.data[0].clone();
        let mut draft = row.order.into_data();
        draft.executed_at = Some(time);
        draft.observed_at = time;
        row.order = GiftOrder::try_from(draft).unwrap();
        captured.push(row);
    }
    captured.push(live.data[0].clone());
    let preview =
        history::captured_page(&source_view.report, &source_view.state.sales, captured).unwrap();
    assert_eq!(preview.data.len(), history::PAGE_SIZE as usize);
    assert!(
        preview
            .data
            .windows(2)
            .all(|rows| rows[0].order.executed_at >= rows[1].order.executed_at)
    );
    assert!(preview.data.iter().all(|row| row.order.executed_at
        >= query.period.as_ref().unwrap().from
        && row.order.executed_at < query.period.as_ref().unwrap().until));

    assert!(sales.text.contains("#124"));
    assert!(!sales.text.contains("#129"));
    assert!(sales_html.contains("type=\"disabled\">←</tg-button>"));
    assert_eq!(sales.artwork, reply.artwork);
    let next_id = callback(sales_html, "→");
    let next = commands.report(next_id).await;
    assert!(next.text.contains("#129"));
    assert!(!next.text.contains("#124"));
    let next_html = next.rich_html.as_ref().unwrap();
    assert!(next_html.contains(">2</tg-button>"));
    assert!(next_html.contains("type=\"disabled\">→</tg-button>"));
    let previous_id = callback(next_html, "←");
    let back = commands.report(previous_id).await;
    assert!(back.text.contains("#124"));
    assert_eq!(
        commands.report(next_id).await.rich_html,
        next.rich_html,
        "repeated Next does not advance twice"
    );
    let overview = commands.report(callback(next_html, "Floors")).await;
    let overview_html = overview.rich_html.as_ref().unwrap();
    assert!(overview_html.contains("<th align=\"left\">Attribute</th>"));
    assert!(!overview.text.contains("Recent sales"));
    let returned = commands.report(callback(overview_html, "Sales")).await;
    assert!(
        returned
            .rich_html
            .as_ref()
            .unwrap()
            .contains(">2</tg-button>"),
        "tabs preserve the visited sales page"
    );
    let scoped = commands.report(callback(next_html, "Collection")).await;
    assert!(
        scoped
            .rich_html
            .as_ref()
            .unwrap()
            .contains(">1</tg-button>")
    );
    assert!(scoped.text.contains("#124"));
    let refreshed = commands.report(callback(next_html, "↻")).await;
    assert!(
        refreshed
            .rich_html
            .as_ref()
            .unwrap()
            .contains(">2</tg-button>")
    );
    assert!(refreshed.text.contains("#129"));
    // Bind the trusted renderer fixtures just as the ordinary command entry point does.
    commands.reports.reply(7, market_next.clone());
    commands.reports.reply(7, next.clone());
    // Gallery is an explicit snapshot, never another paginated image stream.
    let market_gallery = commands
        .handle(CommandRequest {
            command: Command::Report(callback(market_next.rich_html.as_ref().unwrap(), "Gallery")),
            user_id: 7,
            message_id: 0,
            inline: false,
        })
        .await
        .unwrap();
    let gallery_html = market_gallery.rich_html.as_ref().unwrap();
    assert!(matches!(
        market_gallery.artwork,
        Some(market_media::Artwork::Gallery(_))
    ));
    assert!(gallery_html.contains(">Browse</tg-button>"));
    for label in ["→", "←", "↻"] {
        assert!(!gallery_html.contains(&format!(">{label}</tg-button>")));
    }
    let snapshot_query = market_gallery
        .buttons
        .iter()
        .flatten()
        .find_map(|b| match b {
            ReplyButton::Share {
                destination: market_telegram::ShareLink::Inline(query),
                ..
            } => Some(query.clone()),
            _ => None,
        })
        .unwrap();
    assert!(!snapshot_query.starts_with("s:"));
    let shared_request = commands
        .parse_event(
            &Event::Inline {
                id: 201,
                user_id: 999,
                query: snapshot_query.clone(),
                offset: String::new(),
            },
            &[],
        )
        .unwrap();
    assert!(matches!(shared_request.command, Command::Browse(_)));
    let other_user = commands.handle(shared_request).await.unwrap();
    assert!(
        other_user.artwork.is_none(),
        "resharing resolves the target into fast linked browsing"
    );
    assert!(
        other_user.text.contains("#201"),
        "the current target starts at its first page"
    );
    assert_eq!(
        commands
            .report(callback(market_next.rich_html.as_ref().unwrap(), "Gallery"))
            .await,
        market_gallery,
        "repeated explicit previews still reuse rendering"
    );
    let browsed = commands.report(callback(gallery_html, "Browse")).await;
    assert!(browsed.artwork.is_none());
    assert!(browsed.text.contains("#212"));
    let sales_gallery = commands
        .handle(CommandRequest {
            command: Command::Report(callback(next_html, "Gallery")),
            user_id: 7,
            message_id: 0,
            inline: false,
        })
        .await
        .unwrap();
    assert!(
        matches!(&sales_gallery.artwork, Some(market_media::Artwork::Gallery(g) ) if g.columns == 2)
    );
    for label in ["→", "←", "↻", "Collection", "Model"] {
        assert!(
            !sales_gallery
                .rich_html
                .as_ref()
                .unwrap()
                .contains(&format!(">{label}</tg-button>"))
        );
    }
    let sales_browsed = commands
        .report(callback(
            sales_gallery.rich_html.as_ref().unwrap(),
            "Browse",
        ))
        .await;
    assert!(!matches!(
        sales_browsed.artwork,
        Some(market_media::Artwork::Gallery(_))
    ));
    assert!(
        sales_browsed
            .rich_html
            .as_ref()
            .unwrap()
            .contains(">2</tg-button>")
    );
    task.shutdown().await.unwrap();
    let stop = tokio_util::sync::CancellationToken::new();
    stop.cancel();
    data.task.run(stop).await.unwrap();
    assert!(
        commands
            .report(cached_market_next)
            .await
            .text
            .contains("#212"),
        "visited market pages survive service shutdown"
    );
    let expired = commands.report(pending_market_next).await;
    assert_eq!(expired.kind, market_telegram::ReplyKind::Notice);
    assert!(expired.text.contains("Refresh"));
    assert_eq!(
        commands.report(callback(market_html, "↻")).await.kind,
        market_telegram::ReplyKind::Notice,
        "a failed refresh must preserve the displayed market page",
    );
    assert_eq!(
        commands
            .report(ReportCallback {
                id: u64::MAX,
                ..sales_id
            })
            .await
            .kind,
        market_telegram::ReplyKind::Notice
    );

    // Previously visited pages and quote columns still work after their services stop.
    let cached_back = commands
        .report(callback(refreshed.rich_html.as_ref().unwrap(), "←"))
        .await;
    assert!(cached_back.text.contains("#124"));
    let cached_valuation = commands
        .report(callback(cached_back.rich_html.as_ref().unwrap(), "Floors"))
        .await;
    assert!(
        cached_valuation
            .rich_html
            .as_ref()
            .unwrap()
            .contains("<th align=\"left\">Attribute</th>")
    );
    let cached_sales = commands
        .report(callback(
            cached_valuation.rich_html.as_ref().unwrap(),
            "Sales",
        ))
        .await;
    assert!(cached_sales.text.contains("#124"));
    assert!(
        !cached_sales
            .rich_html
            .as_ref()
            .unwrap()
            .contains("Retry sales")
    );
    let failed_refresh = commands
        .report(callback(refreshed.rich_html.as_ref().unwrap(), "↻"))
        .await;
    let failed_html = failed_refresh.rich_html.as_ref().unwrap();
    assert!(!failed_html.contains("Retry valuation"));
    assert_eq!(failed_html.matches(">↻</tg-button>").count(), 1);
    assert!(failed_refresh.text.contains("unavailable"));
    assert!(failed_html.contains(">2</tg-button>"));
    assert!(
        failed_refresh.text.contains("#129"),
        "refresh failure retains report and page"
    );
    let retry_refresh = commands.report(callback(failed_html, "↻")).await;
    assert!(retry_refresh.text.contains("unavailable"));
    assert!(retry_refresh.text.contains("#129"));
    let missing = commands
        .report(callback(cached_back.rich_html.as_ref().unwrap(), "Model"))
        .await;
    let missing_html = missing.rich_html.as_ref().unwrap();
    assert!(missing_html.contains("Retry sales"));
    assert!(missing_html.contains("<b>Estimate</b>"));
    assert!(missing_html.contains("type=\"disabled\" style=\"primary\">Sales</tg-button>"));
    let retry_sales = commands.report(callback(missing_html, "Retry sales")).await;
    assert!(
        retry_sales
            .rich_html
            .as_ref()
            .unwrap()
            .contains("Retry sales")
    );
    let target = market_links::Target::Gift {
        slug: GiftSlug::new("Gift-123").unwrap(),
        currency_id: "GRAM".into(),
    };
    assert_eq!(
        target.share(None).unwrap(),
        market_links::ShareDestination::Inline {
            query: "Gift-123 GRAM".into()
        }
    );
    let target = market_links::Target::Market {
        selection: browse::parse("/market collection=Gift").unwrap(),
    };
    let market_links::ShareDestination::Inline { query } = target.share(None).unwrap() else {
        panic!()
    };
    assert!(
        parse::target(&query, 500, &[] as &[String], None).is_some(),
        "targets survive service shutdown"
    );
    reads.shutdown().await;
}

/// Explicit opt-in only: sends one real message to the recipient authorized for this smoke test.
#[tokio::test]
#[ignore = "sends a real Telegram message; requires explicit credentials and captured public valuation data"]
async fn send_live_rich_preview_to_authorized_recipient() {
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
    let expected_bot = std::env::var("MARKET_TEST_BOT_ID")
        .expect("explicit test bot ID")
        .parse::<u64>()
        .unwrap();
    let recipient = std::env::var("MARKET_TEST_RECIPIENT")
        .expect("explicit authorized recipient")
        .parse::<i64>()
        .unwrap();
    assert_eq!(
        credentials.bot_id(),
        expected_bot,
        "unexpected smoke-test bot"
    );
    let dir = tempfile::tempdir().unwrap();
    let scheduler = market_invocations::Runtime::start(Default::default()).unwrap();
    let session_directory = std::env::var_os("MARKET_TEST_SESSION_DIR")
        .map(std::path::PathBuf::from)
        .expect("reuse an authorized session directory for live previews");
    let mut connection = Connection::open(credentials, &session_directory, scheduler.handle())
        .await
        .unwrap();
    let media_directory = std::env::var_os("MARKET_TEST_MEDIA_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| dir.path().join("media"));
    let (cache, mut downloader) = market_media::MediaCache::open(media_directory)
        .await
        .unwrap();
    connection.transport.bind_artwork(cache.clone());
    let actions = connection.transport.actions();
    let user = actions.identity().await.unwrap();
    assert_eq!(user.id, expected_bot);
    eprintln!(
        "Authenticated bot: inline={}, guest={}, main_app={}",
        user.inline_enabled, user.guest_enabled, user.has_main_web_app
    );
    let media_actions = actions.clone();
    let downloads = tokio::spawn(async move {
        downloader.run(|key| media_actions.media(key)).await;
    });
    let value: market_valuation::ValuationReport = serde_json::from_slice(
        &std::fs::read(std::env::var("MARKET_TEST_VALUATION").unwrap()).unwrap(),
    )
    .unwrap();
    let mut floors = report::FloorQuotes::default();
    for path in [
        std::env::var("MARKET_TEST_FLOORS").ok(),
        std::env::var("MARKET_TEST_STARS_FLOORS").ok(),
    ]
    .into_iter()
    .flatten()
    {
        let captured: market_valuation::ValuationReport =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let values = captured
            .results
            .into_iter()
            .find_map(|result| match result {
                market_valuation::StrategyOutcome::Value(
                    market_valuation::StrategyValuation::CatalogFloor(value),
                ) => Some(value),
                _ => None,
            });
        match captured.currency_id.as_str() {
            "GRAM" => floors.gram = values,
            "XTR" => floors.stars = values,
            _ => panic!("preview floor currency is unsupported"),
        }
    }
    let slug = value.target.slug().unwrap().clone();
    // Optional public metadata captured by the read-only pack inspection test.
    // Preview mappings are explicit; never infer custom emoji validity from media IDs.
    let mut icons = crate::ReportIcons::default();
    if let Ok(path) = std::env::var("MARKET_TEST_REPORT_ICONS") {
        let captured: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        for (name, icon) in [
            ("model", crate::ReportIcon::Model),
            ("symbol", crate::ReportIcon::Symbol),
            ("gram", crate::ReportIcon::Gram),
        ] {
            if let Some(entry) = captured.get(name) {
                icons = icons
                    .with_custom(
                        icon,
                        TelegramDocumentId::new(
                            entry["document_id"].as_str().unwrap().parse().unwrap(),
                        )
                        .unwrap(),
                        entry["alternative"].as_str().unwrap(),
                    )
                    .unwrap();
            }
        }
    }
    let mut report = report::Report {
        icons: Arc::new(icons),
        query: ValueRequest {
            gift: slug,
            currency_id: value.currency_id.clone(),
        },
        value: Arc::new(value),
        floors,
        catalog: Some(
            serde_json::from_slice(
                &std::fs::read(std::env::var("MARKET_TEST_CATALOG").unwrap()).unwrap(),
            )
            .unwrap(),
        ),
        artwork: None,
    };
    if let Ok(id) = std::env::var("MARKET_TEST_DOCUMENT_ID") {
        let model = TelegramDocumentId::new(id.parse().unwrap()).unwrap();
        let pattern = std::env::var("MARKET_TEST_PATTERN_ID")
            .ok()
            .map(|id| TelegramDocumentId::new(id.parse().unwrap()).unwrap());
        report.set_artwork(model, pattern);
    }
    let report = Arc::new(report);
    let reply = if let Ok(path) = std::env::var("MARKET_TEST_HISTORY") {
        let captured: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let mut rows = Vec::new();
        for row in captured["data"].as_array().unwrap() {
            let book: GiftBook = serde_json::from_value(row["book"].clone()).unwrap();
            let target = &row["target"];
            assert_eq!(target["kind"], "gift");
            let gift = CollectibleGift {
                id: serde_json::from_value(target["id"].clone()).unwrap(),
                collection_id: book.collection_id.clone(),
                number: serde_json::from_value(target["number"].clone()).unwrap(),
                traits: serde_json::from_value(target["traits"].clone()).unwrap(),
            };
            let order = GiftOrder::try_from(GiftOrderData {
                id: serde_json::from_value(row["id"].clone()).unwrap(),
                book,
                kind: GiftOrderKind::Executed,
                target: GiftTarget::Gift { gift },
                price: serde_json::from_value(row["price"].clone()).unwrap(),
                quantity: serde_json::from_value(row["quantity"].clone()).unwrap(),
                observed_at: serde_json::from_value(row["observed_at"].clone()).unwrap(),
                executed_at: serde_json::from_value(row["executed_at"].clone()).unwrap(),
                expires_at: None,
                market_url: serde_json::from_value(row["market_url"].clone()).unwrap(),
            })
            .unwrap();
            rows.push(market_actors::OrderedEntry {
                key: market_actors::ReadPosition::decode(row["key"].as_str().unwrap()).unwrap(),
                order,
            });
        }
        let mut current = views::View::initial(report).unwrap();
        current.state.tab = views::Tab::Sales;
        let gallery = std::env::var("MARKET_TEST_GALLERY").is_ok_and(|v| v == "1");
        if gallery {
            current.state.mode = GridMode::Gallery;
        }
        let actions = reports::Reports::default();
        let result = history::captured_page(&current.report, &current.state.sales, rows).unwrap();
        let mut reply = views::render(&actions, &current, Some(&result), None);
        if gallery {
            let catalog = current.report.catalog.as_ref().unwrap();
            let catalogs = [(catalog.collection.id.clone(), catalog.clone())].into();
            let documents = current
                .report
                .gift()
                .and_then(|g| g.traits.model.clone())
                .zip(current.report.artwork)
                .map(|(id, cover)| (id, TelegramDocumentId::new(cover.model.get()).unwrap()))
                .into_iter()
                .collect();
            let mut patterns = std::collections::BTreeMap::new();
            if let (Some(cover), Some(id)) = (
                current.report.artwork,
                current
                    .report
                    .gift()
                    .and_then(|g| g.traits.pattern.as_ref()),
            ) {
                if let Some(document) = cover.pattern {
                    patterns.insert(id.clone(), document);
                }
            }
            reply.artwork = Some(gallery::order_cells(
                &result.data,
                &documents,
                &catalogs,
                &patterns,
                2,
            ));
        }
        reply.bounded()
    } else {
        let current = views::View::initial(report).unwrap();
        views::render(&reports::Reports::default(), &current, None, None).bounded()
    };
    if let Ok(path) = std::env::var("MARKET_TEST_REPLY_OUTPUT") {
        std::fs::write(
            path,
            serde_json::to_vec(&serde_json::json!({
                "text": reply.text,
                "title": reply.title,
                "summary": reply.summary,
                "rich_html": reply.rich_html,
                "emoji_documents": reply.emoji_documents,
                "artwork": reply.artwork.as_ref().and_then(|art| if let market_media::Artwork::Gift(cover) = art { Some(serde_json::json!({
                    "model": cover.model.get(),
                    "pattern": cover.pattern.map(market_media::DocumentId::get),
                    "palette": cover.palette.map(|palette| [palette.center, palette.edge, palette.pattern]),
                })) } else { None }),
            }))
            .unwrap(),
        )
        .unwrap();
    }
    // Captures retain production callbacks for inline protocol tests. One-shot
    // messages disable them because no worker retains these temporary views.
    let reply = static_preview(reply);
    if let Ok(path) = std::env::var("MARKET_TEST_COVER_OUTPUT") {
        let started = std::time::Instant::now();
        let artwork = reply.artwork.as_ref().expect("explicit artwork");
        let render = || async {
            match artwork {
                market_media::Artwork::Gift(cover) => cache.cover(*cover).await,
                market_media::Artwork::Gallery(gallery) => cache.gallery(gallery.clone()).await,
            }
        };
        let asset = render()
            .await
            .expect("high-resolution cover must succeed before this preview is sent");
        std::fs::write(path, asset.bytes()).unwrap();
        let cached = render().await.unwrap();
        assert_eq!(cached.etag(), asset.etag());
        let width = u32::from_be_bytes(asset.bytes()[16..20].try_into().unwrap());
        let height = u32::from_be_bytes(asset.bytes()[20..24].try_into().unwrap());
        eprintln!(
            "Prepared {}x{} artwork: {} bytes, {} ms, {} cover / {} gallery generation(s)",
            width,
            height,
            asset.bytes().len(),
            started.elapsed().as_millis(),
            cache.stats().cover_generations,
            cache.stats().gallery_generations,
        );
    }
    let event = Event::Message(Message {
        peer: PeerRef {
            id: PeerId::user(recipient).unwrap(),
            auth: PeerAuth::from_hash(0),
        },
        id: preview_message_id(),
        outgoing: false,
        text: String::new(),
    });
    let prepare_only = std::env::var("MARKET_TEST_PREPARE_ONLY").is_ok_and(|value| value == "1");
    let result = if prepare_only {
        Ok(())
    } else {
        actions.send(&event, Some(&reply), None).await
    };
    cache.shutdown().await;
    let _ = downloads.await;
    connection.close().await.unwrap();
    scheduler.shutdown().await;
    result.unwrap();
    eprintln!(
        "{} rich valuation preview",
        if prepare_only {
            "Prepared"
        } else {
            "Sent to the authorized recipient:"
        }
    );
}

pub(super) fn static_preview(mut reply: Reply) -> Reply {
    if let Some(html) = reply.rich_html.take() {
        let mut remaining = html.as_str();
        let mut output = String::new();
        while let Some(index) = remaining.find("<tg-button type=\"callback_data\"") {
            output.push_str(&remaining[..index]);
            let start = &remaining[index..];
            let end = start.find('>').expect("rendered button tag");
            output.push_str(if start[..end].contains("style=\"primary\"") {
                "<tg-button type=\"disabled\" style=\"primary\">"
            } else {
                "<tg-button type=\"disabled\">"
            });
            remaining = &start[end + 1..];
        }
        output.push_str(remaining);
        reply.rich_html = Some(output);
    }
    reply
}

/// Separate opt-in preview runs are new deliveries; retries within a run keep one ID.
pub(super) fn preview_message_id() -> i32 {
    use std::sync::{
        OnceLock,
        atomic::{AtomicI32, Ordering},
    };
    static NEXT: OnceLock<AtomicI32> = OnceLock::new();
    NEXT.get_or_init(|| {
        AtomicI32::new((jiff::Timestamp::now().as_millisecond() as i32 & i32::MAX).max(1))
    })
    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| {
        Some(if id == i32::MAX { 1 } else { id + 1 })
    })
    .unwrap()
}
#[test]
fn preview_runs_use_distinct_positive_delivery_identities() {
    let first = preview_message_id();
    let second = preview_message_id();
    assert!(first > 0 && second > 0);
    assert_ne!(first, second);
}
#[test]
fn one_shot_previews_preserve_tab_layout_without_expiring_callbacks() {
    let current = views::View::initial(view()).unwrap();
    let initial = views::render(&reports::Reports::default(), &current, None, None);
    let preview = static_preview(initial.clone());
    let html = preview.rich_html.unwrap();
    assert!(!html.contains("callback_data"));
    assert!(!html.contains("data=\"v:"));
    assert!(html.contains("style=\"primary\">Floors</tg-button>"));
    assert!(html.contains(">Sales</tg-button>"));
    assert!(html.contains(">↻</tg-button>"));
    assert_eq!(html.matches(">↻</tg-button>").count(), 1);
    let refresh = html.find(">↻</tg-button>").unwrap();
    let price = html.find("<h1>17.88 ").unwrap();
    let tabs = html.find(">Floors</tg-button>").unwrap();
    assert!(refresh < price && price < tabs);
    for removed in ["Ask floors", "Refresh estimate", "Valuation</tg-button>"] {
        assert!(!html.contains(removed), "{removed}");
    }
    assert!(html.contains("<th align=\"left\">Attribute</th>"));
    assert!(preview.text.contains("Estimate: 17.88 GRAM"));
    assert!(preview.text.contains("Floors · GRAM | Stars"));
    assert_eq!(preview.text, initial.text);
}

#[test]
fn attribute_table_keeps_native_quote_columns_names_and_missing_values_separate() {
    use market_valuation::{
        CatalogFloorValuation, CollectionFloors, FloorValuation, PairFloor, TraitFloors,
        ValuationMetric,
    };
    let quotes = |price: &str| {
        let floor = FloorValuation {
            price: ValuationMetric::Value {
                value: Amount::new(price).unwrap(),
            },
            count: 1,
        };
        Arc::new(CatalogFloorValuation {
            collections: vec![CollectionFloors {
                collection_id: "collection".into(),
                evaluated_at: jiff::Timestamp::now(),
                floor: floor.clone(),
                traits: Some(TraitFloors {
                    models: [("m".into(), floor.clone())].into(),
                    backdrops: [("b".into(), floor.clone())].into(),
                    patterns: [("p".into(), floor.clone())].into(),
                    pair: Some(PairFloor {
                        model_id: "m".into(),
                        backdrop_id: "b".into(),
                        floor,
                    }),
                }),
            }],
        })
    };
    let mut current = view();
    let report = Arc::get_mut(&mut current).unwrap();
    let catalog = report.catalog.as_mut().unwrap();
    catalog.backdrops = Arc::new(vec![GiftBackdrop {
        aliases: Default::default(),
        id: "b".into(),
        name: "Green <&>".into(),
        palette: None,
        rarity: None,
    }]);
    catalog.patterns = Arc::new(vec![GiftPattern {
        aliases: Default::default(),
        id: "p".into(),
        name: "Sun & Moon".into(),
        media: None,
        rarity: None,
    }]);
    report.floors = report::FloorQuotes {
        gram: Some(quotes("10.346")),
        stars: Some(quotes("1500.00")),
    };
    let reply = report.render();
    let html = reply.rich_html.unwrap();
    let header = html.split("<table").next().unwrap();
    assert!(!header.contains("&lt;b&gt;Model&lt;/b&gt;"));
    assert!(!header.contains("Green"));
    assert!(!header.contains("Sun &amp; Moon"));
    assert!(html.contains(&format!("<td>{} &lt;b&gt;Model&lt;/b&gt;</td><td align=\"right\">10.35</td><td align=\"right\">1 500</td>", report.icons.html(crate::ReportIcon::Model))));
    assert!(html.contains("Green &lt;&amp;&gt;"));
    assert!(html.contains("Sun &amp; Moon"));
    assert!(html.contains("Stars</th>"));
    assert!(reply.text.contains("10.35 GRAM · 1 500 Stars"));
    assert!(!html.contains(">GRAM</tg-button>"));
    assert!(!html.contains(">Stars</tg-button>"));
    report.floors.gram = None;
    let html = report.render().rich_html.unwrap();
    assert!(
        html.contains("<td align=\"right\">—</td><td align=\"right\">1 500</td>"),
        "a missing native quote is not converted or copied"
    );
    report.floors.stars = None;
    assert!(
        report
            .render()
            .rich_html
            .unwrap()
            .contains("Sun &amp; Moon"),
        "attributes remain visible when floor services are unavailable"
    );
}

#[test]
fn private_and_inline_links_dispatch_the_same_target_without_inventory_checks() {
    let usernames = ["market_bot".to_owned()];
    let origin = url::Url::parse("https://app.example/").unwrap();
    for text in [
        "Gift-123 XTR",
        "market f=c=@slug:Gift|b=@name:Black currency=GRAM mode=bid bid_match=all",
        "portfolio 123 XTR sort=number_desc",
        "market f=b=@name:Black currency=XTR",
    ] {
        let target = market_links::Target::parse_text(text, 7).unwrap();
        let expected = parse::command_target(target.clone()).unwrap();
        for url in [
            target.website_url(&origin).unwrap(),
            target.bot_url("market_bot", Some(&origin)).unwrap(),
        ] {
            let command = parse::target(url.as_str(), 7, &usernames, Some(&origin));
            assert_eq!(command, Some(expected.clone()), "{url}");
        }
        if let Some(payload) = target.launch(64).unwrap() {
            let input = format!("/start@market_bot {payload}");
            let request = parse::parse(
                market_telegram::IncomingMessage {
                    private: true,
                    outgoing: false,
                    user_id: Some(7),
                    message_id: 1,
                    text: Some(&input),
                },
                &usernames,
            )
            .unwrap();
            assert_eq!(request.command, expected);
            assert_eq!(request.user_id, 7);
        }
        if let Command::Portfolio(p) = expected {
            assert_eq!(p.mode, PortfolioMode::Inline);
            assert_ne!(p.action, PortfolioAction::Refresh);
        }
    }
    for text in [
        "https://other.example/#gift=Gift-123",
        "https://t.me/other_bot?start=t_Z2lmdA",
        "https://user:secret@t.me/market_bot?start=x",
    ] {
        assert!(
            parse::target(text, 7, &usernames, Some(&origin)).is_none(),
            "{text}"
        );
    }
    assert!(matches!(
        parse::parse(
            market_telegram::IncomingMessage {
                private: true,
                outgoing: false,
                user_id: Some(7),
                message_id: 1,
                text: Some("/start broken")
            },
            &usernames
        )
        .unwrap()
        .command,
        Command::Help
    ));
}

#[actix::test]
async fn landing_checks_are_explicit_and_bound_to_the_authenticated_sender() {
    let reply = LaunchCommands
        .handle(CommandRequest {
            command: Command::Start,
            user_id: 7,
            message_id: 1,
            inline: false,
        })
        .await
        .unwrap();
    let reports = reports::Reports::default();
    let reply = reports.reply(7, reply);
    let data = reply
        .buttons
        .iter()
        .flatten()
        .find_map(|b| match b {
            ReplyButton::Callback { data, .. } => Some(data.clone()),
            _ => None,
        })
        .unwrap();
    let target = CallbackTarget::Inline(InlineMessageId::Standard {
        dc_id: 2,
        id: 1,
        access_hash: 0,
    });
    let event = |user_id| Event::Callback {
        id: 1,
        user_id,
        data: data.clone(),
        target: target.clone(),
    };
    assert!(matches!(
        reports.parse(&event(8), &[]).unwrap().command,
        Command::CallbackDenied
    ));
    let Command::Portfolio(request) = reports.parse(&event(7), &[]).unwrap().command else {
        panic!()
    };
    assert_eq!(request.user_id.get(), 7);
    // Inline copies cannot trigger checks even if a private landing control is copied.
    assert_eq!(request.action, PortfolioAction::Navigate);
}

#[test]
fn guest_targets_reuse_cached_navigation_and_never_start_inventory_checks() {
    let event = Event::Guest {
        id: 1,
        user_id: 7,
        text: "@market_bot portfolio 123 XTR sort=number_desc".into(),
        references: vec![],
    };
    let request = interactions::parse_event(&event, &["market_bot".into()]).unwrap();
    assert!(request.inline);
    let Command::Portfolio(p) = request.command else {
        panic!()
    };
    assert_eq!(p.user_id.get(), 123);
    assert_eq!(p.mode, PortfolioMode::Inline);
    assert_eq!(p.action, PortfolioAction::Open);
}

#[test]
fn malformed_explicit_targets_never_fall_back_to_gram_or_reply_context() {
    let names = ["market_bot".to_owned()];
    for text in [
        "Gift-123 USD",
        "https://t.me/nft/Gift-123 USD",
        "market f=c=gift currency=USD Gift-123",
        "portfolio 123 USD",
    ] {
        assert!(
            parse::input(text, 7, &names, None, &["Gift-456".into()]).is_none(),
            "{text}"
        );
        assert!(
            interactions::parse_event(
                &Event::Inline {
                    id: 1,
                    user_id: 7,
                    query: text.into(),
                    offset: String::new()
                },
                &names
            )
            .is_none(),
            "{text}"
        );
    }
    assert!(parse::input("", 7, &names, None, &[]).is_none());
    assert!(matches!(
        parse::input("", 7, &names, None, &["Gift-123".into()]),
        Some(Command::Gift(_))
    ));
    assert!(matches!(
        parse::input("Please check Gift-123 in TON", 7, &names, None, &[]),
        Some(Command::Gift(_))
    ));
}
