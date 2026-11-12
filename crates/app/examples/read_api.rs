//! A downstream service uses the public API credential, never core enrollment.
use clap::Parser;
use market_app::wire as a;
use market_view::protocol::wire as m;
use std::{path::PathBuf, time::Duration};
use tonic::{
    Request,
    transport::{Certificate, Channel, ClientTlsConfig},
};
#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "http://127.0.0.1:50053")]
    endpoint: String,
    #[arg(long)]
    ca: Option<PathBuf>,
    #[arg(long)]
    item_type: Option<String>,
    #[arg(long, default_value = "TON")]
    currency: String,
}
fn request<T>(value: T) -> anyhow::Result<Request<T>> {
    let mut request = Request::new(value);
    if let Ok(token) = std::env::var("MARKET_API_TOKEN") {
        request
            .metadata_mut()
            .insert("authorization", format!("Bearer {token}").parse()?);
    }
    Ok(request)
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let mut endpoint = Channel::from_shared(args.endpoint)?
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10));
    if let Some(ca) = args.ca {
        endpoint = endpoint.tls_config(
            ClientTlsConfig::new()
                .ca_certificate(Certificate::from_pem(tokio::fs::read(ca).await?)),
        )?;
    }
    let mut client = a::market_api_client::MarketApiClient::new(endpoint.connect().await?);
    let status = client.get_status(request(m::Empty {})?).await?.into_inner();
    anyhow::ensure!(status.ready, "API replica not ready");
    if let Some(kind) = args.item_type.clone() {
        client
            .get_order_book(request(a::BookQuery {
                item_type_id: kind,
                currency_id: args.currency,
                ..Default::default()
            })?)
            .await?;
    }
    let mut stream = client
        .snapshot(request(a::SnapshotQuery {
            item_type_id: args.item_type.clone(),
            ..Default::default()
        })?)
        .await?
        .into_inner();
    let mut cursor = None;
    let mut records = 0;
    while let Some(frame) = stream.message().await? {
        records += frame.records.len();
        if frame.complete {
            cursor = frame.meta.and_then(|meta| meta.cursor);
        }
    }
    let cursor = cursor.ok_or_else(|| anyhow::anyhow!("snapshot did not complete"))?;
    let mut updates = client
        .watch_changes(request(a::ApiWatchQuery {
            after: Some(cursor),
            item_type_id: args.item_type,
            ..Default::default()
        })?)
        .await?
        .into_inner();
    let first = updates
        .message()
        .await?
        .ok_or_else(|| anyhow::anyhow!("update stream ended"))?;
    anyhow::ensure!(
        matches!(first.r#type.as_str(), "checkpoint" | "changes"),
        "unexpected update frame"
    );
    println!(
        "Public gRPC verified: {} total records, {records} exported records, snapshot/replay handoff",
        status.records
    );
    Ok(())
}
