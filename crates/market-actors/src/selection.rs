//! Keep requests within the actor responsible for their entity projection.
use market_hub_protocol::wire as w;
use tonic::Status;
pub(crate) fn orders(request: &w::ReadRequest) -> Result<(), Status> {
    let selection = request
        .selection
        .as_ref()
        .ok_or_else(|| Status::invalid_argument("select an orderbook scope or order identities"))?;
    if selection.source.is_some() || selection.executed_at.is_some() {
        return Ok(());
    }
    if (!selection.kinds.is_empty() || !selection.keys.is_empty())
        && selection
            .kinds
            .iter()
            .all(|kind| *kind == w::EntityKind::Order as i32)
        && selection
            .keys
            .iter()
            .all(|key| key.kind == w::EntityKind::Order as i32)
    {
        Ok(())
    } else {
        Err(Status::invalid_argument(
            "orderbook queries must select only orders",
        ))
    }
}
#[cfg(test)]
pub(crate) fn catalog(request: &w::ReadRequest) -> Result<(), Status> {
    let selection = request
        .selection
        .as_ref()
        .ok_or_else(|| Status::invalid_argument("select a catalog kind or catalog identities"))?;
    let is_catalog = |kind: i32| {
        matches!(
            w::EntityKind::try_from(kind),
            Ok(w::EntityKind::Venue
                | w::EntityKind::Currency
                | w::EntityKind::ItemType
                | w::EntityKind::Attribute
                | w::EntityKind::AttributeOption
                | w::EntityKind::Feed)
        )
    };
    if selection.source.is_none()
        && selection.executed_at.is_none()
        && (!selection.kinds.is_empty() || !selection.keys.is_empty())
        && selection.kinds.iter().all(|kind| is_catalog(*kind))
        && selection.keys.iter().all(|key| is_catalog(key.kind))
    {
        Ok(())
    } else {
        Err(Status::invalid_argument(
            "catalog queries must select only catalog entities",
        ))
    }
}
