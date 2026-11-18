# Partition indexes and direct replacement benchmark

Historical results for the grouped-index change. See the [partition-owned orders report](partition-owned-orders-benchmark.md) for the current layout and measurements.

Measured on 2026-11-18. This follow-up compares the preceding shared-reference implementation with grouped partition indexes and direct order replacement. The earlier [shared-reference benchmark](ranked-index-benchmark.md) used the original `f9d5dd9` implementation as its baseline; the baseline here already has shared references and borrowed ranked reads.

## Layout measured in this iteration

```text
State.entities: Key → Arc<Entity>                 authoritative payloads
State.books.partitions: BookPartition → Arc<PartitionState>
    PartitionState.prices:  RankKey → Arc<Entity>
    PartitionState.times:   RankKey → Arc<Entity>
    PartitionState.numbers: RankKey → Arc<Entity>
```

`BookPartition` is venue + collection + currency + side. There is one outer partition map instead of three. Partition state is copy-on-write: snapshots share unchanged partitions, and a mutation clones the touched partition's three persistent index roots only when needed. No fourth per-partition order map was added. Catalog and feed entities remain in `entities`; execution history remains in redb.

Same-partition replacements update the authoritative entity map directly, replace all three indexed record references, and remove only rank entries whose values changed. Metadata-only replacements preserve rank entries while refreshing their payloads. The active-book directory and feed references remain untouched on this path. Moves remove the old partition entry and add the new one; deletion and terminal updates remove empty partitions. New placement and ranks are validated before index changes.

## Method

- Apple MacBookPro18,1, arm64, 32 GiB RAM; rustc 1.98.0 (88d9e12ae), Cargo 1.98.0.
- The identical deterministic benchmark harness ran on both versions: 40,000 active orders, eight venues, two collections, two currencies, 16 models, approximately 75% asks, exact prices and tied ranks.
- Public `ReplicaReader::read` cases return 200 owned messages and include request cloning, filtering and tokens. `price_sparse` matches approximately one in 16 candidates. `price_continuation` begins after five pages. The identity case also uses the shared reader path.
- Each process warms each case four times, then records nine samples of 32 operations. Three before/after pairs ran sequentially, reversing pair order in pair two. Both binaries were built before the paired runs. No builds or other project checks ran during timing. Tables show the median of three process medians.
- In-memory mutation operations retain the original persistent state, clone its roots, apply 100 changes, then drop the temporary state. They include copy-on-write and destruction. Cases change price, metadata at unchanged ranks, side/collection, or remove records.
- `persist_update_100` replaces the same 100 payloads and advances the checkpoint through `ReplicaWriter::apply`, including redb transaction and publication. It tests durable same-rank replacement, not changed-price ingestion. Identical payloads still receive a new shared record; the implementation does not special-case equality.
- Allocation counters use a separate release build with `profile-allocations`: four warmups and one sample of 32 operations. The latency build uses the normal allocator. Retained memory is requested allocations minus freed bytes for one constructed state, excluding fixtures, database, retained snapshots and allocator/RSS overhead. Reallocation growth/shrink is already included in those byte counters.

## Latency

Negative changes mean faster. Read rows use microseconds per 200-order page; mutation rows use milliseconds per 100-record batch.

| Case | Shared references before | Grouped + replacement after | Change |
| --- | ---: | ---: | ---: |
| `price_multi` | 157.099 µs | 158.432 µs | +0.8% |
| `price_single` | 143.164 µs | 144.961 µs | +1.3% |
| `time_desc_multi` | 146.254 µs | 148.839 µs | +1.8% |
| `number_desc_multi` | 146.100 µs | 144.397 µs | -1.2% |
| `price_sparse` | 1298.051 µs | 1237.631 µs | -4.7% |
| `identity_control` | 232.038 µs | 228.798 µs | -1.4% |
| `price_continuation` | 156.184 µs | 156.143 µs | -0.0% |
| `update_100_price` | 2.415 ms | 2.095 ms | -13.3% |
| `update_100_metadata` | 2.230 ms | 1.687 ms | -24.3% |
| `update_100_partition` | 2.428 ms | 2.215 ms | -8.8% |
| `remove_100` | 1.844 ms | 1.753 ms | -5.0% |
| `persist_update_100` | 8.023 ms | 7.282 ms | -9.2% |

The same-partition price and metadata batches improve approximately 13% and 24%. Partition moves improve approximately 9%, deletion approximately 5%, and durable same-rank batches approximately 9%. Nonselective ranked read medians move between -1.2% and +1.8%; there is no material read-speed improvement established by this refactor. Selective reads measured 4.7% faster in this run. These are local synthetic results, not production throughput guarantees.

