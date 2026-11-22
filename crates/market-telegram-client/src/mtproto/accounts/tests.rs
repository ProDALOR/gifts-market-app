use super::*;

fn export() -> Export {
    Export {
        api_id: 12345,
        auth_key: "ab".repeat(256),
        dc_id: 2,
        server_address: "127.0.0.1:443".parse().unwrap(),
    }
}
fn gift() -> tl::enums::StarGift {
    let tl::enums::SavedStarGift::Gift(saved) = crate::mtproto::gifts::tests::saved_gift(123, 456);
    saved.gift
}
#[test]
fn rejects_invalid_exports_without_echoing_secrets() {
    for field in ["api", "dc", "port", "zero", "hex", "short", "unicode"] {
        let mut value = export();
        match field {
            "api" => value.api_id = 0,
            "dc" => value.dc_id = 6,
            "port" => value.server_address.set_port(0),
            "zero" => value.auth_key = "00".repeat(256),
            "hex" => value.auth_key = "secret".repeat(86),
            "short" => value.auth_key.pop().map(|_| ()).unwrap(),
            "unicode" => value.auth_key = "é".repeat(256),
            _ => unreachable!(),
        }
        let secret = value.auth_key.clone();
        let Err(error) = Account::try_from(value) else {
            panic!("accepted {field}")
        };
        assert!(!error.to_string().contains(&secret));
    }
    let account = Account::try_from(export()).ok().unwrap();
    assert_eq!(account.auth_key, [0xab; 256]);
}
#[tokio::test]
async fn bounded_export_import_is_read_only_and_rejects_duplicates() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.json");
    let value = serde_json::json!({"api_id":12345,"auth_key":"ab".repeat(256),"dc_id":2,"server_address":"127.0.0.1:443"});
    let bytes = serde_json::to_vec(&vec![value.clone()]).unwrap();
    tokio::fs::write(&path, &bytes).await.unwrap();
    let worker = AccountWorker::from_file(&path, dir.path().join("locks"))
        .await
        .unwrap();
    assert_eq!(tokio::fs::read(&path).await.unwrap(), bytes);
    assert_eq!(
        worker
            .resolver()
            .resolve(&GiftSlug::new("Gift-1").unwrap())
            .await,
        Err(Error::Unavailable)
    );
    tokio::fs::write(
        &path,
        serde_json::to_vec(&vec![value.clone(), value]).unwrap(),
    )
    .await
    .unwrap();
    assert!(matches!(
        AccountWorker::from_file(&path, dir.path().into()).await,
        Err(BotError::Configuration(
            "Duplicate Telegram account session"
        ))
    ));
    tokio::fs::write(&path, vec![b' '; 65_537]).await.unwrap();
    assert!(matches!(
        AccountWorker::from_file(&path, dir.path().into()).await,
        Err(BotError::Configuration(
            "Telegram accounts file exceeds 64 KiB"
        ))
    ));
    tokio::fs::write(&path, b"sensitive invalid content")
        .await
        .unwrap();
    let Err(error) = AccountWorker::from_file(&path, dir.path().into()).await else {
        panic!()
    };
    assert!(!error.to_string().contains("sensitive"));
}
#[test]
fn maps_only_matching_unique_gifts_and_rejects_conflicting_traits() {
    let slug = GiftSlug::new("gift-1").unwrap();
    let facts = map_gift(gift(), &slug).unwrap();
    assert_eq!(facts.collection.get(), 123);
    assert_eq!(facts.model.get(), 456);
    assert_eq!(facts.number.get(), 1);
    assert_eq!(facts.slug, slug);
    for case in [
        "burned",
        "slug",
        "number",
        "model",
        "duplicate",
        "collection",
    ] {
        let tl::enums::StarGift::Unique(mut value) = gift() else {
            panic!()
        };
        match case {
            "burned" => value.burned = true,
            "slug" => value.slug = "Other-1".into(),
            "number" => value.num = 2,
            "model" => value.attributes.clear(),
            "duplicate" => value.attributes.push(value.attributes[0].clone()),
            "collection" => value.gift_id = 0,
            _ => unreachable!(),
        }
        assert_eq!(
            map_gift(tl::enums::StarGift::Unique(value), &slug),
            Err(if case == "burned" {
                Error::NotFound
            } else {
                Error::InvalidData
            })
        );
    }
}

