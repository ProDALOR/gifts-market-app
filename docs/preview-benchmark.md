# Gift preview format comparison

## Decision

Model stills now use **cached Rust-generated first-frame SVGs** as ordinary images at every size, including listing cards and collection rows. Telegram thumbnails remain the fallback if an SVG is unavailable. Instantiate Lottie only for TGS playback; enlarged playback uses SVG. Reduced motion keeps static images and creates no player.

The [SVG quality gate](native-svg-validation.md) now passes all 43 gifts. `market-media` generates and persists SVGs on demand from the existing TGS cache with bounded, coalesced Rust jobs. The API serves a catalog-scoped preview URL with compression and conditional caching. Runtime gift media is not committed; the fixed Liberty Figure branding assets are bundled separately. The measurements below describe the original browser-export comparison; Rust-export size and quality results are recorded separately.

## Method

Measured on 2026-11-19 using local cached Telegram gifts, without authenticating or making hub/Telegram requests. The benchmark uses the installed Lottie and Chromium versions. It selects 40 evenly spaced quantiles of decompressed animation size from 427 cached TGS files, then includes cached brick compositions and the real Artisan Brick opacity-gradient regression (`5195366267158033241`, composition name `Hot video card`). This produces 43 distinct gifts, including simple shapes, detailed gradients, transparency, precompositions and pixel art. Composition names in the report come from the animation, not the catalog.

For each gift, render frame zero with Lottie's SVG renderer, serialize that frame to a standalone SVG, and rasterize that exact SVG into transparent 256px and 512px WebP at browser encoder quality 0.9. Include the original Telegram WebP as a separate practical baseline. The compressed SVG sizes use gzip; uncompressed sizes are also recorded. SVG therefore gets the benefit of HTTP compression as the production preview API now does.

For quality, independently screenshot the live Lottie SVG at 96px, 320px and 512px. Compare each static image against that reference at the same output size, composited over both dark and light backgrounds. This avoids treating fully transparent RGB as visible error or comparing a sharp vector to a reference blurred by downsampling. The report records PSNR and the percentage of pixels with a channel error above 10/255. PSNR is a fidelity measurement against this renderer, not a perceptual quality score or proof of parity with Telegram's native renderer. Telegram's chosen poster frame may differ from frame zero, so its numerical differences are not purely compression loss.

Performance runs rotate format order seven times. Fetching and decoding are timed separately; fresh Blob URLs prevent decoded-image reuse across runs. After decoding all 43 images, mount 100 ordinary image elements reusing those sources. Measure 45 frames of actual document scrolling, then 45 frames that also resize every image between 80px and 96px. The resize phase is a deliberate stress case, not a normal scrolling estimate. Frame intervals above 25ms are counted separately. CDP records renderer task/layout time; its JS heap counter does not include total decoded-image or GPU memory.

## Results

Apple M1 Pro, macOS arm64, Chromium 159.0.8154.6, 1200 × 900 viewport at device scale 1. Values below are medians: bytes per gift and timing per 43-image batch/100-image grid, not time per image. These local headless-browser measurements are not mobile-device or network-throughput guarantees.

| Static source | Median transfer | Decode 43 images | Scroll frame | Resize frame | Resize frames >25ms, out of 45 |
| --- | ---: | ---: | ---: | ---: | ---: |
| Extracted SVG, gzip | 12.4 KiB | 134.7 ms | 16.7 ms | 27.0 ms | 34 |
| Generated WebP, 256px, quality 0.9 | 12.7 KiB | 42.4 ms | 16.7 ms | 16.7 ms | 0 |
| Generated WebP, 512px, quality 0.9 | 28.7 KiB | 112.7 ms | 16.7 ms | 16.7 ms | 0 |
| Existing Telegram WebP, 128px | **6.1 KiB** | **20.0 ms** | 16.7 ms | 16.7 ms | 0 |

