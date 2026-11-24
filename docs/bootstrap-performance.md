# Bootstrap transfer and storage

The app retains complete history. Snapshot rows count catalog entries, feeds, active orders and archived executions; they are not a unique-gift count. Hub ordering/deletion records are not replica entities. Executions remain disk-backed rather than entering active orderbook indexes.

## Transport and recovery

`OpenSnapshot` captures one frozen RAM/history view and returns an authenticated session token plus its Watch checkpoint. `ReadSnapshotPart` streams current entities and execution history separately from this same view. The client uses three independently created gRPC channels: live/control, current snapshot and history snapshot. Cloning a channel would share the underlying connection and does not provide this isolation. The two part streams share one retained-view quota slot, renew its idle deadline while working, and never incorporate later writes/backfills.

Each part has a one-page producer channel and one bounded calculation in flight. Each page respects the existing 500-row/byte limits and carries the session cursor. A nonempty continuation is a part-local progress marker, not a `Read` token. An empty marker ends that part. The client validates part membership, stable checkpoints, final markers and clean EOF from **both** streams. Only then can staging and journal replay publish the completed dataset. Watch opens after the first page; replay remains ordered through a single journal even when snapshot pages interleave.

Each part can start once. The server closes the session after both finish; cancellation/failure closes the entire session. Explicit `CloseSnapshot` handles setup cancellation and unstarted parts, and idle expiry covers disconnected clients that never start streaming. Destruction of retained roots/database snapshots runs off the async event loop. Active calculations retain their leases and permits until they actually finish.

For rolling upgrades, `UNIMPLEMENTED` during `OpenSnapshot` falls back directly to paged `Read(SNAPSHOT)`. A read-only probe of the deployed hub on 2026-11-24 confirmed it still requires this path. The intermediate experimental full-stream `Snapshot` RPC and its separate producer/lease implementation have been removed from both repositories. Other errors, including failures starting either coordinated part, never fall back into a different snapshot. Failed/cancelled builds remain unpublished; crash recovery, journal replay and atomic database replacement remain authoritative. Deploy the hub first to enable coordinated exports; an app-only upgrade still uses its supported fallback.

## Execution isolation

The hub keeps its single authoritative writer on its Actix Arbiter. Interactive page/Books reads and Watch projections each retain the existing `MARKET_READ_WORKERS` capacity (default two) and now use independent blocking pools. Bulk snapshot exports have two dedicated permits/threads, shared across all sessions and both parts. Backpressure releases calculation permits between pages. Snapshot admission and execution have separate operation histograms (`snapshot_wait`, `snapshot_work`); `market_snapshot_workers_busy` / `market_snapshot_workers_limit` expose bulk occupancy without hiding interactive capacity.

The app has one serial storage pool for staging, journal writes, replay, replacement and cleanup. Maintenance runs concurrently with the replication future, so queuing an expiry job behind disk work cannot suspend prefetch or Watch journaling. Ordinary query execution uses up to four dedicated threads and valuation preparation up to two, capped by CPU count and the configured read limit. Both share the existing total read budget; cancellation keeps permits until work exits. Shutdown drains tracked work before closing the pools.

Valuation reports coalesce by canonical query **before** read admission and snapshot pinning. Floor preparations additionally coalesce per collection/currency outside the global cache lock. Active entries cannot be evicted, and an older pinned snapshot can compute a private result but cannot overwrite a newer cached generation. Summary/trait refresh deadlines and the one-minute policy remain shared. API and bot retain the same actors, replica and caches.

The bot and its invocation scheduler run on a dedicated supervised thread/current-thread Tokio runtime. Failures propagate to the existing optional-worker supervisor; shutdown cancels and joins the thread after invocation cleanup. Application and bot event loops report delays above 100 ms. Blocking-job debug logs separate queue wait from execution time by pool. Isolation bounds contention; it does not remove shared CPU/disk limits or promise faster storage on a single-core container.

The app and hub proto copies now match exactly. Time/number ranking uses native replica read options, preserving persistent sort codes without adding unsupported upstream enum values.

## Journal and indexes

