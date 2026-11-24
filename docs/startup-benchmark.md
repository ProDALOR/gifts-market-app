# Replica startup restore

Startup reads `CURRENT` once and decodes each row once. The persisted identity must match the decoded payload before an insert-only restore builds the active-order indexes. The fast path relies on the database's unique identity keys and a new, empty state; live updates continue to use validated replacement logic. Legacy executed rows in `CURRENT` stay excluded from active books.

Order locations, all three ranked indexes and attribute memberships share one `Arc<Key>` per order. Comparisons and pagination still use identity values. Attribute updates compare borrowed facts, without temporary sets or cloned attribute/value keys on membership lookups. New orders reuse existing partition keys before checking the book directory. Payloads and pinned snapshots keep their existing copy-on-write behavior.

Clean database opens validate source identity, schema types and the execution-rank marker in a read transaction. They perform no application write transaction and do not rescan execution history. redb still opens the database and performs its own recovery when necessary. Only an empty database uses transactional initialization. Saved databases with missing service identity or required data tables fail without altering their checkpoint. Missing execution-rank tables are rebuilt even if the marker remains; missing staging journals are recreated without rescanning history. Rank rebuilds atomically replace the whole derived table, remove stale ranks and verify each execution payload against its stored identity. Invalid history rolls the entire repair back. Checkpoints, journal replay, snapshot replacement and crash recovery retain their existing formats and lifecycle. No storage reset is required.

## Reproduce

```sh
CARGO_INCREMENTAL=0 cargo test -p market-replica --lib --locked --offline \
  storage::startup_benchmark::startup_restore_benchmark -- --ignored --exact --nocapture --test-threads=1
CARGO_INCREMENTAL=0 cargo test -p market-replica --lib --features profile-allocations --locked --offline \
  storage::startup_benchmark::startup_restore_benchmark -- --ignored --exact --nocapture --test-threads=1
MARKET_BENCH_ITERATIONS=4 CARGO_INCREMENTAL=0 cargo test -p market-replica --lib --locked --offline \
  benchmarks::orderbooks_benchmark -- --ignored --exact --nocapture --test-threads=1
```

Run allocation measurements separately from latency measurements. Both benchmarks use disposable databases and need no hub, Telegram configuration or existing data directory. The orderbook harness uses typed internal read options for time/number sorts, preserving the generic upstream protocol.

## Initial optimization measurements

Recorded on 2026-11-24 using an unoptimized debug build on the same local machine. These measurements compare implementations, not production or cold-disk startup latency. The startup fixture contains 20,000 active orders plus 29 metadata records. The database is populated before timing, and decode-only traversal precedes index restoration in each sample, warming pages. `writer_open` measures the complete `ReplicaWriter::open`, including database open and index restoration. Fixture creation and teardown are excluded.

Before and after binaries were built separately and copied out of Cargo's shared output directory before timing. The baseline uses commit `1c3e7d4` with the same benchmark harness. Two processes per version ran sequentially in reversed order; each process measures three opens. No compilation or project checks ran during timing. The existing local app remained running. Allocation counts use a separate instrumented build; retained bytes measure requested live allocations, not RSS or allocator overhead.

| Phase | Before | After | Change |
| --- | ---: | ---: | ---: |
| `database_open` | 74.72 ms | 69.35 ms | -7.2% |
| `decode_only` | 106.95 ms | 108.19 ms | +1.2% |
| `restore_state` | 338.71 ms | 271.91 ms | -19.7% |
| `writer_open` | 414.04 ms | 334.88 ms | -19.1% |

Restore allocations fell from **918,981 to 443,250** (-51.8%). Allocated bytes fell from **80,904,398 to 56,502,776** (-30.2%); retained state bytes fell from **56,072,622 to 49,835,230** (-11.1%). Decode-only allocations and revision-index allocations were unchanged in this initial comparison (before the review follow-up below). Database-open allocations fell from 4,706 to 4,400.

The read/update benchmark uses 40,000 orders, four warmups, nine samples of four operations and two processes per version, reversing execution order. Values below are medians of the process medians. Read rows are per 200-record page; mutation rows are per 100-record batch with the original snapshot pinned.

