# First-frame SVG validation

The production converter is a pure Rust binary in `market-media`, using published Velato 0.12.0 with a first-frame compatibility adapter. All 43 pinned real gifts pass the browser compatibility gate. Docker builds it with Cargo; no native renderer download, Python, Meson/Ninja or C++ is needed.

## Fixes and quality gate

The initial pure Rust prototype passed 32/43 after supplying omitted transform defaults. The remaining cases exposed independent gradient-opacity stop loss, ignored fill/stroke opacity, and ignored stroke dash patterns. The gradient fix retained all authored breakpoints and exports color and opacity separately. Respecting style opacity brought the result to 42/43. Importing animated stroke dashes fixed the cigar and brought the result to 43/43. The adapter supplies omitted transform defaults and repairs gradients, opacity and dash data through Velato’s public model. The evaluator itself is unmodified.

Every original compressed source must match its SHA-256 in `web/scripts/fixtures/preview-corpus.json`. The reference is Lottie's browser-rendered frame zero from the existing format benchmark. The validator passes each compressed TGS directly to the production helper, then renders its standalone SVG through an ordinary browser image at 512px, composites over dark and light backgrounds, and uses the shared pixel-comparison helper.

Thresholds remain PSNR ≥30 dB and at most 10% of pixels with a channel error >10/255. Export failures, invalid images, embedded raster content and external references fail the gate. No models are excluded and no tolerance was relaxed. This is a compatibility gate for these gifts, not a proof of support for all future TGS. Remaining small rasterization differences are visible in the report.

The final run used Chromium 159.0.8154.6 on macOS arm64. It also exercises the production SVG validator, so over-limit or unsupported output fails before image comparison. Median gzip SVG size was approximately 8.3 KiB. Runtime timings in the report are diagnostics, not a throughput benchmark.

## Reproduce

Populate the regular media cache and install frontend dependencies and Playwright Chromium. Generate the pinned Lottie reference once, then build and check the actual converter:

```sh
cargo build --locked -p market-media --bin market-media-render
npm --prefix web run benchmark:previews -- --corpus web/scripts/fixtures/preview-corpus.json
npm --prefix web run validate:previews
```

The validator exits 1 for any failure and saves `results.json`, hashes, per-gift SVG/PNG outputs and a comparison gallery under `.market/run/svg-validation`. The report records one renderer executable/hash and one summary, without experimental candidate loops or expanded JSON staging. Its `--renderer`, `--media-dir`, `--reference` and `--output` paths resolve from the repository root. The ignored cache integration test additionally checks the real corpus:

```sh
cargo test -p market-media real_corpus_generates_valid_cached_previews -- --ignored
```

## Production lifecycle

The API resolves model documents through the catalog. `MediaCache::preview` first uses the disk index's source identity to look up the derivative directly. A cached SVG remains usable after its TGS is evicted and across offline restarts. Identity is recovered from the existing digest-bearing preview filenames; no sidecar format is required. Startup and maintenance hash changed originals when needed, without decompressing them. Source replacement invalidates older derivatives and rejects conversions that finish against an older source. Content-addressed SVGs remain in memory until byte-budget eviction; originals retain their refresh TTL.

Only a cache miss loads the original TGS, before admitting conversion, avoiding a dependency between download and rendering pools. The helper accepts compressed TGS directly; the parent no longer decompresses and writes expanded JSON. The adapter retains only paint patches before consuming JSON in Velato, keeps native solid opacity/stroke width evaluation, and shares repaired precompositions at equal local frame times. The helper runs out of process so timeout, last-consumer cancellation and shutdown can terminate expensive work and reap the process before releasing a slot.

Two conversions run concurrently, each with a ten-second child timeout. Generated files share the existing disk/memory quotas, request coalescing, atomic writes and validation. The derivative key is `<document-id>.preview-2-<source-sha256>.svg`; the previous C++ derivatives are removed automatically. Catalog URLs include the renderer version so browser caches also refresh. Conditional requests retain weak ETags for compressed SVGs.