// Explicit, read-only smoke test; never runs in ordinary test suites.
#[tokio::test]
#[ignore = "requires an authorized accounts export and Telegram network access"]
async fn existing_session_resolves_public_slug_and_shuts_down() {
    let path = std::env::var_os("MARKET_TELEGRAM_ACCOUNTS").expect("set MARKET_TELEGRAM_ACCOUNTS");
    let dir = tempfile::tempdir().unwrap();
    let worker = AccountWorker::from_file(Path::new(&path), dir.path().into())
        .await
        .unwrap();
    let resolver = worker.resolver();
    let stop = CancellationToken::new();
    let task = tokio::spawn(worker.run(stop.clone()));
    let result = tokio::time::timeout(Duration::from_secs(45), async {
        loop {
            if resolver.0.live.read().unwrap().iter().any(Option::is_some) {
                break resolver
                    .resolve(&GiftSlug::new("ResistanceDog-14029").unwrap())
                    .await;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;
    stop.cancel();
    tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let details = result
        .expect("user session authentication timed out")
        .expect("public gift lookup failed");
    assert_eq!(details.slug, GiftSlug::new("ResistanceDog-14029").unwrap());
    assert_eq!(details.number.get(), 14029);
    assert!(resolver.0.live.read().unwrap().iter().all(Option::is_none));
}

fn resolver() -> AccountGiftResolver {
    AccountGiftResolver(Arc::new(Shared {
        live: RwLock::new(vec![None]),
        next: AtomicUsize::new(0),
        cache: Mutex::new(VecDeque::new()),
    }))
}
#[tokio::test(start_paused = true)]
async fn shared_slug_cache_waits_coalesce_and_expiry_does_not_extend_on_access() {
    let resolver = resolver();
    let slug = GiftSlug::new("Gift-1").unwrap();
    let slot: CacheSlot = Arc::new(tokio::sync::Mutex::new(None));
    resolver
        .0
        .cache
        .lock()
        .unwrap()
        .push_back((slug.clone(), slot.clone()));
    let mut preparing = slot.lock().await;
    let mut jobs = vec![];
    for _ in 0..8 {
        let resolver = resolver.clone();
        jobs.push(tokio::spawn(async move {
            resolver.resolve(&GiftSlug::new("gift-1").unwrap()).await
        }));
    }
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
    assert_eq!(resolver.0.cache.lock().unwrap().len(), 1);
    *preparing = Some((
        Instant::now() + Duration::from_secs(300),
        map_gift(gift(), &slug),
    ));
    drop(preparing);
    for job in jobs {
        assert!(job.await.unwrap().is_ok());
    }
    tokio::time::advance(Duration::from_secs(299)).await;
    assert!(resolver.resolve(&slug).await.is_ok());
    tokio::time::advance(Duration::from_secs(1)).await;
    assert_eq!(resolver.resolve(&slug).await, Err(Error::Unavailable));
    *slot.lock().await = Some((
        Instant::now() + Duration::from_secs(15),
        Err(Error::NotFound),
    ));
    assert_eq!(resolver.resolve(&slug).await, Err(Error::NotFound));
    tokio::time::advance(Duration::from_secs(15)).await;
    assert_eq!(resolver.resolve(&slug).await, Err(Error::Unavailable));
}
#[tokio::test]
async fn cache_is_bounded_and_cancelled_worker_opens_no_session_files() {
    let resolver = resolver();
    for n in 1..=200 {
        assert_eq!(
            resolver
                .resolve(&GiftSlug::new(format!("Gift-{n}")).unwrap())
                .await,
            Err(Error::Unavailable)
        );
    }
    assert_eq!(resolver.0.cache.lock().unwrap().len(), 128);
    let dir = tempfile::tempdir().unwrap();
    let directory = dir.path().join("sessions");
    let worker = AccountWorker {
        accounts: vec![Account::try_from(export()).ok().unwrap()],
        directory: directory.clone(),
        resolver,
    };
    let stop = CancellationToken::new();
    stop.cancel();
    worker.run(stop).await.unwrap();
    assert!(!directory.exists());
}
#[test]
fn slug_lookups_share_the_inventory_group_rate_without_the_profile_page_gate() {
    let lookup = crate::mtproto::invocations::policy(42, Group::GiftLookup, Replay::Safe);
    let inventory = crate::mtproto::invocations::policy(42, Group::Gifts(7), Replay::Safe);
    assert_eq!(lookup.limits.len(), 2);
    for (a, b) in lookup.limits.iter().zip(&inventory.limits) {
        assert_eq!(a.key, b.key);
        assert_eq!(a.interval, b.interval);
        assert_eq!(a.concurrency, b.concurrency);
    }
}
