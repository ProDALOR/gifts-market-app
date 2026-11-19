use futures_util::StreamExt;
use market_api::{ApiConfig, ApiServer};
use market_app::MarketRuntime;
use std::{path::Path, time::Duration};
use tokio_tungstenite::tungstenite::Message;

fn server(runtime: &MarketRuntime, web_dir: &Path) -> ApiServer {
    ApiServer::bind(
        ApiConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            workers: 2,
            web_dir: Some(web_dir.to_owned()),
            cors_origins: vec![],
            limits: Default::default(),
        },
        runtime.handle(),
        runtime.valuation(),
    )
    .unwrap()
}

#[tokio::test]
async fn application_starts_api_once_and_shutdown_closes_connections_and_releases_listener() {
    let directory = tempfile::tempdir().unwrap();
    let web_dir = directory.path().join("web");
    std::fs::create_dir_all(&web_dir).unwrap();
    std::fs::write(web_dir.join("index.html"), "<html></html>").unwrap();
    let runtime = MarketRuntime::open(directory.path(), "test", 2, 2)
        .await
        .unwrap();
    let (cache, downloads) = market_media::MediaCache::open(directory.path().join("media"))
        .await
        .unwrap();
    drop(downloads);
    let api = server(&runtime, &web_dir).with_media(cache.clone());
    let address = api.local_addr().unwrap();
    runtime.start_api(api).unwrap();
    assert!(runtime.start_api(server(&runtime, &web_dir)).is_err());

    let (mut socket, _) = tokio::time::timeout(
        Duration::from_secs(3),
        tokio_tungstenite::connect_async(format!("ws://{address}/ws")),
    )
    .await
    .unwrap()
    .unwrap();
    tokio::time::timeout(Duration::from_secs(3), runtime.shutdown())
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            match socket.next().await {
                Some(Ok(Message::Close(_))) => break,
                Some(Ok(_)) => continue,
                other => panic!("API did not gracefully close its WebSocket: {other:?}"),
            }
        }
    })
    .await
    .unwrap();
    assert!(tokio::net::TcpStream::connect(address).await.is_err());
    assert_eq!(
        cache
            .cached(market_media::MediaKey::thumbnail(
                market_media::DocumentId::new(42).unwrap()
            ))
            .await
            .unwrap_err(),
        market_media::MediaError::Unavailable
    );
    assert!(runtime.start_api(server(&runtime, &web_dir)).is_err());
}

#[cfg(not(any(feature = "bot", feature = "miniapp")))]
#[tokio::test]
async fn api_only_process_ignores_telegram_environment_and_creates_no_auth_storage() {
    use std::process::Stdio;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let directory = tempfile::tempdir().unwrap();
    let data_dir = directory.path().join("data");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_market-app"))
        .args([
            "--bind",
            &address.to_string(),
            "--hub-url",
            "http://127.0.0.1:1",
            "--hub-token",
            "public-app.private-test-service-token",
        ])
        .arg("--no-web")
        .arg("--data-dir")
        .arg(&data_dir)
        .envs([
            ("MARKET_CORS_ORIGINS", ""),
            ("MARKET_HTTP_WORKERS", "1"),
            ("MARKET_MAX_CONCURRENT_QUERIES", "3"),
            ("MARKET_MAX_LIVE_QUERIES", "4"),
            ("MARKET_MINIAPP_BOT_ID", "not-an-id"),
            ("MARKET_MINIAPP_BOT_USERNAME", "invalid username"),
            ("MARKET_PUBLIC_URL", "not-a-url"),
            ("TELEGRAM_API_ID", "not-an-id"),
            ("TELEGRAM_API_HASH", "unused"),
            ("TELEGRAM_BOT_TOKEN", "unused"),
            ("MARKET_SESSION_KEY", "unused"),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let response = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            assert!(
                child.try_wait().unwrap().is_none(),
                "API-only process exited during startup"
            );
            if let Ok(mut stream) = tokio::net::TcpStream::connect(address).await {
                stream
                    .write_all(
                        b"GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
                    )
                    .await
                    .unwrap();
                let mut response = String::new();
                stream.read_to_string(&mut response).await.unwrap();
                break response;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    child.kill().await.unwrap();
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(!data_dir.join("session.key").exists());
    assert!(!data_dir.join("bot").exists());
}

#[tokio::test]
async fn cancelled_server_future_closes_media_admission_and_releases_listener() {
    let directory = tempfile::tempdir().unwrap();
    let web_dir = directory.path().join("web");
    std::fs::create_dir_all(&web_dir).unwrap();
    std::fs::write(web_dir.join("index.html"), "<html></html>").unwrap();
    let runtime = MarketRuntime::open(directory.path(), "test", 2, 2)
        .await
        .unwrap();
    let (cache, downloads) = market_media::MediaCache::open(directory.path().join("media"))
        .await
        .unwrap();
    drop(downloads);
    let api = server(&runtime, &web_dir).with_media(cache.clone());
    let address = api.local_addr().unwrap();
    let job = tokio::spawn(api.run(tokio_util::sync::CancellationToken::new()));
    let (mut socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}/ws"))
        .await
        .unwrap();
    job.abort();
    let _ = job.await;
    let key = market_media::MediaKey::thumbnail(market_media::DocumentId::new(42).unwrap());
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if cache.cached(key).await.is_err()
                && tokio::net::TcpStream::connect(address).await.is_err()
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let _ = socket.close(None).await;
    runtime.shutdown().await.unwrap();
}

#[cfg(feature = "bot")]
#[tokio::test]
async fn credential_free_process_starts_api_without_telegram_storage() {
    use std::process::Stdio;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    for blank in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let data_dir = directory.path().join("data");
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_market-app"));
        command
            .env_clear()
            .args([
                "--bind",
                &address.to_string(),
                "--hub-url",
                "http://127.0.0.1:1",
                "--hub-token",
                "public-app.private-test-service-token",
                "--no-web",
                "--http-workers",
                "1",
            ])
            .arg("--data-dir")
            .arg(&data_dir);
        if blank {
            for name in ["TELEGRAM_BOT_TOKEN", "TELEGRAM_API_ID", "TELEGRAM_API_HASH"] {
                command.env(name, "");
            }
        }
        let mut child = command
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let response = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                assert!(
                    child.try_wait().unwrap().is_none(),
                    "Credential-free process exited during startup"
                );
                if let Ok(mut stream) = tokio::net::TcpStream::connect(address).await {
                    stream
                        .write_all(
                            b"GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
                        )
                        .await
                        .unwrap();
                    let mut response = String::new();
                    stream.read_to_string(&mut response).await.unwrap();
                    break response;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        child.kill().await.unwrap();
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert!(!data_dir.join("bot").exists());
        assert!(!data_dir.join("session.key").exists());
    }
}

#[cfg(feature = "bot")]
#[tokio::test]
async fn partial_credentials_fail_before_creating_runtime_storage() {
    let directory = tempfile::tempdir().unwrap();
    let data_dir = directory.path().join("data");
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_market-app"))
        .env_clear()
        .env("TELEGRAM_BOT_TOKEN", "42:private_test_secret")
        .arg("--data-dir")
        .arg(&data_dir)
        .output()
        .await
        .unwrap();
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(!output.status.success());
    assert!(error.contains("TELEGRAM_API_ID"));
    assert!(error.contains("TELEGRAM_API_HASH"));
    assert!(!error.contains("private_test_secret"));
    assert!(!data_dir.exists());
}
