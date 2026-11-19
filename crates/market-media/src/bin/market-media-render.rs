//! Bounded, disposable renderer process; the parent owns concurrency, timeout and cancellation.
use std::{io::Read, process::ExitCode};
// Exit 2 means deterministic invalid/unsupported media; exit 1 means a retryable IO failure.
fn run() -> Result<(), (u8, String)> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err((
            1,
            "usage: market-media-render input.tgs-or-json output.svg".into(),
        ));
    }
    let file = std::fs::File::open(&args[0]).map_err(|e| (1, e.to_string()))?;
    let mut input = Vec::new();
    file.take(market_media::MAX_TGS_JSON_BYTES as u64 + 1)
        .read_to_end(&mut input)
        .map_err(|e| (1, e.to_string()))?;
    let svg = std::panic::catch_unwind(|| market_media::first_frame_svg(&input))
        .unwrap_or(Err(market_media::MediaError::InvalidMedia))
        .map_err(|e| (2, format!("SVG export failed: {e:?}")))?;
    std::fs::write(&args[1], svg).map_err(|e| (1, e.to_string()))
}
fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err((code, error)) => {
            eprintln!("{error}");
            ExitCode::from(code)
        }
    }
}
