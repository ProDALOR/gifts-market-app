use market_links::MarketSelection;
use serde::Deserialize;
#[derive(Deserialize)]
struct Fixture {
    name: String,
    query: String,
    line: String,
}
#[test]
fn rust_and_gui_share_filter_grammar_and_roundtrip_fixtures() {
    let fixtures: Vec<Fixture> = serde_json::from_str(include_str!("codec.json")).unwrap();
    for f in fixtures {
        let s = MarketSelection::from_query(&f.query).unwrap_or_else(|e| panic!("{}: {e}", f.name));
        assert_eq!(s.filter_line(), f.line, "{}", f.name);
        let restored = MarketSelection::from_query(&s.to_query().unwrap()).unwrap();
        assert_eq!(restored, s, "{}", f.name);
    }
}
#[test]
fn malformed_combinations_are_rejected_atomically() {
    for f in [
        "m=@name:Model",
        "c=a|c=b",
        "x=a",
        "b=",
        "b=@name:A,",
        "b=@name:A|",
        "b=%GG",
        "b=%FF",
        "n=0",
        "n=..",
        "n=+1",
        "n=5..1",
        "c=a,b",
        "b=@slug:Has%20Space",
    ] {
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("f", f)
            .finish();
        assert!(MarketSelection::from_query(&query).is_err(), "{f}");
    }
    assert!(MarketSelection::from_query("f=&f=b%3Dx").is_err());
}
#[test]
fn large_selections_keep_every_filter_in_stateless_website_fragments() {
    let mut s = MarketSelection::from_query(
        "collection=@slug:Gift&currency=XTR&mode=bid&bid_match=all&min=1.20&max=100",
    )
    .unwrap();
    s.filters.models = (0..300)
        .map(|i| format!("@name:Model {i}").try_into().unwrap())
        .collect();
    s.filters.backdrops = vec!["@name:Black|White".into()];
    assert_eq!(
        market_links::Target::Market {
            selection: s.clone()
        }
        .launch(512)
        .unwrap(),
        None
    );
    let target = market_links::Target::Market {
        selection: s.clone(),
    };
    let url = target
        .website_url(&url::Url::parse("https://market.example/app?old=1#old").unwrap())
        .unwrap();
    assert!(url.query().is_none());
    let restored = MarketSelection::from_query(url.fragment().unwrap()).unwrap();
    assert_eq!(restored, s);
    assert!(
        target
            .website_url(&url::Url::parse("javascript:alert(1)").unwrap())
            .is_err()
    );
}
#[test]
fn navigation_does_not_change_rest_or_websocket_query_shapes() {
    let s = MarketSelection::from_query(
        "f=c%3D%40slug%3AGift%7Cm%3D%40name%3AModel&mode=bid&bid_match=all",
    )
    .unwrap();
    let json = serde_json::to_value(s.order_query()).unwrap();
    assert_eq!(json["source"], "bids");
    assert_eq!(json["bid_match"], "all");
    assert_eq!(
        json["filters"]["models"],
        serde_json::json!(["@name:Model"])
    );
    assert!(json.get("f").is_none());
}
