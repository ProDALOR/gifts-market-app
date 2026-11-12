use clap::Parser;
use market_history::{ArchiveEntry, HistoryArchive};
use market_reader::{node::NodeConfig, registration::NodeCredentials};
use market_view::protocol::wire as w;
use std::path::PathBuf;
#[derive(Parser)]
struct Args {
    #[arg(
        long,
        env = "MARKET_CORE_URL",
        default_value = "http://127.0.0.1:50052"
    )]
    endpoint: String,
    #[arg(long)]
    core_ca: Option<PathBuf>,
    #[arg(long, requires = "client_key")]
    client_cert: Option<PathBuf>,
    #[arg(long, requires = "client_cert")]
    client_key: Option<PathBuf>,
    /// This archive process owns a registered reader identity.
    #[arg(long)]
    node_id: Option<String>,
    #[arg(long, default_value = ".market/history.json")]
    credentials: PathBuf,
    #[arg(long, env = "MARKET_REGISTRATION_TOKEN", hide_env_values = true)]
    registration_token: Option<String>,
    #[arg(long)]
    output: PathBuf,
    /// Export a stopped archive to JSONL on stdout.
    #[arg(long)]
    dump: bool,
    #[arg(long, default_value_t=8*1024*1024)]
    segment_bytes: u64,
}
fn export(entry: &ArchiveEntry) -> market_json::Result<serde_json::Value> {
    Ok(match entry {
        ArchiveEntry::CatalogPage {
            generation,
            revision,
            offset,
            entries,
            complete,
        } => {
            serde_json::json!({"type":"catalog_page","generation":generation,"revision":revision,"offset":offset,"entries":entries.iter().map(market_json::catalog_value).collect::<Result<Vec<_>,_>>()?,"complete":complete})
        }
        ArchiveEntry::Batch {
            cursor,
            catalog_revision,
            events,
        } => {
            serde_json::json!({"type":"batch","cursor":cursor,"catalog_revision":catalog_revision,"events":events.iter().map(market_json::event_value).collect::<Result<Vec<_>,_>>()?})
        }
        ArchiveEntry::Gap { .. } => serde_json::to_value(entry)?,
    })
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    if args.dump {
        let archive = HistoryArchive::open(&args.output)?;
        archive.visit(|entry| {
            use std::io::Write;
            let mut out = std::io::stdout().lock();
            serde_json::to_writer(
                &mut out,
                &export(&entry).map_err(|error| std::io::Error::other(error.to_string()))?,
            )
            .map_err(|error| std::io::Error::other(error.to_string()))?;
            out.write_all(b"\n")?;
            Ok(())
        })?;
        return Ok(());
    }
    let identity = NodeCredentials::load_or_create(
        &args.credentials,
        args.node_id.as_deref(),
        w::NodeRole::Reader,
    )?;
    let mut node = NodeConfig::new(args.endpoint, identity);
    node.ca = args.core_ca;
    node.client_cert = args.client_cert;
    node.client_key = args.client_key;
    node.registration_token = args.registration_token;
    node.validate()?;
    let mut archive = HistoryArchive::with_options(
        args.output,
        market_history::ArchiveOptions {
            segment_bytes: args.segment_bytes,
        },
    )?;
    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);
    let mut reset = false;
    let mut delay = 1;
    loop {
        let result = tokio::select! {value=history_node::follow(&node,&mut archive,reset)=>value,value=&mut shutdown=>{value?;return Ok(());}};
        match result {
            Ok(()) => return Ok(()),
            Err(error) => {
                if let Some(status) = error.downcast_ref::<tonic::Status>() {
                    match status.code() {
                        tonic::Code::OutOfRange | tonic::Code::FailedPrecondition => reset = true,
                        tonic::Code::Unavailable
                        | tonic::Code::DeadlineExceeded
                        | tonic::Code::Cancelled
                        | tonic::Code::ResourceExhausted => {}
                        _ => return Err(error),
                    }
                } else if error.downcast_ref::<tonic::transport::Error>().is_none() {
                    return Err(error);
                }
                eprintln!("History connection interrupted ({error}); retrying in {delay}s");
            }
        }
        tokio::select! {_=tokio::time::sleep(std::time::Duration::from_secs(delay))=>{},value=&mut shutdown=>{value?;return Ok(());}};
        delay = (delay * 2).min(15);
    }
}