The SVG median before compression is 98.7 KiB, and individual SVGs contain 130–10,395 descendant nodes. All 43 SVGs total 607 KiB after gzip, versus 571 KiB for 256px WebP, 1,283 KiB for 512px WebP and 257 KiB for the existing thumbnails. SVG is smaller than 512px WebP here, but not smaller than the actual default. Gzip SVG and WebP256 have similar median transfer sizes; the existing thumbnail batch is 58% smaller than the SVG batch and decodes about 6.7 times faster.

| Same-frame fidelity, median PSNR | At 96px | At 320px | At 512px |
| --- | ---: | ---: | ---: |
| Extracted SVG | Exact match | Exact match | 51.3 dB |
| Generated WebP256 | 32.4 dB | 31.6 dB | 29.7 dB |
| Generated WebP512 | 32.7 dB | 36.9 dB | 39.2 dB |

Exact pixel matches are recorded as 100 in JSON. Vector rerasterization and scaling a raster image use different edge sampling, so even visually good WebP can differ numerically at 96px. Visual inspection confirms sharper fine lines and gradients in SVG/512px WebP at larger sizes; the existing thumbnail is visibly softer. The native Telegram baseline is omitted from this fidelity table because its poster-frame choice is independent.

Generation itself takes a median 63.7 ms per gift in the browser, including Lottie initialization, frame extraction and both WebP exports. This excludes downloading the TGS and is not a server conversion benchmark. The Rust runtime converter has its own bounded workers and persistent derivative cache; these browser-generation timings do not measure it.

The CPU-throttled follow-up uses 12 complexity quantiles plus the three regression cases (15 gifts), three rotated runs and a 4× CPU slowdown. SVG decoding takes 221.3 ms versus 20.1 ms for WebP256, 43.5 ms for WebP512 and 13.1 ms for the existing thumbnails. Normal scrolling remains about 16.7 ms per frame for every format; resizing rises to 33.3 ms for SVG, with 44/45 frames over 25ms, while all WebP variants remain near 16.7 ms with no frames over 25ms. These batch timings must not be compared directly to the 43-gift unthrottled batch. Its artifacts are in `.market/run/preview-benchmark-throttled`.

The machine-specific results and exact source SHA-256 hashes are saved in `.market/run/preview-benchmark/results.json`. `comparison.html` and `comparison.png` show all four formats side by side on the same backdrop.

## Quality and application impact

SVG preserves scalable contours and fine details. The generated 512px WebP also looks considerably clearer than the existing thumbnail at 160 CSS pixels and at high device pixel ratios, at a higher transfer cost. The 256px export is a useful middle ground, but still costs an additional TGS download and conversion on a cache miss.

The original 128px WebP is suitable for the common 48px grid image at 2× device pixel ratio, but becomes visibly softer when enlarged. The GUI now prefers server-cached SVG images for all model stills, keeping static rendering in native images. The measurements above remain the original comparison, rather than a new benchmark of that policy. Lottie is still instantiated only for actual playback, sharing its existing download and playback budgets.

All 43 sampled first frames contain visible artwork. That does not guarantee frame zero is a suitable poster for every future gift. Telegram's thumbnail remains the fallback for derived previews.

## Reproduce

The follow-up [native SVG validation](native-svg-validation.md) records the exporter fixes and passing quality gate used to move first-frame generation into `market-media`.

Install the frontend dependencies and Playwright Chromium, then point the script at a populated media cache:

```sh
npm --prefix web run benchmark:previews -- --media-dir /path/to/media --samples 40 --runs 7
```

The default cache is `data/live-hub/media`. Output defaults to `.market/run/preview-benchmark`; `--output /path/to/results` selects another directory. Relative cache/output paths resolve from the repository root, including when invoked through npm. TGS-only caches are supported, with the Telegram WebP baseline included only when available. Neither source nor generated gift assets are committed. The script does not alter the source cache.

Use `--corpus web/scripts/fixtures/preview-corpus.json` to select the exact 43-gift regression corpus by document ID and SHA-256 instead of resampling a changed cache. The Rust-export quality gate requires this pinned corpus by default; the fixture stores text metadata only.

For a CPU-throttled follow-up, use `--cpu-rate 4` and a separate output directory. CPU throttling on desktop Chromium is not a real mobile-device benchmark and does not emulate its GPU, network or memory limits.
