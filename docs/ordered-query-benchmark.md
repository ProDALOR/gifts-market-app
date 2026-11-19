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

### Ordered Base64 text encoding

A further run compares the preceding compact hexadecimal release binary with version 3, using the same fixture and sampling method:

| Operation | Compact hex | Ordered Base64 |
| --- | ---: | ---: |
| Ordering-key bytes for 200 rows | 33,200 | 22,200 |
| Head, 200 rows, including keys | 117.71 µs | 111.17 µs |
| Fixed range, 200 rows, including keys | 165.08 µs | 157.25 µs |
| Native head, before encoding | 63.42 µs | 61.75 µs |
| Native range, before encoding | 106.67 µs | 107.92 µs |

Key text shrinks by 33.1%, retaining the same binary fields. Unpadded Base64 uses the ASCII-ordered, URL-unreserved alphabet `0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ_abcdefghijklmnopqrstuvwxyz~`, so ordinary string comparison still follows native order. Repeated-character syntax such as `0{10}` would need a custom comparator; it is not used. Native keys own one Serde string converter shared by rows, range bounds and continuations. Decoding rejects old versions, padding, noncanonical trailing bits and invalid ranks. Regression tests cover every sort, ascending/descending ties, history, prefix/Unicode identities, before/at/after edges and JSON roundtrips. Deploy frontend/backend together and restart watches when upgrading.

### Filter-independent positions

Version 4 removes the 32-byte query fingerprint from every key. A three-byte header retains the format version, sort/direction and selected source, including active/history storage. Ranks, identity tie-breakers and before/at/after edges are unchanged. The following run compares the preceding version 3 release binary with version 4 using the same fixture:

| Operation | With fingerprint | Without fingerprint |
| --- | ---: | ---: |
| Ordering-key bytes for 200 rows | 22,200 | 13,600 |
| Head, 200 rows, including keys | 109.63 µs | 102.33 µs |
| Fixed range, 200 rows, including keys | 158.67 µs | 144.75 µs |
| Native head, before encoding | 61.96 µs | 61.54 µs |
| Native range, before encoding | 107.29 µs | 105.08 µs |

Key text shrinks by a further 38.7%. Timing remains a local microbenchmark. Positions can be reused across filters and execution periods with compatible sort/source; the supplied query always determines the returned rows. Price ranks use the new query's currency units without conversion or validation of the original currency. Internal generic read continuations retain their existing query validation. The key-only fingerprint type and hexadecimal parsing helpers are removed. Regression tests cover reuse in finite reads and fixed ranges for every sort, identical keys across filters, REST-to-WebSocket reuse, sort/direction/source mismatches and canonical encoding. Clients must discard version 3 keys and initialize fresh watches when upgrading.

### Removing format-version markers

The current encoding packs sort/direction and source into one header byte, removing the format-version byte and a separate source byte. It retains canonical decoding, sort/source validation, identity tie-breakers and boundary ordering. For the short `v`/`x` identity fixture, keys shrink from 14 to 11 characters; price keys shrink from 52 to 50. The tables above describe their historical formats. Catalog content tokens are separate equality-only validators and now carry 22 Base64url characters without a format-version prefix. Discard previously encoded order keys and restart watches on upgrade; older catalog ETags automatically revalidate.

## Visited-listing cache

The frontend benchmark retains 10,000 or 100,000 visited rows, repeatedly refreshing a 200-row interior range while changing one listing price under number ordering. It excludes fixture generation, initial population, React rendering and activity/viewport projection. The new cache keeps an identity index, finds coverage by binary search and compares only incoming rows; producing the complete visited-row array still requires copying array references.

| Cached rows | Before | After |
| --- | ---: | ---: |
| 10,000 | 3.779 ms | 0.136 ms |
| 100,000 | 52.460 ms | 0.388 ms |

The server retains read, scan, byte, concurrency and snapshot budgets. A finite scan may return an empty slice with a `next` key when its scan budget is reached. That key advances past scanned candidates; its returned coverage never includes unscanned rows. A fixed range either returns complete coverage or an explicit capacity error.
