use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use market_links::{ShareDestination, Target};
#[test]
fn targets_use_the_same_queries_launches_and_inline_text_as_the_gui() {
    let fixtures: serde_json::Value = serde_json::from_str(include_str!("targets.json")).unwrap();
    let origin = url::Url::parse("https://app.example/").unwrap();
    for f in fixtures.as_array().unwrap() {
        let query = f["query"].as_str().unwrap();
        let target = Target::from_query(query).unwrap();
        assert_eq!(target.query().unwrap(), query);
        assert_eq!(
            target.inline_query().unwrap(),
            f["inline"].as_str().unwrap()
        );
        assert_eq!(
            Target::parse_text(f["inline"].as_str().unwrap(), 7).unwrap(),
            target
        );
        if let Some(launch) = f["launch"].as_str() {
            assert_eq!(target.launch(512).unwrap().unwrap(), launch);
            assert_eq!(Target::from_launch(launch).unwrap(), target);
        }
        assert_eq!(
            Target::from_url(&target.website_url(&origin).unwrap()).unwrap(),
            target
        );
    }
}
#[test]
fn legacy_launches_still_decode_but_writers_use_one_format() {
    for query in [
        "collection=gift&gift=Gift-123&currency=XTR",
        "f=c%3Dgift&currency=GRAM",
    ] {
        let old = format!("v_{}", URL_SAFE_NO_PAD.encode(query));
        let target = Target::from_launch(&old).unwrap();
        assert!(matches!(target, Target::Market { .. }));
        assert!(target.launch(512).unwrap().unwrap().starts_with("t_"));
    }
    for old in [
        format!(
            "p_{}",
            URL_SAFE_NO_PAD.encode("user=7&currency=XTR&f=&sort=value_desc")
        ),
        "portfolio_7".into(),
    ] {
        assert!(matches!(
            Target::from_launch(&old).unwrap(),
            Target::Portfolio { .. }
        ));
    }
}
#[test]
fn complete_target_fallbacks_have_no_snapshot_or_expiry() {
    let mut selection = market_links::MarketSelection::from_query("collection=gift").unwrap();
    selection.filters.models = (0..300)
        .map(|i| format!("model-{i}").try_into().unwrap())
        .collect();
    let target = Target::Market { selection };
    let origin = url::Url::parse("https://app.example/").unwrap();
    assert!(target.launch(512).unwrap().is_none());
    let ShareDestination::Link { url } = target.share(Some(&origin)).unwrap() else {
        panic!()
    };
    assert_eq!(Target::from_url(&url).unwrap(), target);
    assert!(url.query().is_none());
    let bot = target.bot_url("gift_bot", Some(&origin)).unwrap();
    assert!(
        bot.query_pairs()
            .any(|(k, v)| k == "text" && Target::parse_text(&v, 1).ok().as_ref() == Some(&target))
    );
    assert!(target.bot_url("bad/name", Some(&origin)).is_err());
}
#[test]
fn navigation_targets_reject_versions_pagination_and_ambiguous_inputs() {
    for query in [
        "t=portfolio&user=7&version=abc",
        "t=portfolio&user=7&offset=30",
        "t=portfolio&user=7&limit=30",
        "t=portfolio&user=0",
        "t=portfolio&user=9007199254740992",
        "t=portfolio&user=7&user=8",
        "t=portfolio&user=7&f=m%3Dx",
        "t=portfolio&user=7&f=n%3D1..9",
        "t=unknown",
    ] {
        assert!(Target::from_query(query).is_err(), "{query}");
    }
}