Rendering also has limits before and during conversion: source nodes/numeric expansion, parent and precomposition depth, star/repeater expansion, aggregate geometry/draw work, and buffered SVG size. This prevents small inputs from expanding into unrestricted render batches. The helper exits with code 2 for unsupported, invalid or over-budget media and code 1 for IO failures. Permanent failures receive the longer negative-cache TTL; transient failures remain retryable. Stderr is drained concurrently while retaining at most 4 KiB for debug diagnostics.

The helper accepts compressed TGS or validated JSON, rejects external assets and expression-only properties, and has no Telegram access. Expression metadata beside baked values/keyframes is removed without execution. WebP thumbnails and ordinary TGS originals remain unchanged; expression-bearing TGS is normalized before caching or delivery. GUI stills load SVG images with immediate thumbnail fallback and visibility-aware recovery governed by shared retry deadlines. Recovery checks native decoding before remounting, so malformed successful responses cannot cause loops. Recovery accepts the SVG byte budget and honors retryable HTTP statuses. Lottie is instantiated only for playback.

## Loading placeholders and recovery

Gift and symbol silhouettes are bundled GUI vectors with CSS shimmer. They need no HTTP requests, per-model outline cache or placeholder API fields. Artwork readiness removes the placeholder; leaving the viewport, hiding the page or unmounting stops shimmer. Reduced motion keeps a static shape. Startup and disk maintenance remove only retired positive numeric-ID `<document-id>.placeholder.svg` files, preserving unrelated files.

SVG stills and thumbnail fallbacks have independent failure/recovery state. A missing SVG cannot block a retryable thumbnail; a recovered fallback stays visible until the SVG loads. Changing artwork or leaving the viewport cancels recovery consumers. Images, animations and patterns use shared retry deadlines: missing/invalid assets are cached for 60 seconds, transient failures for two seconds or the full server `Retry-After`. Long hints are not shortened; waiting retries release transfer slots. Cancellation is not cached as a miss, and native decoding prevents malformed successful responses from causing remount loops.

Cards and details use the same Telegram-style 18-symbol positions, scales and individual opacities. Attribute-selection banners keep a repeating pattern layout. All symbol inputs use one URL-array interface, and backdrop canvases redraw on size/palette/data changes rather than animation frames.

All Rust media work is demand-driven through bounded FIFO downloads and a separate render pool. No startup thumbnail warming or background prefetch is performed.

## Wise Owl expression compatibility

Wise Owl (`5413686816387672057`, Myth Mask) exposed a rejection before rendering: its 18,894-byte TGS includes a path expression at `layers[41].shapes[0].it[0].ks.x`, alongside baked shape keyframes. Rejecting every string `x` made the animation return 404; SVG generation depends on that animation and also returned 404.

`market-media` now removes expression metadata from properties with a baked value or keyframes. Neither Rust nor browser players execute it. Ordinary TGS bytes stay identical; changed TGS is compressed once, validated and content-hashed before persistence or HTTP delivery. The renderer uses the same normalization rules for direct JSON/TGS input. External assets, fonts, expression-only properties, decompression and rendering limits remain enforced.

The real source SHA-256 was `4e6fabf6327f5eedcc02ef3b33e1a0df3936238b46bd3d8fa050826fdc8c44df`. At 512px, the native SVG matched Lottie light with PSNR 38.73 dB and 0.54% changed pixels; all four sampled animation frames differed. The source and generated artwork stay outside the repository. Small synthetic tests reproduce the expression/path structure, verify unchanged keyframes, expression-free output, idempotent normalization, original-byte preservation and useful validation logs.

To repeat the real-file download/cache/renderer/offline-restart regression after building `market-media-render`:

```sh
MARKET_TEST_TGS=/path/to/wise-owl.tgs cargo test -p market-media wise_owl_download_preview_and_offline_restart -- --ignored
```

Media failure logs include document ID, variant and stage. Telegram lookup/download failures report RPC code/name or MIME/size rejection; TGS validation reports a stable reason such as `expression_without_keyframes`, `external_asset` or `expanded_size`. Renderer spawn, timeout and exit failures are visible at warning level. Payloads, expressions and Telegram file credentials are not logged. Failed jobs are negatively cached, so repeated HTTP requests do not repeat the source failure log. To trace rare successes/normalizations as well, use `RUST_LOG=info,market_media=debug,market_telegram_client=debug`.
