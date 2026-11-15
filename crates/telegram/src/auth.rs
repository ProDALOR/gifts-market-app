use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signature, VerifyingKey};
use hmac::{Hmac, Mac};
use jiff::Timestamp;
use percent_encoding::percent_decode_str;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::Write,
    path::Path,
};
type HmacSha256 = Hmac<Sha256>;
const MAX_INIT_BYTES: usize = 16 * 1024;
const AUTH_AGE: i64 = 3600;
const SESSION_AGE: i64 = 24 * 3600;
const TELEGRAM_KEY: [u8; 32] = [
    0xe7, 0xbf, 0x03, 0xa2, 0xfa, 0x46, 0x02, 0xaf, 0x45, 0x80, 0x70, 0x3d, 0x88, 0xdd, 0xa5, 0xbb,
    0x59, 0xf3, 0x2e, 0xd8, 0xb0, 0x2a, 0x56, 0xc1, 0x87, 0xfe, 0x7d, 0x34, 0xca, 0xed, 0x24, 0x2d,
];
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthError {
    Invalid,
    Expired,
}
impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Invalid => "Telegram sign-in is invalid",
            Self::Expired => "Sign-in expired. Reopen the app or sign in again",
        })
    }
}
impl std::error::Error for AuthError {}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct User {
    pub id: String,
    pub first_name: String,
    pub last_name: Option<String>,
    pub username: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct Session {
    pub token: String,
    #[cfg_attr(feature="schema",schema(value_type=String,format=DateTime))]
    pub expires_at: Timestamp,
    pub user: User,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
pub struct Identity {
    #[cfg_attr(feature="schema",schema(value_type=String,format=DateTime))]
    pub expires_at: Timestamp,
    pub user: User,
}
#[derive(Deserialize)]
struct TelegramUser {
    id: u64,
    first_name: String,
    last_name: Option<String>,
    username: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Claims {
    bot_id: u64,
    issued_at: Timestamp,
    expires_at: Timestamp,
    user: User,
}
#[derive(Clone)]
pub struct Auth {
    bot_id: u64,
    key: VerifyingKey,
    session_key: [u8; 32],
}
impl Auth {
    pub fn production(bot_id: u64, session_key: [u8; 32]) -> anyhow::Result<Self> {
        Self::new(bot_id, TELEGRAM_KEY, session_key)
    }
    /// Explicit verification key for embedding and deterministic protocol tests. The app always uses production().
    pub fn new(
        bot_id: u64,
        verification_key: [u8; 32],
        session_key: [u8; 32],
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(bot_id > 0, "Telegram bot ID must be positive");
        Ok(Self {
            bot_id,
            key: VerifyingKey::from_bytes(&verification_key)?,
            session_key,
        })
    }
    pub fn bot_id(&self) -> u64 {
        self.bot_id
    }
    pub fn sign_in(&self, raw: &str, now: Timestamp) -> Result<Session, AuthError> {
        let values = fields(raw)?;
        let signature = values.get("signature").ok_or(AuthError::Invalid)?;
        let bytes = URL_SAFE_NO_PAD
            .decode(signature.trim_end_matches('='))
            .map_err(|_| AuthError::Invalid)?;
        let signature = Signature::from_slice(&bytes).map_err(|_| AuthError::Invalid)?;
        let data = check_string(self.bot_id, &values);
        self.key
            .verify_strict(data.as_bytes(), &signature)
            .map_err(|_| AuthError::Invalid)?;
        let date = values
            .get("auth_date")
            .ok_or(AuthError::Invalid)?
            .parse::<i64>()
            .map_err(|_| AuthError::Invalid)?;
        if date > now.as_second() + 30 {
            return Err(AuthError::Invalid);
        }
        if date < now.as_second() - AUTH_AGE {
            return Err(AuthError::Expired);
        }
        let raw: TelegramUser = serde_json::from_str(values.get("user").ok_or(AuthError::Invalid)?)
            .map_err(|_| AuthError::Invalid)?;
        if raw.id == 0
            || raw.id > i64::MAX as u64
            || raw.first_name.is_empty()
            || raw.first_name.len() > 256
            || raw.last_name.as_ref().is_some_and(|v| v.len() > 256)
            || raw.username.as_ref().is_some_and(|v| v.len() > 64)
        {
            return Err(AuthError::Invalid);
        }
        let user = User {
            id: raw.id.to_string(),
            first_name: raw.first_name,
            last_name: raw.last_name,
            username: raw.username,
        };
        let expires_at = Timestamp::from_second(now.as_second() + SESSION_AGE)
            .map_err(|_| AuthError::Invalid)?;
        let claims = Claims {
            bot_id: self.bot_id,
            issued_at: now,
            expires_at,
            user: user.clone(),
        };
        let payload =
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).map_err(|_| AuthError::Invalid)?);
        let mut mac =
            HmacSha256::new_from_slice(&self.session_key).map_err(|_| AuthError::Invalid)?;
        mac.update(b"market-app-session\0");
        mac.update(payload.as_bytes());
        let signature = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
        Ok(Session {
            token: format!("{payload}.{signature}"),
            expires_at,
            user,
        })
    }
    pub fn identity(&self, token: &str, now: Timestamp) -> Result<Identity, AuthError> {
        if token.len() > 4096 {
            return Err(AuthError::Invalid);
        }
        let (payload, signature) = token.split_once('.').ok_or(AuthError::Invalid)?;
        let signature = URL_SAFE_NO_PAD
            .decode(signature)
            .map_err(|_| AuthError::Invalid)?;
        let mut mac =
            HmacSha256::new_from_slice(&self.session_key).map_err(|_| AuthError::Invalid)?;
        mac.update(b"market-app-session\0");
        mac.update(payload.as_bytes());
        mac.verify_slice(&signature)
            .map_err(|_| AuthError::Invalid)?;
        let raw = URL_SAFE_NO_PAD
            .decode(payload)
            .map_err(|_| AuthError::Invalid)?;
        let claims: Claims = serde_json::from_slice(&raw).map_err(|_| AuthError::Invalid)?;
        if claims.bot_id != self.bot_id
            || claims.issued_at.as_second() > now.as_second() + 30
            || claims.expires_at <= claims.issued_at
        {
            return Err(AuthError::Invalid);
        }
        if claims.expires_at <= now {
            return Err(AuthError::Expired);
        }
        Ok(Identity {
            expires_at: claims.expires_at,
            user: claims.user,
        })
    }
}
fn fields(raw: &str) -> Result<BTreeMap<String, String>, AuthError> {
    if raw.is_empty() || raw.len() > MAX_INIT_BYTES {
        return Err(AuthError::Invalid);
    }
    let decode = |s: &str| -> Result<String, AuthError> {
        let bytes = s.as_bytes();
        for (i, b) in bytes.iter().enumerate() {
            if *b == b'%'
                && (i + 2 >= bytes.len()
                    || !bytes[i + 1].is_ascii_hexdigit()
                    || !bytes[i + 2].is_ascii_hexdigit())
            {
                return Err(AuthError::Invalid);
            }
        }
        let value = s.replace('+', " ");
        percent_decode_str(&value)
            .decode_utf8()
            .map(|v| v.into_owned())
            .map_err(|_| AuthError::Invalid)
    };
    let mut values = BTreeMap::new();
    for pair in raw.split('&') {
        let (key, value) = pair.split_once('=').ok_or(AuthError::Invalid)?;
        let key = decode(key)?;
        let value = decode(value)?;
        if values.len() >= 32
            || key.is_empty()
            || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            || value.contains(['\n', '\r'])
            || values.insert(key, value).is_some()
        {
            return Err(AuthError::Invalid);
        }
    }
    Ok(values)
}
fn check_string(bot_id: u64, values: &BTreeMap<String, String>) -> String {
    let mut result = format!("{bot_id}:WebAppData");
    for (key, value) in values {
        if key != "hash" && key != "signature" {
            result.push('\n');
            result.push_str(key);
            result.push('=');
            result.push_str(value);
        }
    }
    result
}
pub fn generate_key() -> anyhow::Result<[u8; 32]> {
    let mut key = [0; 32];
    getrandom::fill(&mut key).map_err(|_| anyhow::anyhow!("Cannot generate session key"))?;
    Ok(key)
}
pub fn encode_key(key: &[u8; 32]) -> String {
    URL_SAFE_NO_PAD.encode(key)
}
pub fn session_key(path: &Path, configured: Option<&str>) -> anyhow::Result<[u8; 32]> {
    let decode = |text: &str| -> anyhow::Result<[u8; 32]> {
        URL_SAFE_NO_PAD
            .decode(text.trim())
            .ok()
            .and_then(|v| v.try_into().ok())
            .ok_or_else(|| anyhow::anyhow!("Session key must be a base64url-encoded 32-byte key"))
    };
    if let Some(configured) = configured {
        return decode(configured);
    }
    if path.exists() {
        return decode(&std::fs::read_to_string(path)?);
    }
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Invalid session key path"))?;
    std::fs::create_dir_all(parent)?;
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let key = generate_key()?;
    let result = (|| -> anyhow::Result<()> {
        let mut file = options.open(&temporary)?;
        file.write_all(encode_key(&key).as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)?;
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result?;
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    fn proof(key: &SigningKey, bot: u64, now: i64) -> String {
        let mut values: BTreeMap<_, _> = [
            ("auth_date".into(), now.to_string()),
            ("query_id".into(), "query".into()),
            (
                "user".into(),
                r#"{"id":9007199254740991,"first_name":"Anna + Катя","username":"anna"}"#.into(),
            ),
        ]
        .into();
        values.insert(
            "signature".into(),
            URL_SAFE_NO_PAD.encode(key.sign(check_string(bot, &values).as_bytes()).to_bytes()),
        );
        values.insert("hash".into(), "not-used-for-ed25519".into());
        url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(values)
            .finish()
    }
    #[test]
    fn signed_init_data_issues_expiring_bot_bound_sessions() {
        let signing = SigningKey::from_bytes(&[1; 32]);
        let now = Timestamp::from_second(1_800_000_000).unwrap();
        let auth = Auth::new(42, signing.verifying_key().to_bytes(), [7; 32]).unwrap();
        let raw = proof(&signing, 42, now.as_second());
        let session = auth.sign_in(&raw, now).unwrap();
        assert_eq!(session.user.first_name, "Anna + Катя");
        assert_eq!(session.user.id, "9007199254740991");
        assert_eq!(
            auth.identity(&session.token, now).unwrap().user,
            session.user
        );
        assert_eq!(
            auth.identity(&session.token, session.expires_at)
                .unwrap_err(),
            AuthError::Expired
        );
        assert!(
            Auth::new(43, signing.verifying_key().to_bytes(), [7; 32])
                .unwrap()
                .sign_in(&raw, now)
                .is_err()
        );
        assert!(
            Auth::new(43, signing.verifying_key().to_bytes(), [7; 32])
                .unwrap()
                .identity(&session.token, now)
                .is_err()
        );
        assert!(
            Auth::new(42, signing.verifying_key().to_bytes(), [8; 32])
                .unwrap()
                .identity(&session.token, now)
                .is_err()
        );
        let mut token = session.token.into_bytes();
        token[2] = if token[2] == b'a' { b'b' } else { b'a' };
        assert!(
            auth.identity(std::str::from_utf8(&token).unwrap(), now)
                .is_err()
        );
    }
    #[test]
    fn forged_duplicated_expired_and_future_launch_data_are_rejected() {
        let signing = SigningKey::from_bytes(&[2; 32]);
        let auth = Auth::new(42, signing.verifying_key().to_bytes(), [3; 32]).unwrap();
        let now = Timestamp::from_second(1_800_000_000).unwrap();
        let valid = proof(&signing, 42, now.as_second());
        assert!(auth.sign_in(&valid.replace("Anna", "Evil"), now).is_err());
        assert!(
            auth.sign_in(&(valid.clone() + "&user=another"), now)
                .is_err()
        );
        assert!(auth.sign_in(&(valid + "&bad=%FF"), now).is_err());
        assert_eq!(
            auth.sign_in(&proof(&signing, 42, now.as_second() - 3601), now)
                .unwrap_err(),
            AuthError::Expired
        );
        assert!(
            auth.sign_in(&proof(&signing, 42, now.as_second() + 31), now)
                .is_err()
        );
        for raw in [
            "user=%",
            "auth_date=1&signature=invalid",
            "user=%GG",
            "user=x&user=y",
        ] {
            assert!(auth.sign_in(raw, now).is_err());
        }
        assert!(
            auth.sign_in(
                &proof(&SigningKey::from_bytes(&[4; 32]), 42, now.as_second()),
                now
            )
            .is_err()
        );
    }
    #[test]
    fn session_key_survives_restart_and_configured_keys_are_validated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.key");
        let key = session_key(&path, None).unwrap();
        assert_eq!(session_key(&path, None).unwrap(), key);
        assert_eq!(
            session_key(&path, Some(&encode_key(&[9; 32]))).unwrap(),
            [9; 32]
        );
        assert!(session_key(&path, Some("weak")).is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}
