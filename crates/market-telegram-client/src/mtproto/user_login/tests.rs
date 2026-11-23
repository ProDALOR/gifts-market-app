use super::*;
fn export(byte: u8) -> Export {
    Export {
        api_id: 12345,
        auth_key: format!("{byte:02x}").repeat(256),
        dc_id: 2,
        server_address: "149.154.167.51:443".parse().unwrap(),
    }
}
#[tokio::test]
async fn exports_append_atomically_preserve_records_and_load_in_the_existing_worker() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.json");
    let mut first = serde_json::to_value(export(1)).unwrap();
    first["label"] = serde_json::json!("original");
    std::fs::write(&path, serde_json::to_vec(&vec![first.clone()]).unwrap()).unwrap();
    let destination = AccountExports::open(&path).unwrap();
    assert!(
        AccountExports::open(&path).is_err(),
        "concurrent writers must not lose records"
    );
    destination.append(export(2)).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let rows: Vec<serde_json::Value> = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0], first);
    let worker =
        super::super::accounts::AccountWorker::from_file(&path, dir.path().join("runtime"))
            .await
            .unwrap();
    let stop = CancellationToken::new();
    stop.cancel();
    worker.run(stop).await.unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let previous = std::fs::read(&path).unwrap();
    assert!(
        AccountExports::open(&path)
            .unwrap()
            .append(export(2))
            .is_err()
    );
    assert_eq!(
        std::fs::read(&path).unwrap(),
        previous,
        "invalid appends leave the export unchanged"
    );
    assert!(
        AccountExports::open(&path).is_ok(),
        "failures release the export lease"
    );
}
#[test]
fn exports_validate_before_login_and_never_reveal_invalid_input() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.json");
    let destination = AccountExports::open(&path).unwrap();
    assert!(!path.exists());
    drop(destination);
    assert!(
        !path.exists(),
        "cancelled login must not create an empty export"
    );
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        1,
        "temporary export is removed, stable lock remains"
    );
    for bytes in [
        b"private malformed secret".to_vec(),
        serde_json::to_vec(&serde_json::json!([{"auth_key":"private-key"}])).unwrap(),
        vec![b'x'; 65537],
    ] {
        std::fs::write(&path, &bytes).unwrap();
        let message = AccountExports::open(&path).err().unwrap().to_string();
        assert!(!message.contains("private"));
        assert!(!message.contains("private-key"));
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
    let full = (1..=16)
        .map(|n| serde_json::to_value(export(n)).unwrap())
        .collect::<Vec<_>>();
    std::fs::write(&path, serde_json::to_vec(&full).unwrap()).unwrap();
    assert!(
        AccountExports::open(&path)
            .err()
            .unwrap()
            .to_string()
            .contains("16")
    );
    std::fs::write(&path, b"[]").unwrap();
    AccountExports::open(&path)
        .unwrap()
        .append(export(3))
        .unwrap();
    assert!(validate_exports(&std::fs::read(&path).unwrap()).is_ok());
}
fn password_token() -> PasswordToken {
    PasswordToken::new(grammers_client::tl::types::account::Password {
        has_recovery: false,
        has_secure_values: false,
        has_password: false,
        current_algo: None,
        srp_b: None,
        srp_id: None,
        hint: None,
        email_unconfirmed_pattern: None,
        new_algo: grammers_client::tl::enums::PasswordKdfAlgo::Unknown,
        new_secure_algo: grammers_client::tl::enums::SecurePasswordKdfAlgo::Unknown,
        secure_random: vec![],
        pending_reset_date: None,
        login_email_pattern: None,
    })
}
#[tokio::test]
async fn login_state_validation_and_export_need_no_network_or_new_authorization() {
    let scheduler = market_invocations::Runtime::start(Default::default()).unwrap();
    assert!(UserLogin::start(0, "a".repeat(32), scheduler.handle()).is_err());
    assert!(
        UserLogin::start(12345, "private-api-hash".into(), scheduler.handle())
            .err()
            .unwrap()
            .to_string()
            .contains("32 hexadecimal")
    );
    let mut login = UserLogin::start(12345, "a".repeat(32), scheduler.handle()).unwrap();
    assert!(login.export().is_err());
    assert!(login.submit_code("12345".into()).await.is_err());
    assert!(login.submit_password("secret".into()).await.is_err());
    for phone in ["private-phone", "+abcd", "+123", "+1234567890123456"] {
        assert!(login.request_code(phone.into()).await.is_err());
    }
    assert_eq!(
        login.apply(Err(SignInError::InvalidCode)).unwrap(),
        LoginStep::InvalidCode
    );
    assert_eq!(
        login
            .apply(Err(SignInError::PasswordRequired(password_token())))
            .unwrap(),
        LoginStep::PasswordRequired
    );
    assert!(login.password.is_some());
    assert_eq!(
        login
            .apply(Err(SignInError::InvalidPassword(password_token())))
            .unwrap(),
        LoginStep::InvalidPassword
    );
    assert!(
        login.password.is_some(),
        "password retries retain the new SRP challenge"
    );
    assert!(login.apply(Err(SignInError::SignUpRequired)).is_err());
    let mut dc = login.session.dc_option(2).unwrap().unwrap();
    dc.auth_key = Some([7; 256]);
    login.session.set_dc_option(&dc).await.unwrap();
    login.authorized = true;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("new.json");
    AccountExports::open(&path).unwrap().save(&login).unwrap();
    validate_exports(&std::fs::read(path).unwrap()).unwrap();
    login.close().await;
    scheduler.shutdown().await;
}
