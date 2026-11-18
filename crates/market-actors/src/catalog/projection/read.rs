//! Catalog reads use the common bounded replica helpers.
pub(super) use market_replica::catalog_read::entities;
pub(crate) fn invalid(value: impl std::fmt::Display) -> tonic::Status {
    tonic::Status::data_loss(value.to_string())
}