## Allocations and retained memory

Per operation, measured separately from latency. Read allocations are unchanged. Update gains come from reduced tree and directory work; output messages and pagination behavior are unchanged.

| Case | Allocations before → after | Requested bytes before → after |
| --- | ---: | ---: |
| `price_multi` | 3,688.00 → 3,688.00 | 409,514.00 → 409,514.00 |
| `price_single` | 3,643.00 → 3,643.00 | 407,873.00 → 407,873.00 |
| `time_desc_multi` | 3,688.00 → 3,688.00 | 408,829.00 → 408,829.00 |
| `number_desc_multi` | 3,688.00 → 3,688.00 | 408,899.00 → 408,899.00 |
| `price_sparse` | 18,811.00 → 18,811.00 | 539,084.00 → 539,084.00 |
| `identity_control` | 6,738.00 → 6,738.00 | 427,815.00 → 427,815.00 |
| `price_continuation` | 3,714.00 → 3,714.00 | 410,690.00 → 410,690.00 |
| `update_100_price` | 39,051.00 → 35,002.00 | 3,020,325.00 → 2,905,093.00 |
| `update_100_metadata` | 36,223.00 → 30,538.00 | 2,786,039.00 → 2,557,097.00 |
| `update_100_partition` | 37,340.00 → 34,857.00 | 3,543,669.00 → 3,470,883.00 |
| `remove_100` | 32,157.00 → 31,518.00 | 2,760,479.00 → 2,747,159.00 |
| `persist_update_100` | 37,066.97 → 32,868.03 | 3,112,507.12 → 2,985,101.50 |

Retained state: **103,072,159 → 103,060,975 bytes** (98.30 → 98.29 MiB), **-0.011%**. This is effectively unchanged. Metadata replacement allocations decrease approximately 15.7%. The fixture measures one state; peak memory with many concurrent retained snapshots still needs workload-specific profiling.

## Reproduce and provenance

From the repository root:

```sh
MARKET_BENCH_LABEL=partition-after CARGO_INCREMENTAL=0 cargo test --release --locked --offline -p market-replica benchmarks::orderbooks_benchmark -- --ignored --exact --nocapture --test-threads=1
MARKET_BENCH_LABEL=partition-after CARGO_INCREMENTAL=0 cargo test --release --locked --offline -p market-replica --features profile-allocations benchmarks::orderbooks_benchmark -- --ignored --exact --nocapture --test-threads=1
```

`MARKET_BENCH_ORDERS` and `MARKET_BENCH_ITERATIONS` override 40,000 and 32. Fixtures must fill every 200-order case. Install the locked dependencies first if needed. Do not use `--all-features` for latency runs because it enables test allocator instrumentation. Normal test runs ignore the manual benchmark.

For another comparison, capture a baseline executable and results before editing, build both versions before timing, then alternate their test executables with identical arguments and fixture settings. This baseline was captured from the working tree at the end of the shared-reference iteration, not from a separate commit. Its source SHA-256 fingerprints are:

| Baseline file | SHA-256 |
| --- | --- |
| `crates/market-replica/src/book_index.rs` | `4ec7a9d4612b592aeaf0c66fa6eb21501df13009b9ea451fe835e7ecc0ebbfd0` |
| `crates/market-replica/src/state.rs` | `a0070dcceefeed1f415841a3f53a78efd4077d30fd2cfd576852810f88eeb811` |
| `crates/market-replica/src/benchmarks.rs` | `0bf2ea6a4aab30b280648a0ce4a1a961531523fa117bc5db814817e042a730d9` |

[Raw measurements](benchmarks/partition-index-results.jsonl) include all process medians, sample minima/maxima, fixture metadata and allocation counters. Existing regression coverage for snapshots, cross-venue ties, cursor compatibility, scan/byte limits, durable rollback and restart remains in place. Added tests cover independent rank changes, unchanged directory roots and unrelated partition identity, invalid replacement rollback, and feed references during side/book moves and terminal removal. The original pointer-sharing test also checks directory stability for metadata-only replacements.

## Validation

- `cargo fmt --all -- --check`: passed.
- `CARGO_INCREMENTAL=0 cargo clippy --locked --offline --workspace --all-targets --all-features -- -D warnings`: passed.
- `CARGO_INCREMENTAL=0 cargo test --locked --offline --workspace --all-features`: 263 passed, three ignored (the manual benchmark and two browser tests). The manual release benchmark was run explicitly in both configurations above.
