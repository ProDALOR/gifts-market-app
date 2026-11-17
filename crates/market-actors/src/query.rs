//! Compile actor-owned queries into private replica requests.
use super::{
    error::{DataError, Result},
    models::*,
};
use market_hub_protocol::wire as w;

fn limit(value: u32) -> Result<u32> {
    if !(1..=200).contains(&value) {
        Err(DataError::invalid("limit must be 1..200"))
    } else {
        Ok(value)
    }
}
impl BooksQuery {
    pub(crate) fn into_request(self) -> Result<w::BooksRequest> {
        Ok(w::BooksRequest {
            after: self.after.map(Into::into),
            limit: if self.limit == 0 {
                200
            } else {
                limit(self.limit)?
            },
        })
    }
}
impl CatalogQuery {
    pub(crate) fn into_read(self) -> Result<w::ReadRequest> {
        if self.ids.len() > 500 {
            return Err(DataError::invalid("at most 500 catalog IDs"));
        }
        let kind = w::EntityKind::from(self.kind) as i32;
        Ok(w::ReadRequest {
            selection: Some(w::Selection {
                kinds: vec![kind],
                keys: self
                    .ids
                    .into_iter()
                    .map(|id| w::Key {
                        kind,
                        id,
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            }),
            limit: limit(self.limit)?,
            page_token: self.page_token,
            ..Default::default()
        })
    }
}
impl OrdersQuery {
    pub(crate) fn into_read(mut self, mode: w::ReadMode) -> Result<w::ReadRequest> {
        if self
            .scope
            .item_type_id
            .as_ref()
            .is_none_or(String::is_empty)
            || self.scope.currency_id.as_ref().is_none_or(String::is_empty)
        {
            return Err(DataError::invalid("select an item type and quote currency"));
        }
        if self.scope.modes.is_empty() {
            self.scope.modes = vec![Mode::Ask];
        }
        if self.scope.modes.contains(&Mode::Executed) {
            return Err(DataError::invalid(
                "use the executions query for executed orders",
            ));
        }
        Ok(w::ReadRequest {
            selection: Some(w::Selection {
                source: Some(w::selection::Source::Scope(self.scope.try_into()?)),
                ..Default::default()
            }),
            limit: limit(self.limit)?,
            page_token: self.page_token,
            mode: mode as i32,
            sort: w::ReadSort::from(self.sort) as i32,
        })
    }
}
impl ExecutionsQuery {
    pub(crate) fn into_read(mut self) -> Result<w::ReadRequest> {
        self.scope.modes = vec![Mode::Executed];
        Ok(w::ReadRequest {
            selection: Some(w::Selection {
                source: Some(w::selection::Source::Scope(self.scope.try_into()?)),
                executed_at: self.period.map(|period| w::Period {
                    from_ms: period.from.map(|time| time.as_millisecond()),
                    until_ms: period.until.map(|time| time.as_millisecond()),
                }),
                ..Default::default()
            }),
            limit: limit(self.limit)?,
            page_token: self.page_token,
            sort: w::ReadSort::from(self.sort) as i32,
            ..Default::default()
        })
    }
}
