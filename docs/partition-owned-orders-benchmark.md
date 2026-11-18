# Partition-owned active orders benchmark

Measured on 2026-11-18. The baseline is the immediately preceding grouped-index/direct-replacement implementation: active orders still belonged to the global entity map. Both versions use the expanded benchmark harness, including identity-key, mixed-kind and metadata reads. The earlier [grouped-index report](partition-index-benchmark.md) and [shared-reference report](ranked-index-benchmark.md) retain their historical measurements; their baseline timings should not be substituted for the paired baseline here.

## Current layout

```text
State.metadata: Key → Arc<Entity>                    catalog and feeds only
State.books.partitions: Arc<BookPartition> → Arc<PartitionState>
    PartitionState.orders:  order ID → Arc<Entity>   authoritative active orders
    PartitionState.prices:  RankKey → Arc<Entity>
    PartitionState.times:   RankKey → Arc<Entity>
    PartitionState.numbers: RankKey → Arc<Entity>
State.books.locations: Key → Arc<BookPartition>      ordered routing directory
```

Partition keys are interned and shared by location entries. The local order map needs only the order ID because venue is part of the partition key. The final implementation uses compact persistent ordered maps: the measured hash-map alternative used considerably more retained memory. All maps are immutable across pinned views and use copy-on-write for updates. Indexed records share the same payload as the owning partition; locations contain no order payloads.

`State::entity`, `State::range`, `State::get` and the read-only `State::books` accessor hide storage layout from readers and revision/dependency tracking. Both underlying State fields are private. Metadata scans avoid the order directory. Generic identity scans concatenate metadata and the ordered location directory, preserving the previous `(kind, venue, ID)` ordering; order records are resolved inside their partitions. Order remains the final entity kind in this legacy ordering.

Same-partition replacement updates the local order map and refreshes the three ranked references without rewriting the location directory. Partition moves, deletes and terminal transitions update the directory and remove empty partitions. Rank/placement validation precedes mutation. Feed declarations remain independent of active partitions. Existing wire records, database tables/checkpoints and continuation formats are unchanged; reopening reconstructs the location directory from current records.

## Method

- Apple MacBookPro18,1, arm64, 32 GiB RAM; rustc 1.98.0 (88d9e12ae), Cargo 1.98.0.
- Deterministic fixture: 40,000 active orders, eight venues, two collections, two currencies, 16 models, approximately 75% asks, exact decimal prices and tied ranks.
- Public reader cases include request cloning, validation, filtering, continuation encoding and owned response messages. Read pages contain 200 rows except `metadata_options`, which returns all 16 options. `price_sparse` matches approximately one in 16 candidates. `identity_control` is the scoped, filtered identity read from previous reports. `identity_orders` scans all order identities, `identity_keys` requests 200 explicit keys, and `identity_mixed` crosses the metadata/order boundary. `price_continuation` starts after five pages.
- Release builds, incremental compilation disabled, normal allocator for latency. Four warmups per case, then nine samples of 32 operations per process. Three before/after pairs ran sequentially, reversing order in pair two. Binaries were built before timing; no builds or project checks ran during paired timing. Table entries are medians of the three process medians.
- Mutation cases pin the original persistent state, clone roots, apply/drop 100 updates each iteration, and include copy-on-write and destruction. Cases change price, metadata at unchanged ranks, side/collection, or remove records. The durable case repeatedly replaces identical payloads with a new checkpoint through `ReplicaWriter::apply`, including redb and publication. It measures durable same-rank replacement, not changed-price ingestion.
- Allocation counts use a separate release build with `profile-allocations`, four warmups and one 32-operation sample. Retained memory is requested allocated bytes minus freed bytes after constructing one state. Fixture input, database, retained views and allocator/RSS overhead are excluded; realloc growth/shrink is already included in the byte counters.

## Latency

Negative changes mean faster. Reads use microseconds per page; mutations use milliseconds per 100-record batch.

| Case | Global ownership before | Partition ownership after | Change |
| --- | ---: | ---: | ---: |
| `price_multi` | 159.301 µs | 154.389 µs | -3.1% |
| `price_single` | 149.562 µs | 146.070 µs | -2.3% |
| `time_desc_multi` | 156.146 µs | 145.652 µs | -6.7% |
| `number_desc_multi` | 151.996 µs | 145.133 µs | -4.5% |
| `price_sparse` | 1234.832 µs | 1292.626 µs | +4.7% |
| `identity_control` | 252.881 µs | 446.298 µs | +76.5% |
| `identity_orders` | 131.117 µs | 163.691 µs | +24.8% |
| `identity_keys` | 285.288 µs | 338.046 µs | +18.5% |
| `identity_mixed` | 114.279 µs | 142.684 µs | +24.9% |
| `metadata_options` | 3.565 µs | 3.730 µs | +4.6% |
| `price_continuation` | 162.465 µs | 157.023 µs | -3.3% |
| `update_100_price` | 2.511 ms | 2.309 ms | -8.0% |
| `update_100_metadata` | 2.022 ms | 1.893 ms | -6.4% |
| `update_100_partition` | 2.535 ms | 2.600 ms | +2.6% |
| `remove_100` | 2.115 ms | 2.345 ms | +10.8% |
| `persist_update_100` | 7.830 ms | 8.063 ms | +3.0% |

