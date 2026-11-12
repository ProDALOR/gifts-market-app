//! A trusted service connects directly to core with its own issued service token.
use clap::Parser;
use market_app::client::CoreClient;
use market_app::wire as a;
use tonic::Request;
#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "http://127.0.0.1:50051")]
    endpoint: String,
    #[arg(long)]
    item_type: Option<String>,
    #[arg(long, default_value = "TON")]
    currency: String,
}
fn request<T>(core: &CoreClient, value: T) -> Request<T> {
    let mut request = Request::new(value);
    request
        .metadata_mut()
        .insert("authorization", core.authorization());
    request.set_timeout(std::time::Duration::from_secs(10));
    request
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let core = CoreClient::new(args.endpoint, std::env::var("MARKET_NODE_TOKEN")?)?;
    let channel = core.channel();
    let mut client = a::market_read_client::MarketReadClient::new(channel.clone());
    let status = client
        .get_status(request(&core, a::Empty {}))
        .await?
        .into_inner();
    anyhow::ensure!(status.ready, "core not ready");
    anyhow::ensure!(
        !status.started_at.is_empty() && status.oldest_cursor.is_some(),
        "core operational status missing"
    );
    anyhow::ensure!(
        status.records == 0 || status.record_bytes > 0,
        "core storage counters missing"
    );
    let stored = client
        .list_stored_orders(request(
            &core,
            a::OrdersQuery {
                item_type_id: args.item_type.clone(),
                currency_id: Some(args.currency.clone()),
                limit: Some(10),
                ..Default::default()
            },
        ))
        .await?
        .into_inner();
    anyhow::ensure!(stored.meta.is_some(), "stored order metadata missing");
    let mut replay = client
        .replay(request(
            &core,
            a::SyncRequest {
                after: status.oldest_cursor,
            },
        ))
        .await?
        .into_inner();
    let mut batches = 0;
    let mut checkpoint = false;
    while let Some(frame) = replay.message().await? {
        match a::FrameKind::try_from(frame.kind)? {
            a::FrameKind::Delta if !checkpoint => batches += usize::from(frame.complete),
            a::FrameKind::Checkpoint if !checkpoint => checkpoint = true,
            _ => anyhow::bail!("unexpected bounded replay frame"),
        }
    }
    anyhow::ensure!(
        checkpoint && batches <= 10,
        "bounded replay did not complete"
    );
    if let Some(kind) = args.item_type.clone() {
        let query = a::BookQuery {
            item_type_id: kind,
            currency_id: args.currency,
            ..Default::default()
        };
        client.get_order_book(request(&core, query.clone())).await?;
        let mut updates = client
            .watch_order_book(request(&core, query))
            .await?
            .into_inner();
        anyhow::ensure!(
            updates
                .message()
                .await?
                .is_some_and(|frame| frame.asks.is_some()),
            "book subscription missing initial view"
        );
    }
    client
        .list_trades(request(
            &core,
            a::TradesQuery {
                limit: Some(10),
                ..Default::default()
            },
        ))
        .await?;
    let mut replication = a::market_read_client::MarketReadClient::new(channel);
    let mut stream = replication
        .sync(request(&core, a::SyncRequest { after: None }))
        .await?
        .into_inner();
    let mut records = 0;
    while let Some(frame) = stream.message().await? {
        records += frame.records.len();
        if frame.kind == a::FrameKind::Checkpoint as i32 {
            break;
        }
    }
    println!(
        "Direct core gRPC verified: {} core records, {records} streamed records, {} stored orders, {batches} replay revisions, durable trade query",
        status.records,
        stored.data.len()
    );
    Ok(())
}
