//! Bounded, disposable renderer process; the parent owns concurrency, timeout and cancellation.
use std::{io::Read, process::ExitCode};
// Exit 2 means deterministic invalid/unsupported media; exit 1 means a retryable IO failure.
fn run() -> Result<(), (u8, String)> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err((
            1,
            "usage: market-media-render input.tgs-or-json output.svg | input.svg output.png".into(),
        ));
    }
    let path = std::path::Path::new(&args[0]);
    let is_svg = path.extension().is_some_and(|ext| ext == "svg");
    let gallery = is_svg && path.file_name().is_some_and(|name| name == "gallery.svg");
    let max_bytes = if gallery {
        market_media::MAX_GALLERY_SVG_BYTES
    } else {
        3 * market_media::MAX_PREVIEW_BYTES
    };
    let file = std::fs::File::open(path).map_err(|e| (1, e.to_string()))?;
    let mut input = Vec::new();
    file.take(max_bytes as u64 + 1)
        .read_to_end(&mut input)
        .map_err(|e| (1, e.to_string()))?;
    if is_svg {
        let png = std::panic::catch_unwind(|| {
            if gallery {
                market_media::render_gallery_png(&input)
            } else {
                market_media::render_cover_png(&input)
            }
        })
        .unwrap_or(Err(market_media::MediaError::InvalidMedia))
        .map_err(|e| (2, format!("Artwork render failed: {e:?}")))?;
        return std::fs::write(&args[1], png).map_err(|e| (1, e.to_string()));
    }
    let svg = std::panic::catch_unwind(|| market_media::first_frame_svg(&input))
        .unwrap_or(Err(market_media::MediaError::InvalidMedia))
        .map_err(|e| (2, format!("SVG export failed: {e:?}")))?;
    std::fs::write(&args[1], svg).map_err(|e| (1, e.to_string()))
}
fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(std::io::stderr)
        .with_max_level(tracing::Level::WARN)
        .init();
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err((code, error)) => {
            eprintln!("{error}");
            ExitCode::from(code)
        }
    }
}
