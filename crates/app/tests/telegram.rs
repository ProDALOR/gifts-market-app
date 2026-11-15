use actix_web::{App, test, web};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signer, SigningKey};
use market_app::{configure, security_headers, telegram::Telegram};
use market_telegram_app::auth::Auth;
use serde_json::json;
fn proof(key: &SigningKey, bot: u64, date: i64) -> String {
    let user = r#"{"id":9007199254740991,"first_name":"Anna","username":"anna"}"#;
    let check = format!("{bot}:WebAppData\nauth_date={date}\nuser={user}");
    url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs([
            ("auth_date", date.to_string()),
            ("user", user.into()),
            (
                "signature",
                URL_SAFE_NO_PAD.encode(key.sign(check.as_bytes()).to_bytes()),
            ),
        ])
        .finish()
}
#[actix_web::test]
async fn telegram_authentication_uses_signed_identity_and_exposes_only_public_configuration() {
    let key = SigningKey::from_bytes(&[17; 32]);
    let auth = Auth::new(42, key.verifying_key().to_bytes(), [91; 32]).unwrap();
    let state = Telegram::new(
        Some(auth),
        Some("test_market_bot".into()),
        Some("https://markets.example".into()),
    )
    .unwrap();
    let app = test::init_service(
        App::new()
            .wrap(security_headers())
            .app_data(web::Data::new(state))
            .configure(configure),
    )
    .await;
    let config =
        test::call_service(&app, test::TestRequest::get().uri("/telegram").to_request()).await;
    let csp = config
        .headers()
        .get("content-security-policy")
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    assert!(csp.contains("https://telegram.org/js/telegram-web-app.js"));
    assert!(csp.contains("https://web.telegram.org"));
    assert!(!csp.contains("frame-ancestors 'none'"));
    let config: serde_json::Value = test::read_body_json(config).await;
    assert_eq!(config["bot_id"], "42");
    assert_eq!(config["auth_enabled"], true);
    assert!(config.get("token").is_none());
    let raw = proof(&key, 42, jiff::Timestamp::now().as_second());
    let response = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/telegram")
            .set_json(json!({"init_data":raw}))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
    let session: serde_json::Value = test::read_body_json(response).await;
    assert_eq!(session["user"]["id"], "9007199254740991");
    assert_eq!(session["user"]["first_name"], "Anna");
    assert!(session["expires_at"].as_str().unwrap().contains('T'));
    let me = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/auth/me")
            .insert_header((
                "Authorization",
                format!("Bearer {}", session["token"].as_str().unwrap()),
            ))
            .to_request(),
    )
    .await;
    assert_eq!(me.status(), 200);
    let user: serde_json::Value = test::read_body_json(me).await;
    assert_eq!(user["user"]["id"], session["user"]["id"]);
    assert!(user.get("token").is_none());
    for data in [
        raw.replace("Anna", "Forged"),
        proof(&key, 99, jiff::Timestamp::now().as_second()),
        proof(&key, 42, jiff::Timestamp::now().as_second() - 3601),
    ] {
        let response = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/auth/telegram")
                .set_json(json!({"init_data":data}))
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), 401);
        let error: serde_json::Value = test::read_body_json(response).await;
        assert_eq!(error["error"]["code"], "unauthorized");
    }
    let me = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/auth/me")
            .insert_header(("Authorization", "Bearer invalid"))
            .to_request(),
    )
    .await;
    assert_eq!(me.status(), 401);
    let anonymous =
        test::call_service(&app, test::TestRequest::get().uri("/healthz").to_request()).await;
    assert_eq!(anonymous.status(), 200);
}
#[actix_web::test]
async fn standalone_browsing_has_no_telegram_dependency() {
    let app = test::init_service(App::new().configure(configure)).await;
    let response: serde_json::Value =
        test::call_and_read_body_json(&app, test::TestRequest::get().uri("/telegram").to_request())
            .await;
    assert_eq!(response["auth_enabled"], false);
    let response = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/auth/telegram")
            .set_json(json!({"init_data":"anything"}))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), 503);
}
