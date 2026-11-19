# Ordered query and visited-listing cache benchmarks

Run from the repository root:

```sh
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 cargo test --release --locked -p market-replica ordered_query_benchmark -- --ignored --nocapture
npm --prefix web run benchmark:query
```

Both benchmarks discard eight warmup iterations and report the median of 64 samples. These are local microbenchmarks from the same development machine, measured before and after this refactor. They are not HTTP throughput or browser frame-rate measurements.

The Rust fixture contains 40,000 active orders across collections and venues. It reads 200 price-ranked asks from one collection, then reads the same fixed coverage. The baseline deep-copied wire entities and encoded both `position` and `sort_key` inside every read. The new native read returns shared entity references and typed positions; encoding happens at the transport boundary. The “including keys” rows explicitly perform that encoding, making them the useful comparison to the baseline. Gift projection, JSON order serialization, network costs and state updates are outside this benchmark.

| Operation | Before | After |
| --- | ---: | ---: |
| Head, 200 rows, including keys | 348.25 µs | 123.83 µs |
| Fixed range, 200 rows, including keys | 445.71 µs | 172.13 µs |
| Native head, before encoding | — | 60.33 µs |
| Native range, before encoding | — | 109.58 µs |
| Ordering-key bytes for 200 rows | 70,912 | 44,800 |

The single versioned ordering key reduces navigation/ordering bytes by about 37%. Native read speed also benefits from avoiding repeated key serialization and raw entity clones. Exact range watches share producers; different coverage still runs independently. A relevant replica change still scans its bounded range—this refactor does not add a durable change journal or incremental replica index.

## Compact navigation keys

A subsequent run measured the compact key format against the preceding release test binary on the same machine. The 40,000-order fixture and 200-row price queries are unchanged.

| Operation | Previous keys | Compact keys |
| --- | ---: | ---: |
| Ordering-key bytes for 200 rows | 44,800 | 33,200 |
| Head, 200 rows, including keys | 121.13 µs | 119.04 µs |
| Fixed range, 200 rows, including keys | 171.71 µs | 160.13 µs |

Price-key payloads shrink by 25.9% in this fixture. Timing is a local microbenchmark, not a throughput guarantee. Prices use packed decimal digits with full precision; time and gift-number ranks use integer encoding. Active identity keys omit the rank, and history identity/time keys encode execution time once. Query fingerprints, identity tie-breakers, ascending/descending ordering and before/after edges are preserved. Durable index encoding is unchanged. Navigation-key encoding is versioned; clients must discard previous-format keys and initialize a fresh watch when upgrading.

The frontend benchmark retains 10,000 or 100,000 visited rows, repeatedly refreshing a 200-row interior range while changing one listing price under number ordering. It excludes fixture generation, initial population, React rendering and activity/viewport projection. The fixture retains the baseline fields to keep object sizes comparable. The new cache keeps an identity index, finds coverage by binary search and compares only incoming rows; producing the complete visited-row array still requires copying array references.

| Cached rows | Before | After |
| --- | ---: | ---: |
| 10,000 | 3.779 ms | 0.136 ms |
| 100,000 | 52.460 ms | 0.388 ms |

The server retains read, scan, byte, concurrency and snapshot budgets. A finite scan may return an empty slice with a `next` key when its scan budget is reached. That key advances past scanned candidates; its returned coverage never includes unscanned rows. A fixed range either returns complete coverage or an explicit capacity error.