These results show an ownership/performance tradeoff. Same-partition mutation medians improve approximately 6–8%, while partition moves cost approximately 3% more, deletion approximately 11% more, and durable same-rank batches approximately 3% more. Ranked reads continue to use shared indexed records directly; their medians vary between approximately -7% and +5% in this run.

Identity reads pay for location → partition → local-order lookup. Plain order and mixed identity pages are approximately 25% slower, explicit-key reads approximately 19% slower, and the scoped identity scan approximately 77% slower because every scanned candidate is resolved before filtering. Metadata-only reads differ by approximately 5% in this small fixture. This is not a universal speed improvement. HTTP serialization/fanout, concurrent readers and actual deployment read/update ratios were not benchmarked.

## Allocations and retained memory

Per measured operation, separately from latency:

| Case | Allocations before → after | Requested bytes before → after |
| --- | ---: | ---: |
| `price_multi` | 3,688.00 → 3,688.00 | 409,514.00 → 409,514.00 |
| `price_single` | 3,643.00 → 3,643.00 | 407,873.00 → 407,873.00 |
| `time_desc_multi` | 3,688.00 → 3,688.00 | 408,829.00 → 408,829.00 |
| `number_desc_multi` | 3,688.00 → 3,688.00 | 408,899.00 → 408,899.00 |
| `price_sparse` | 18,811.00 → 18,811.00 | 539,084.00 → 539,084.00 |
| `identity_control` | 6,738.00 → 6,738.00 | 427,815.00 → 428,119.00 |
| `identity_orders` | 3,415.00 → 3,415.00 | 404,330.00 → 404,634.00 |
| `identity_keys` | 5,031.00 → 5,031.00 | 502,346.00 → 502,346.00 |
| `identity_mixed` | 3,055.00 → 3,057.00 | 358,250.00 → 358,682.00 |
| `metadata_options` | 91.00 → 91.00 | 5,304.00 → 5,608.00 |
| `price_continuation` | 3,714.00 → 3,714.00 | 410,690.00 → 410,690.00 |
| `update_100_price` | 35,002.00 → 35,039.00 | 2,905,093.00 → 3,059,416.00 |
| `update_100_metadata` | 30,538.00 → 30,575.00 | 2,557,097.00 → 2,711,420.00 |
| `update_100_partition` | 34,857.00 → 37,920.00 | 3,470,883.00 → 3,922,251.00 |
| `remove_100` | 31,518.00 → 34,672.00 | 2,747,159.00 → 3,158,686.00 |
| `persist_update_100` | 32,868.03 → 32,905.03 | 2,985,101.50 → 3,139,440.50 |

Retained state: **103,060,975 → 106,823,431 bytes** (98.29 → 101.87 MiB), **+3.65%**. The additional 3,762,456 bytes cover the separate local identity maps and routing directory in this fixture. Order payloads are still allocated once. Peak memory under concurrent retained snapshots needs workload-specific measurement.

## Reproduce and provenance

```sh
MARKET_BENCH_LABEL=owned-after CARGO_INCREMENTAL=0 cargo test --release --locked --offline -p market-replica benchmarks::orderbooks_benchmark -- --ignored --exact --nocapture --test-threads=1
MARKET_BENCH_LABEL=owned-after CARGO_INCREMENTAL=0 cargo test --release --locked --offline -p market-replica --features profile-allocations benchmarks::orderbooks_benchmark -- --ignored --exact --nocapture --test-threads=1
```

`MARKET_BENCH_ORDERS` and `MARKET_BENCH_ITERATIONS` override 40,000 and 32. Smaller fixtures must fill every 200-row case. Install locked dependencies first if the Cargo cache is empty. Do not enable `--all-features` for latency runs: it enables test allocator instrumentation. Normal test runs ignore the manual benchmark.

For a future comparison, capture the baseline binary before modifying storage and use the same harness in both builds. This baseline was captured from the preceding working-tree implementation, not the original `f9d5dd9` commit. SHA-256 fingerprints of its principal source files:

| Baseline file | SHA-256 |
| --- | --- |
| `book_index.rs` | `ce1d0317c9f194c37580a7311ec144107abc2756597ea49751f8fd4723a550b8` |
| `state.rs` | `f28e5656b8277af921c94f3c6193f1881ecf406fd39f80b7782bac3d69a3f97a` |
| `benchmarks.rs` | `1972939600874c77108e5c949a889d22f1b46392ed933ceed71d031e9e1faa01` |

[Raw measurements](benchmarks/partition-owned-order-results.jsonl) contain every process median, sample minimum/maximum, allocation counter and fixture metadata.

## Regression coverage

New tests cover global/kind identity ranges and metadata isolation; interned location keys, partition moves and removal; mixed metadata/order snapshot pages, old `Current` cursor shape and restart ordering; and explicit keys with missing/deleted orders and moved snapshot payloads. Shared-record invariants now verify local ownership, all three ranked indexes and every location entry. Existing tests retain exact decimal bounds, ranked pagination, snapshot isolation, rejected-batch rollback, history, feed references, scoped revisions and dependency invalidation.

## Validation

- `cargo fmt --all -- --check` and `git diff --check`: passed.
- `CARGO_INCREMENTAL=0 cargo clippy --locked --offline --workspace --all-targets --all-features -- -D warnings`: passed.
- `CARGO_INCREMENTAL=0 cargo test --locked --offline --workspace --all-features`: 267 passed, three ignored (the manual benchmark and two browser tests). The release benchmark was run explicitly in both configurations above.
