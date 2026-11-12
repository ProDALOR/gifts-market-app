use market_view as m;
pub use market_view::protocol::catalog::*;
use market_view::protocol::{MAX_MESSAGE_BYTES, wire as w};
use prost::Message;
use std::sync::Arc;
use tonic::{
    Request, Status,
    metadata::{Ascii, MetadataValue},
    transport::Channel,
};
pub async fn fetch(
    channel: Channel,
    authorization: &MetadataValue<Ascii>,
) -> Result<CatalogSnapshot, Status> {
    let mut client = w::market_catalog_client::MarketCatalogClient::new(channel)
        .max_decoding_message_size(MAX_MESSAGE_BYTES)
        .max_encoding_message_size(MAX_MESSAGE_BYTES);
    let mut generation = String::new();
    let mut expected_revision = String::new();
    let mut offset = 0;
    let mut total = None;
    let mut entries = Vec::new();
    let mut bytes = 0usize;
    loop {
        let mut request = Request::new(w::CatalogPageRequest {
            generation: generation.clone(),
            revision: expected_revision.clone(),
            offset,
        });
        request
            .metadata_mut()
            .insert("authorization", authorization.clone());
        let page = client.get_catalog_page(request).await?.into_inner();
        bytes = bytes.saturating_add(page.encoded_len());
        if bytes > MAX_CATALOG_BYTES
            || page.total_entries as usize > MAX_CATALOG_ENTRIES
            || page.entries.len() > CATALOG_PAGE_ENTRIES
        {
            return Err(Status::resource_exhausted("catalog exceeds client limits"));
        }
        if offset == 0 {
            if page.generation.is_empty() || page.revision.is_empty() || page.revision.len() > 256 {
                return Err(Status::data_loss("catalog identity missing"));
            }
            generation = page.generation.clone();
            expected_revision = page.revision.clone();
            total = Some(page.total_entries);
        }
        if page.generation != generation
            || page.revision != expected_revision
            || total != Some(page.total_entries)
        {
            return Err(Status::data_loss("catalog changed between pages"));
        }
        let count = page.entries.len();
        for entry in page.entries {
            entries.push(
                entry
                    .try_into()
                    .map_err(|_| Status::data_loss("invalid catalog entry"))?,
            );
        }
        if entries.len() > page.total_entries as usize {
            return Err(Status::data_loss("catalog entry count exceeded"));
        }
        match page.next_offset {
            Some(next)
                if count > 0 && next as usize == entries.len() && next < page.total_entries =>
            {
                offset = next
            }
            None if entries.len() == page.total_entries as usize => break,
            _ => return Err(Status::data_loss("invalid catalog continuation")),
        }
    }
    let catalog = Arc::new(
        m::Catalog::from_entries(entries)
            .map_err(|_| Status::data_loss("invalid catalog relationships"))?,
    );
    let snapshot = CatalogSnapshot::received(generation, expected_revision.clone(), catalog);
    Ok(snapshot)
}
