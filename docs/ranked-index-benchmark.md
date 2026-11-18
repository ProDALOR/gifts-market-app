# Shared ranked-order references benchmark

Historical results for the initial shared-reference change. See the [partition-owned orders report](partition-owned-orders-benchmark.md) for the current layout and measurements.

Measured on 2026-11-18 against baseline `f9d5dd9`. The baseline contains the same benchmark harness and allocation instrumentation, with the original production ranking, state and reader code. That implementation kept `State.entities` authoritative and stores references to its immutable `Arc<Entity>` in the price, time and gift-number indexes. At that stage, orders had not been moved into partitions.

## Method

- Apple MacBookPro18,1, arm64, 32 GiB RAM; rustc 1.98.0 (88d9e12ae), Cargo 1.98.0.
- Release optimization, incremental compilation disabled. Deterministic fixture: 40,000 active orders, eight venues, two collections, two currencies, 16 models, approximately 75% asks. Exact decimal prices and tied ranks exercise cross-venue merging.
- Read cases use the public `ReplicaReader::read` path, including request cloning, filtering, pagination tokens and owned output messages. Each page contains 200 orders. The selective case matches approximately one in 16 candidates. Continuation starts after five pages.
- Each process warms each case four times, then measures nine samples of 32 operations. Three paired before/after processes ran sequentially, reversing pair order in the second pair. No compilation or other project checks ran during these paired measurements. Tables show the median of the three process medians, not a production throughput guarantee.
- In-memory mutation cases pin the original persistent state, clone its roots and apply/drop a batch of 100 records each iteration. Cases change price, payload without changing rank, side/collection, or remove records. They include copy-on-write and temporary-state destruction. The durable case republishes the same 100 records with a new checkpoint through `ReplicaWriter::apply`, including the redb transaction; it measures same-rank replacement rather than changed-price ingestion.
- Allocation counts use a separate release build with `profile-allocations` and the [stats_alloc system-allocator wrapper](https://docs.rs/stats_alloc/0.1.10/stats_alloc/). Each case has four warmups and one sample of 32 operations. Instrumentation is test-only; the latency build uses the normal allocator. Retained memory is requested allocated bytes minus freed bytes after constructing one state; it excludes input fixtures, database, other views and allocator/RSS overhead. Allocation-byte counters already include realloc growth/shrink.

The `identity_control` case also benefits from the shared scan simplification: identity scans now retain the record returned by the entity iterator, avoiding their own second lookup. It is a comparison case, not an unchanged control.

## Latency

Negative changes mean faster. Reads are microseconds per 200-order page; mutation rows are milliseconds per 100-record batch.

| Case | Before | After | Change |
| --- | ---: | ---: | ---: |
| `price_multi` | 228.565 µs | 155.380 µs | -32.0% |
| `price_single` | 200.260 µs | 145.206 µs | -27.5% |
| `time_desc_multi` | 224.324 µs | 152.924 µs | -31.8% |
| `number_desc_multi` | 208.090 µs | 148.572 µs | -28.6% |
| `price_sparse` | 4444.562 µs | 1388.960 µs | -68.7% |
| `identity_control` | 448.409 µs | 239.237 µs | -46.6% |
| `price_continuation` | 236.730 µs | 160.277 µs | -32.3% |
| `update_100_price` | 2.141 ms | 2.604 ms | +21.6% |
| `update_100_metadata` | 1.913 ms | 2.416 ms | +26.3% |
| `update_100_partition` | 2.079 ms | 2.618 ms | +26.0% |
| `remove_100` | 1.621 ms | 2.146 ms | +32.4% |
| `persist_update_100` | 7.580 ms | 8.193 ms | +8.1% |

## Allocations and memory

Per operation, measured separately from timing. Output message cloning and per-candidate continuation construction remain, so covering indexes primarily remove lookups rather than allocations. Borrowing heap keys avoids intermediate ownership; the saved allocations are mostly the initial merge heads and the lookahead row.

| Case | Allocations before → after | Requested bytes before → after |
| --- | ---: | ---: |
| `price_multi` | 3,705.00 → 3,688.00 | 410,000.00 → 409,514.00 |
| `price_single` | 3,646.00 → 3,643.00 | 408,117.00 → 407,873.00 |
| `time_desc_multi` | 3,705.00 → 3,688.00 | 409,315.00 → 408,829.00 |
| `number_desc_multi` | 3,705.00 → 3,688.00 | 409,385.00 → 408,899.00 |
| `price_sparse` | 18,828.00 → 18,811.00 | 539,570.00 → 539,084.00 |
| `identity_control` | 6,740.00 → 6,738.00 | 427,829.00 → 427,815.00 |
| `price_continuation` | 3,731.00 → 3,714.00 | 411,176.00 → 410,690.00 |
| `update_100_price` | 38,751.00 → 39,051.00 | 2,801,677.00 → 3,020,325.00 |
| `update_100_metadata` | 35,923.00 → 36,223.00 | 2,586,847.00 → 2,786,039.00 |
| `update_100_partition` | 37,040.00 → 37,340.00 | 3,281,513.00 → 3,543,669.00 |
| `remove_100` | 32,157.00 → 32,157.00 | 2,545,951.00 → 2,760,479.00 |
| `persist_update_100` | 36,766.97 → 37,066.97 | 2,919,459.12 → 3,112,507.12 |

Retained state: **101,460,383 → 103,072,159 bytes** (96.76 → 98.30 MiB), **+1.59%**. The added 1,611,776 bytes are approximately 40.3 bytes per active order for this fixture. Three indexes own additional shared pointers and map-node storage; they do not copy order payloads.

## Interpretation

Ranked reads improve approximately 28–32%; selective reads improve approximately 69%. The cost is approximately 22–32% slower copy-on-write mutation batches, 8% slower durable same-rank batches and 1.6% more retained state in this fixture. Updating a covering index now copies shared-pointer slots and adjusts reference counts in copied nodes. This is a measurable read/write tradeoff, not an across-the-board optimization.

This provides a baseline for the partition-storage decision. Measure the actual deployment's read/update ratio, order distribution and concurrent retained views before changing primary storage. These results alone do not establish that moving orders into partitions is necessary. Disk throughput, HTTP serialization, websocket fanout and multi-reader contention were not benchmarked.

## Reproduce

From the repository root, run latency and allocation profiles separately:

```sh
MARKET_BENCH_LABEL=after CARGO_INCREMENTAL=0 cargo test --release --locked --offline -p market-replica benchmarks::orderbooks_benchmark -- --ignored --exact --nocapture --test-threads=1
MARKET_BENCH_LABEL=after CARGO_INCREMENTAL=0 cargo test --release --locked --offline -p market-replica --features profile-allocations benchmarks::orderbooks_benchmark -- --ignored --exact --nocapture --test-threads=1
```

Use `MARKET_BENCH_ORDERS` and `MARKET_BENCH_ITERATIONS` to override 40,000 and 32. Smaller fixtures must still fill every 200-record read case. Install the locked dependencies first if the local Cargo cache is empty. Do not enable `--all-features` for latency runs: it enables allocator instrumentation. Ordinary tests ignore the benchmark.

To regenerate the original baseline, apply only `src/benchmarks.rs`, its test-module declaration, the `profile-allocations` feature, the dev dependency and lockfile changes to `f9d5dd9` in a separate checkout. Keep the original `book_index.rs`, `reader.rs`, `state.rs` and `types.rs`. Compile both checkouts before alternating their test executables, using the same fixture and machine conditions.

[Raw measurements](benchmarks/ranked-index-results.jsonl) include all process medians, sample minima/maxima, allocation counters and fixture metadata.

## Regression coverage and validation

Six new tests cover pointer identity across all three indexes and the authoritative map; same-rank payload replacement; pinned old records; price/time/number and side/collection/currency changes; terminal transitions and repeated removal; cross-venue ties, ranges and continuation boundaries; empty pages at the scan budget; byte budgets and oversized rows; and public snapshot continuations after same-rank updates.

Validation passed: `cargo test --workspace --all-features` (260 passed, three ignored: this manual benchmark and two browser tests), strict workspace Clippy across all targets/features, and `cargo fmt --all -- --check`. All Cargo checks used the lockfile and offline cache. Manual release benchmarks were run separately as described above.
