//! Shared enrollment and private credential storage for independently deployed nodes.
use market_view::protocol::wire as w;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::Path,
    time::Duration,
};
use tonic::{
    Code, Request, Status,
    metadata::{Ascii, MetadataValue},
    transport::Channel,
};
use uuid::Uuid;

pub fn authorization(token: &str) -> Result<MetadataValue<Ascii>, Status> {
    validate_token(token)?;
    MetadataValue::try_from(format!("Bearer {token}"))
        .map_err(|_| Status::invalid_argument("invalid credential encoding"))
}
pub fn valid_node_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}
pub fn validate_identity(id: &str, role: w::NodeRole) -> Result<(), Status> {
    if !valid_node_id(id) || role == w::NodeRole::Unspecified {
        Err(Status::invalid_argument("invalid node identity"))
    } else {
        Ok(())
    }
}
pub fn validate_token(token: &str) -> Result<(), Status> {
    if !(16..=512).contains(&token.len()) || !token.bytes().all(|b| (33..=126).contains(&b)) {
        Err(Status::invalid_argument("invalid credential"))
    } else {
        Ok(())
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeCredentials {
    pub id: String,
    pub role: w::NodeRole,
    pub token: String,
}
impl NodeCredentials {
    pub fn validate(&self) -> Result<(), Status> {
        validate_identity(&self.id, self.role)?;
        validate_token(&self.token)?;
        Ok(())
    }
    /// A complete credential is installed without overwriting a concurrent creator.
    pub fn load_or_create(
        path: &Path,
        id: Option<&str>,
        role: w::NodeRole,
    ) -> Result<Self, Status> {
        if role == w::NodeRole::Unspecified || id.is_some_and(|id| !valid_node_id(id)) {
            return Err(Status::invalid_argument("invalid node identity or role"));
        }
        match Self::load(path, id, role) {
            Ok(value) => return Ok(value),
            Err(error) if error.code() == Code::NotFound => {}
            Err(error) => return Err(error),
        }
        let parent = parent(path);
        fs::create_dir_all(parent).map_err(storage_error)?;
        let credentials = Self {
            id: id
                .map(str::to_owned)
                .unwrap_or_else(|| format!("node-{}", Uuid::new_v4().simple())),
            role,
            token: format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple()),
        };
        let bytes = serde_json::to_vec(&credentials).map_err(storage_error)?;
        let temporary = parent.join(format!(".node-{}", Uuid::new_v4()));
        let persist = || -> Result<Self, Status> {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temporary).map_err(storage_error)?;
            file.write_all(&bytes).map_err(storage_error)?;
            file.sync_all().map_err(storage_error)?;
            match fs::hard_link(&temporary, path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    return Self::load(path, id, role);
                }
                Err(error) => return Err(storage_error(error)),
            }
            sync_directory(parent)?;
            Ok(credentials)
        };
        let result = persist();
        let _ = fs::remove_file(temporary);
        result
    }
    fn load(path: &Path, id: Option<&str>, role: w::NodeRole) -> Result<Self, Status> {
        let metadata = fs::symlink_metadata(path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                Status::not_found("node credentials file not found")
            } else {
                storage_error(error)
            }
        })?;
        if !metadata.file_type().is_file() || metadata.len() > 4096 {
            return Err(Status::invalid_argument(
                "node credentials require a regular file of at most 4 KiB",
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o077 != 0 {
                return Err(Status::invalid_argument(
                    "node credentials require private permissions (0600)",
                ));
            }
        }
        let bytes = fs::read(path).map_err(storage_error)?;
        let value: Self = serde_json::from_slice(&bytes)
            .map_err(|_| Status::invalid_argument("invalid node credentials file"))?;
        value.validate()?;
        if id.is_some_and(|id| id != value.id) || value.role != role {
            return Err(Status::invalid_argument(
                "stored node identity or role differs from configuration",
            ));
        }
        sync_directory(parent(path))?;
        Ok(value)
    }
}
fn parent(path: &Path) -> &Path {
    path.parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}
fn sync_directory(path: &Path) -> Result<(), Status> {
    File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(storage_error)
}
fn storage_error(_: impl std::fmt::Display) -> Status {
    Status::internal("cannot read or persist private node credentials")
}

/// Confirm an existing binding first, so enrollment-token rotation does not
/// interrupt registered nodes. Only unknown credentials need enrollment authority.
pub async fn register(
    channel: Channel,
    identity: &NodeCredentials,
    registration_token: Option<&str>,
) -> Result<w::NodeRegistration, Status> {
    identity.validate()?;
    let mut client = w::market_registration_client::MarketRegistrationClient::new(channel)
        .max_decoding_message_size(4096)
        .max_encoding_message_size(4096);
    let request = |token: &str| -> Result<Request<w::RegisterNodeRequest>, Status> {
        let mut request = Request::new(w::RegisterNodeRequest {
            id: identity.id.clone(),
            token: identity.token.clone(),
            role: identity.role.into(),
        });
        request
            .metadata_mut()
            .insert("authorization", authorization(token)?);
        request.set_timeout(Duration::from_secs(5));
        Ok(request)
    };
    let mut response = client.register_node(request(&identity.token)?).await;
    if response
        .as_ref()
        .is_err_and(|error| error.code() == Code::Unauthenticated)
    {
        let token = registration_token.ok_or_else(|| {
            Status::unauthenticated(
                "node is not registered; supply a registration token for enrollment",
            )
        })?;
        response = client.register_node(request(token)?).await;
    }
    let response = response?.into_inner();
    if response.id != identity.id || response.role != i32::from(identity.role) {
        return Err(Status::data_loss(
            "registration returned an unexpected node identity or role",
        ));
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Folder(std::path::PathBuf);
    impl Folder {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("market-node-credentials-{}", Uuid::new_v4()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn credentials_keep_identity_and_role_and_reject_invalid_files_without_overwrite() {
        let folder = Folder::new();
        let path = folder.0.join("node.json");
        let original = NodeCredentials::load_or_create(&path, None, w::NodeRole::Reader).unwrap();
        let bytes = fs::read(&path).unwrap();
        let restored = NodeCredentials::load_or_create(&path, None, w::NodeRole::Reader).unwrap();
        assert_eq!(restored.id, original.id);
        assert_eq!(restored.token, original.token);
        assert!(
            NodeCredentials::load_or_create(&path, Some("different-node"), w::NodeRole::Reader)
                .is_err()
        );
        assert!(NodeCredentials::load_or_create(&path, None, w::NodeRole::Adapter).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        for corrupt in [
            "{broken",
            "{}",
            r#"{"id":"node","role":99,"token":"private-credential"}"#,
        ] {
            fs::write(&path, corrupt).unwrap();
            assert!(NodeCredentials::load_or_create(&path, None, w::NodeRole::Reader).is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), corrupt);
        }
        fs::write(&path, &bytes).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
            assert!(NodeCredentials::load_or_create(&path, None, w::NodeRole::Reader).is_err());
        }
    }
    #[test]
    fn concurrent_initialization_uses_one_credential() {
        let folder = Folder::new();
        let path = folder.0.join("private/node.json");
        let handles: Vec<_> = (0..12)
            .map(|_| {
                let path = path.clone();
                std::thread::spawn(move || {
                    NodeCredentials::load_or_create(&path, None, w::NodeRole::Reader).unwrap()
                })
            })
            .collect();
        let credentials: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();
        assert!(
            credentials
                .iter()
                .all(|value| value.id == credentials[0].id && value.token == credentials[0].token)
        );
    }
}