| Case | Before | After | Change |
| --- | ---: | ---: | ---: |
| `price_multi` | 867.67 µs | 862.43 µs | -0.6% |
| `price_single` | 768.51 µs | 778.46 µs | +1.3% |
| `time_desc_multi` | 734.66 µs | 775.18 µs | +5.5% |
| `number_desc_multi` | 753.70 µs | 729.83 µs | -3.2% |
| `price_sparse` | 1606.16 µs | 1551.06 µs | -3.4% |
| `identity_control` | 1422.40 µs | 1512.11 µs | +6.3% |
| `identity_orders` | 591.59 µs | 613.42 µs | +3.7% |
| `identity_keys` | 1392.36 µs | 1373.13 µs | -1.4% |
| `identity_mixed` | 510.58 µs | 513.54 µs | +0.6% |
| `metadata_options` | 23.27 µs | 22.94 µs | -1.4% |
| `price_continuation` | 866.38 µs | 859.53 µs | -0.8% |
| `update_100_price` | 4567.97 µs | 3105.95 µs | -32.0% |
| `update_100_metadata` | 3620.93 µs | 2554.75 µs | -29.4% |
| `update_100_partition` | 6349.40 µs | 4051.36 µs | -36.2% |
| `remove_100` | 5325.51 µs | 3109.84 µs | -41.6% |
| `persist_update_100` | 684.20 µs | 692.55 µs | +1.2% |

Reads varied between -3.4% and +6.3% in these debug runs; this is not evidence of a general read speedup. Copy-on-write update batches improved by 29–42%. Persistent same-record updates were essentially unchanged. Use release-mode measurements on the deployment host before extrapolating to production startup or throughput.

## Review follow-up

The review added regression coverage for missing service identity/data tables, stale ranks, invalid execution identities, repair rollback, journal-only repairs, incompatible table types and a pinned restore across a concurrent commit. The clean-validation writer-contention test now has a timeout and releases its writer before asserting, so a regression cannot hang the suite.

Restore continues to read current records and checkpoint metadata from the same pinned read transaction. `ReplicaWriter` acquires its directory lock before opening storage; the database is not exposed until schema inspection/repair completes. No additional startup worker or lock is needed for consistency.

Option-group revision restore now allocates a group name only when that group is first encountered. Trait indexing walks borrowed attributes directly, avoiding a second map lookup for each fact. The data format and insert-only restore's unique, identity-checked input contract are unchanged.

The follow-up allocation run kept active-state restore at 443,250 allocations, while revision-index restoration dropped from 21 allocations to 5 on the same fixture. A three-sample debug timing check against the saved pre-review binary measured restore at 249.17 → 249.22 ms and full writer open at 317.02 → 311.99 ms. Treat these as a regression check, not an additional claimed speedup.

## Startup cancellation and shutdown

The app registers SIGINT/SIGTERM handling before polling startup. A shared cancellation tree covers replica open, service startup and media-cache initialization. Signals request cancellation and continue awaiting the same startup/service future, preserving one cleanup path instead of dropping database work and starting a second shutdown routine.

Replica open uses the bounded storage executor. Cancellation is checked before taking resources, between restored rows and revision entries, between history-rank records, and before committing initialization/repair. Dropping the async opener cancels its child token. Abandoned results retain tracked off-thread destruction, and queued cleanup retains the executor until the value and database handles are destroyed. A native redb open, commit/rollback, or filesystem call cannot be forcibly interrupted; cancellation takes effect at the next safe boundary. Directory locks stay owned until cleanup completes.

Media initialization checks cancellation between files, hashes and cache removals. Existing media write/rename consistency is unchanged. The next open rescans any partially cleaned cache. Cancelled rank repair rolls back the entire transaction; cancelled current-state restore discards only its unpublished memory indexes. Reopening still uses the canonical durable checkpoint, discards interrupted staging files and preserves snapshot-replacement crash recovery.

Startup failures after service creation explicitly stop and drain supervised actors/workers and shared reads. The supervisor also cancels on drop as a fallback. Shutdown drains accepted persistence work and releases a replication worker that was never started. Explicitly pinned library readers still own their snapshots/database handles until dropped; shutdown does not invalidate a snapshot by forcibly closing its database.

Regression tests cover early signals, waiting for cleanup, preservation of shutdown errors, cancellation/abandonment while the directory lock is held, interrupted rank repair and state restore, retry after media scanning stops, startup configuration failure, shutdown before replication starts, and queued cleanup after its last async owner disappears.
