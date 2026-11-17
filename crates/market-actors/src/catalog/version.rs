use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io::{self, Write};
use tonic::Status;
#[derive(Clone)]
pub struct CatalogSnapshot<T> {
    pub data: T,
    pub revision: String,
}
/// Hash serialization directly, avoiding a second retained copy of catalog JSON.
pub(super) fn revision(value: &impl Serialize) -> Result<String, Status> {
    struct HashWriter(Sha256);
    impl Write for HashWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.update(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut writer = HashWriter(Sha256::new());
    serde_json::to_writer(&mut writer, value).map_err(|e| Status::internal(e.to_string()))?;
    Ok(format!("{:x}", writer.0.finalize()))
}