Ready Watch deliveries share a journal transaction without a collection timer. A group holds at most 64 deliveries and less than twice the 1 MiB delivery budget (one overflow delivery is included); each delivery remains independently bounded and retains its own cursor/position. No successful transaction is counted before commit. Cancellation drains admitted storage work before another build can acquire its lease. Replay keeps its independent count/byte limits.

Unpublished staging buffers derived history rank/filter entries and writes them in key order within the same transaction. Payload and terminal-identity validation preserve delivery order. Estimated derived-key bytes flush at 8 MiB plus one execution's keys; vector spare capacity and the existing per-execution key preparation are additional temporary allocations. Published live writes keep incremental indexing. Existing persisted tables, keys, read results and cursor ordering remain unchanged. Initial snapshot finalization stays durable; normal live publication uses the background persistence path described in [replica recovery](replica-bootstrap.md).

Progress/completion logs split catalog, feed, ask, bid and execution rows; include encoded snapshot bytes and rows/second; and distinguish journal transactions, deliveries (`queued`), changes and encoded bytes. History payload and secondary-index times are subsets of database write time. Fetch, staging and journal durations overlap and must not be added to obtain elapsed time. Wire byte counts exclude HTTP/2 framing/compression.

## Local release measurements

2026-11-24, three samples per case, temporary redb databases, ordinary allocator. These are local staging microbenchmarks, not projected production bootstrap durations. The development bot and other checks were running, so timings have contention/noise. Transaction and row counts are deterministic.

| Workload | Previous behavior | Batched/sorted behavior |
|---|---:|---:|
| 2,000 Watch deliveries: journal transactions | 2,000 | 32 |
| Same journal workload: median | 52.01 ms | 32.10 ms |
| 12,000 executions: staging median | 175.17 ms | 175.98 ms |
| 120,000 executions, model/number/backdrop/pattern indexes: staging median | 2.919 s | 2.384 s |

A follow-up run after execution isolation measured 39.34 ms versus 15.91 ms for the same 2,000-delivery journal workload (2,000 versus 32 transactions). The 120,000-execution comparison measured 2.501 s versus 2.245 s, preserving the same payload/index counts. Samples were 2.439/2.501/2.656 s incremental and 2.171/2.245/2.257 s sorted. These runs overlapped verification and the disposable smoke test; compare implementations within each run rather than treating differences across runs as an isolation speedup.

The smaller history case showed no overall improvement; the larger case showed a lower median. Its incremental samples were 2.485/2.919/4.084 s, sorted samples 2.337/2.384/2.464 s. Both produced 120,000 execution payloads, 840,000 filter memberships and 360,000 rank memberships. Differential tests compare every derived key, including duplicate execution retries.

Reproduce journal measurements:

```sh
cargo test --release --locked -p market-replica --lib \
  bootstrap_benchmark::journal_batching_benchmark -- --ignored --exact --test-threads=1 --nocapture
```

Reproduce the larger history comparison:

```sh
cargo test --release --locked -p market-replica --lib \
  storage::tests::history_index_batching_benchmark -- --ignored --exact --test-threads=1 --nocapture
```

Tests cover frozen coordinated current/history export and Watch continuity, physical connection isolation, view-capacity cleanup after cancellation, terminal-frame and part/checkpoint validation, old-hub fallback, batching/replay positions, counters, derived-index equivalence and existing process-crash replacement boundaries. Execution regressions occupy every bulk/preparation worker and verify interactive/Watch work still runs, assert serial storage execution, coalesce concurrent reports, reject stale cache replacement and exercise bot thread shutdown/panic propagation. The disposable real-hub smoke run additionally verifies the app consumes the stream and remains usable across hub restart.

Validation passed: hub client/replica unit and crash-recovery tests, market API integration tests, core hub unit/protocol tests, the real-hub REST/WebSocket/browser/restart smoke, the all-feature app check and the hub frontend production build. Hub Clippy is warning-free. The former eight-argument slice helper has been removed; order queries use one range reader without a lint exception. The smoke harness now uses half-open response ranges, checks exact prices through accessible labels, forces diagnostic logging and runs disposable apps outside the repository so they cannot auto-discover real account exports.