#[test]
fn website_links_preserve_direct_overrides_and_read_old_launch_wrappers() {
    let launch =
        Target::from_query("t=portfolio&user=123&currency=GRAM&f=&sort=number_desc").unwrap();
    let mut url = url::Url::parse("https://app.example/").unwrap();
    url.query_pairs_mut()
        .append_pair("tgWebAppStartParam", &launch.launch(512).unwrap().unwrap())
        .append_pair("currency", "XTR");
    let Target::Portfolio { target } = Target::from_url(&url).unwrap() else {
        panic!()
    };
    assert_eq!(target.user_id.get(), 123);
    assert_eq!(target.currency_id.as_str(), "XTR");
    let url = url::Url::parse(
        "https://app.example/?collection=one&currency=XTR#f=c%3Dtwo%7Cb%3Dblack&currency=GRAM",
    )
    .unwrap();
    let Target::Market { selection } = Target::from_url(&url).unwrap() else {
        panic!()
    };
    assert_eq!(selection.filters.collection_id.unwrap().to_string(), "one");
    assert!(selection.filters.backdrops.is_empty());
    assert_eq!(selection.filters.currency_id.as_str(), "XTR");
    assert!(
        Target::from_url(&url::Url::parse("https://app.example/?f=&f=c%3Dgift").unwrap()).is_err()
    );
    assert!(Target::from_launch("portfolio_7&currency=XTR").is_err());
}

#[test]
fn shares_never_rely_on_telegram_truncating_a_large_draft() {
    let mut selection = market_links::MarketSelection::from_query("collection=gift").unwrap();
    selection.filters.models = (0..500)
        .map(|i| {
            format!("model-{i}-xxxxxxxxxxxxxxxxxxxx")
                .try_into()
                .unwrap()
        })
        .collect();
    let target = Target::Market { selection };
    let origin = url::Url::parse("https://app.example/").unwrap();
    assert!(target.share(Some(&origin)).is_err());
    let complete = target.website_url(&origin).unwrap();
    assert_eq!(Target::from_url(&complete).unwrap(), target);
}

#[test]
fn malformed_prefixes_and_share_snapshot_fields_cannot_change_the_subject() {
    for payload in [
        format!("t_{}", URL_SAFE_NO_PAD.encode("currency=GRAM")),
        format!("p_{}", URL_SAFE_NO_PAD.encode("gift=Gift-123")),
        format!("v_{}", URL_SAFE_NO_PAD.encode("t=portfolio&user=7")),
    ] {
        assert!(Target::from_launch(&payload).is_err(), "{payload}");
    }
    let target = serde_json::json!({"type":"portfolio","target":{"user_id":7,"currency_id":"GRAM","filters":{},"sort":"value_desc"}});
    assert!(serde_json::from_value::<Target>(target.clone()).is_ok());
    for (key, value) in [
        ("offset", serde_json::json!(30)),
        ("limit", serde_json::json!(30)),
        ("version", serde_json::json!("a".repeat(32))),
    ] {
        let mut invalid = target.clone();
        invalid["target"][key] = value;
        assert!(serde_json::from_value::<Target>(invalid).is_err());
    }
}

#[test]
fn copied_bot_links_reject_ambiguous_destinations_and_keep_draft_targets() {
    let target = Target::parse_text("portfolio 123 XTR sort=number_desc", 7).unwrap();
    let url = target.bot_url("gift_bot", None).unwrap();
    assert_eq!(Target::from_bot_url(&url, 7).unwrap(), target);
    for query in [
        "start=one&start=two",
        "startapp=one&text=portfolio+123",
        "text=portfolio+123&text=portfolio+456",
    ] {
        let url = url::Url::parse(&format!("https://t.me/gift_bot?{query}")).unwrap();
        assert!(Target::from_bot_url(&url, 7).is_err());
    }
}

#[test]
fn readable_and_legacy_query_text_apply_the_same_alias_validation() {
    for text in [
        "market f=c=gift|b=@tg:42 currency=GRAM",
        "market f=c%3Dgift%7Cb%3D%40tg%3A42&currency=GRAM",
        "portfolio user=7&f=c%3Dgift%7Cb%3D%40partner%3A42&currency=GRAM",
    ] {
        assert!(Target::parse_text(text, 7).is_err(), "{text}");
    }
}
