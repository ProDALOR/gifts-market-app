# Rust SVG exporter assessment

## Result

Production now uses a pure Rust `market-media-render` binary. The corrected Velato pipeline passes all 43 pinned gifts using the original browser thresholds. The C++ helper and its build scripts were removed.

## Original comparison

On 2026-11-19, the same source hashes and Lottie frame-zero references were tested through browser image elements:

| Pipeline | Exported | Quality pass |
| --- | ---: | ---: |
| Patched C++ ThorVG | 43/43 | 43/43 |
| Stock Velato 0.12.0 | 1/43 | 1/43 |
| Velato with explicit omitted defaults and experimental SVG backend | 43/43 | 32/43 |
| Renamite Lottie/SVG 0.3.18 | 43/43 | 3/43 |
| Corrected Velato, production binary | 43/43 | 43/43 |

Stock Velato rejected omitted position or panicked on omitted rotation. Renamite's best-effort importer skipped null layers and did not preserve parenting, visibly displacing gift parts. Its larger graph included text/font handling and ICU dependencies. Rasterlottie, resvg/usvg and Inlottie were screened but did not offer a suitable standalone Lottie-to-SVG path. Rust bindings to ThorVG and dotlottie-rs retained C++ compilation.

The isolated baseline probes built with `CC=false` and `CXX=false`. Velato with Vello/GPU features disabled used 23 active Cargo packages including the probe; Renamite used 128. Cold release compilation with fetched dependencies and one job took approximately 30 seconds versus 219 seconds on the measured Mac. These are historical host-specific probe measurements, not application or Docker build guarantees.

## Correctness fixes

The remaining 11 Velato failures exposed three problems:

1. Independent gradient opacity stops were collapsed onto color stops. Intermediate opacity breakpoints were lost. Our adapter retains the union of breakpoints and their authored discontinuities; animated raw values are interpolated before that conversion. The SVG backend exports separate color and opacity gradients.
2. Gradient fill/stroke opacity was hardcoded to 100%. Respecting authored opacity brought the corpus to 42/43.
3. Stroke dashes were discarded. The cigar's glowing material became solid colored strokes. Preserving animated dash/gap lengths and offsets fixed the last gift.

The published `velato = 0.12.0` crate is now used directly, with Vello/GPU features disabled. A first-frame adapter in `market-media` normalizes omitted defaults and terminal keyframes, and repairs paint and stroke data through the public model. It delegates easing, geometry and transforms to Velato, samples reused precompositions in each instance’s local time, and rejects source/model mismatches. No fork is shipped. The dependency is pinned because this adapter relies on the public model's structure; upgrades must rerun the compatibility gate. The original 43/43 result does not imply every pixel matches or every future gift is supported; no thresholds or corpus exclusions were changed to obtain it.

## Current build

A regular Cargo build produces both binaries. The production dependency uses disabled Vello/GPU features, and requires no separate compiler or generator. The old experiment workspace was removed. The subprocess remains to preserve cancellation, hard timeouts and application isolation. Replacing it with in-process work would weaken those lifecycle guarantees.

See [validation](native-svg-validation.md) for commands and the gate. Revalidate all 43 gifts before updating the evaluator or SVG backend.
