# Replica bootstrap and recovery

On startup, a saved replica can serve reads immediately, while its durable cursor is used to resume Watch. An expired cursor or changed hub generation requires a full replacement snapshot. The old replica remains readable with delayed availability until replacement completes. An interrupted unpublished build is discarded; it is never treated as a durable replica.

## Why a rebuild can take minutes

The snapshot includes catalog, active orders and execution history. Reads request 500 rows per page, so a large history requires many sequential continuation requests. Downloading is only part of the work: snapshot rows build in-memory indexes and write indexed execution history to redb. Watch deliveries arriving during that work are journaled to the same staging database, then replayed before publication. A busy stream can build a substantial catch-up backlog.

Previously, each page download waited for the preceding page's staging transaction, and every journal delivery required a separate replay transaction. Transient errors during page streaming could also discard the whole build. Finalization used two durable transactions.

The production warning `cursor expired or hub restarted; restart Read` explains why a rebuild started. By itself, it does not identify which phase accounts for several minutes. Local storage measurements cannot establish the production network or disk bottleneck.

## Bounded changes

- Stage the first page immediately, then prefetch one bounded batch while another stages. Each batch holds at most eight pages, 4,000 rows or 4 MiB of encoded page data, whichever limit is reached first. Continuations remain sequential. A page that does not fit is carried into the next batch without another request. The pipeline retains at most one staging batch, one prefetch batch and one overflow page; decoded objects and database/index memory have additional overhead. Oversized individual pages fail before staging.
- Replay up to 64 deliveries or 1 MiB of encoded journal data per transaction, preserving delivery order. ACK-only groups advance the checkpoint without rewriting state. A failed group invalidates its unpublished builder; no part of that build can be published. Pre-commit storage failures roll back their transaction, and the previous canonical replica remains available.
- Commit the final checkpoint and remove the journal in one durable transaction. Publish only after finalization succeeds; the storage format and atomic replacement behavior are unchanged.
- Retry transient capacity, unavailable and deadline errors on the same page continuation, including errors after receiving a partial stream. Each attempt has a ten-second timeout, with at most six attempts. Expired continuations and other errors return immediately to session recovery.
- Reset reconnect backoff after a completed bootstrap, as well as after successfully applying a live batch.

Staging transactions remain non-durable until finalization. Background live persistence still commits changes and their recoverable checkpoint together; memory visibility precedes that transaction. No protocol, configuration or public API changes are needed.

## Live updates

After receiving a Watch delivery, the worker drains only additional frames already ready, up to 64 deliveries or 1 MiB of encoded data per transaction. It does not wait on a timer or delay a lone frame to fill a group. A frame exceeding the remaining group budget is kept for the next transaction, so the worker may hold one additional decoded delivery. Changes retain their original order. A stream error or EOF after collected frames is handled after admitting and publishing those frames. Cancellation discards unadmitted buffered frames; accepted background writes drain before choosing the durable reconnect checkpoint.

An empty acknowledgement at the published cursor confirms live availability without a transaction, new view or data notification. A changed cursor is queued for background persistence, even when no entities changed. Checkpoint-only writes open only metadata. Identical active entities retain their shared references and skip row replacement; execution retries still validate immutable contents but do not invalidate history. Actual writes reuse one serialization buffer per transaction. Revision invalidation groups affected books with borrowed keys rather than allocating two separate sets of book identities. Data-change notifications are independent of checkpoint/observation timestamps; availability transitions still wake consumers.

Live publication prepares the RAM root, scoped revisions and an immutable execution tail, reserves bounded background-write capacity, then publishes the whole view. A dedicated `replica-persistence` thread stores accepted deliveries in order. It groups only jobs already available, with no collection timer. Up to 256 queued/unreconciled jobs and 16 MiB of estimated payload are admitted; overload is explicit rather than silently dropping updates. Production transfers ownership of received batches to persistence instead of cloning their payloads.

New history records are immediately queryable from the pending tail, with the same chronological, price, number, range and pagination rules as the disk indexes. Completing a write replaces the durable base and retires only its acknowledged tail. Logical revisions, data notifications and pinned snapshots do not change during this maintenance. Live updates and reads remain available while the database writer is blocked.

Publication recovery retains current, uncertain and prepared states. A prepared view retains its RAM root and scoped revisions; retry does not scan entities. Background completion may refresh its backing database without changing logical content. Ordinary reconnect drains accepted writes before choosing a recoverable cursor. Invalid input fails before enqueue/publication. A background failure marks availability delayed and recovery rebuilds from one consistent database snapshot, even when the cursor happens to be unchanged. Facts and their durable cursor always share one redb transaction.

The live cursor and view timestamp can run ahead of the database checkpoint. Published success acknowledges memory visibility, not fsync. A hard crash may lose the unflushed tail, which upstream replay or a replacement snapshot repairs. Graceful shutdown drains pending writes. Snapshot replacement drains old-generation live persistence before renaming, preserving existing atomic replacement and directory-sync barriers.

Checkpoint timestamps remain attached to immutable views: pinned reads retain the selected time. Availability remains connection status. Wire contracts and stored formats do not change.

An isolated 10,000-order debug benchmark, without network or credentials:

```sh
CARGO_INCREMENTAL=0 cargo test -p market-replica live_update_benchmark -- --ignored --nocapture --test-threads=1
```

One local run measured 1,000 repeated acknowledgements in 0.30 ms, 256 individual foreground updates in 15.25 ms, and four ready groups of 64 in 4.30 ms. Remaining persistence drained in about 12 ms for each case. The earlier synchronous version took about 1.53 s for 256 individual updates. These are local debug measurements, not end-to-end network latency or production guarantees.

## Snapshot replacement and restart

Replacement follows a fixed sequence:

1. Finalize staging with an explicit immediate-durability transaction containing its checkpoint and journal removal.
2. Prepare the immutable view, read its checkpoint/history, and open the parent directory before renaming anything.
3. Rename `building.redb` over `replica.redb` in the same directory. There is no delete-then-rename gap.
4. Retain the new view/database and directory handle, then sync the directory. A failed sync remains a pending barrier.
5. Publish the view and adopt its database handle together, then release old handles outside the view lock.

Failure before rename preserves the canonical old database. After rename, recovery retains the new database lock and retries the directory barrier before publication. Both live writes and another snapshot build must finish that recovery first; they cannot continue writing to the unlinked old database. A successful barrier is not repeated if publication later fails. Existing pinned readers can continue reading the old inode without seeing new data mixed into it.

On restart, the canonical `replica.redb` is authoritative. Redb recovers its committed transactions; state, history and checkpoint are read from one snapshot. Any `building.redb` is discarded, including a finalized build that was never installed. The app never guesses that an uninstalled staging file should replace the canonical database.

Regression tests cover rename and directory-sync failures, retained database locks, live writes after recovery, concurrent readers, and subprocess exits at six boundaries: during building, after finalization, after rename before directory sync, after directory sync before publication, after publication, and after a subsequent memory-first live publication (recovering either the complete earlier or complete newer checkpoint). The subprocess exits skip Rust destructors and redb shutdown. These test process crashes, not power loss. Before the directory barrier completes, power-loss recovery may retain the old or new file name according to filesystem guarantees; the app relies on same-directory atomic rename, redb's durable commit and successful directory sync.

## Cancellation and memory ownership

Writer admission uses an OS lock on the stable `replica.lock` file, acquired before opening the database or cleaning staging files. Database locks follow individual file handles and are insufficient across replacement: another opener could retain the old inode while the canonical name changes. The directory lock stays held across every generation and is released after the writer's database/publication handles, or automatically on process exit. The empty lock file persists and needs no stale-lock deletion; it must not be removed while a writer runs. Reader views do not own this admission lock and can retain older immutable database handles.

The replication worker owns cancellation once, covering page requests, retries, replay and reconnect backoff. Dropping a network future does not stop an already running blocking transaction. On session failure, recovery drains admitted storage jobs before starting another session; shutdown does the same before marking the replica stale. Tracking includes deferred destruction of abandoned snapshot builders, staging databases and prepared snapshots, including results already returned into async prefetch buffers. Their cleanup runs on storage workers rather than async threads. This prevents an old job from retaining a build lease or publishing after a new session starts. Transactions already admitted at cancellation finish atomically; cancellation does not interrupt their disk writes.

Page-to-change conversion and decoded payload destruction run inside the storage worker, once per batch. The staging timer includes payload destruction. Prefetch retains its buffered prefix and continuation across awaits, so cancelling a fetch cannot lose completed pages or turn a retry into false EOF. A single journal write can wait on the staging database while page prefetch continues to be polled; it must finish before the batch result advances the journal position. Replay borrows each decoded batch's changes for its state and storage passes instead of allocating another flattened vector, and stops at the collected journal position without an extra empty read. Execution payloads are not cloned into the in-memory state: only their active-order identities need removal, while the original payload is indexed in storage. Active entities still need owned data in immutable published views. Unpublished staging roots are mutated directly instead of being cloned for every transaction. Any failed batch or interrupted mutation makes the builder unpublishable; it must be discarded before another build begins. Production already restarts failed builds, so there is no full RAM rebuild from staging storage just to discard it. These paths preserve the old canonical database and the exclusive build lease. Live updates retain their persistent transactional clones for rollback.

Journal closure on abandonment is an atomic flag update, without waiting on a journal/database mutex. An admitted push upgrades a weak build lease and retains it through its transaction; a replacement build cannot start until that write releases it. Finalization closes admission and waits for all accepted writes before flushing/removing the journal. Closed journal handles do not retain the exclusive lease. Writes serialize through one reusable encoding buffer per build. Recovery drains writes and deferred cleanup before reusing the staging path.

## Timing logs

INFO logs mark bootstrap start, snapshot download completion and publication. Progress is logged at phase boundaries when at least five seconds have elapsed, with `pages`, `rows`, `snapshot_batches`, `queued`, `replayed`, `backlog`, build timings and `elapsed_ms`. A blocked operation does not emit periodic progress until it returns.

Completion includes `download_ms`, `staging_ms`, `replay_ms`, `finalize_ms` and `publish_ms`. Download and staging overlap: their totals must not be added to estimate wall time. The download duration includes page retries; staging and replay durations measure work inside the storage worker. Overall elapsed time also includes stream setup, journaling and scheduling. New logs do not include authentication or continuation tokens.

Detailed fields split successful build transactions into `build_state_ms` (RAM indexing), `build_db_wait_ms` (database writer admission), `build_db_write_ms` (encoding, row/index updates and checkpoint preparation), and `build_db_commit_ms`. `build_transactions` counts both snapshot staging and replay transactions; completion timings cover both phases. `worker_wait_ms` measures snapshot staging worker queueing. `journal_ms` includes queueing and writer waits for journal writes. These durations can overlap and must not be summed as wall time. Snapshot replacement preparation logs `prepare_ms` (history/checkpoint reads, revision preparation and retained-view expiry) and `rename_ms`. A separate snapshot publication log reports `directory_sync_ms`, `view_switch_ms` and `retire_ms` for old roots/database handles. Retirement remains outside the published-view lock.

Large download time suggests page/transport latency; large staging time suggests index construction or disk work. A growing backlog or large replay time points to catch-up throughput. Large finalization/publication time points to the durable flush or replacement. Watch is not drained during final flush/publication, so this interval should remain short.

## Local replay benchmark

Run without a hub or credentials:

```sh
CARGO_INCREMENTAL=0 cargo test --release --locked -p market-replica bootstrap_ -- --ignored --nocapture --test-threads=1
```

`bootstrap_storage_benchmark` stages 10,000 active orders in 500-row chunks, journals 2,000 one-order deliveries, replays them, and finalizes the replica. `bootstrap_batching_benchmark` compares the previous clone-per-page loop with direct mutation at 500 and 4,000 rows per transaction. It uses 100,000 orders split equally between active orders and executions, validates persisted state, and reports three samples per mode. Timers exclude fixture construction and final publication.

Local optimized-build results and their detailed timings are recorded below. Docker end-to-end gains depend on upstream latency, disk performance and concurrent load.

| Staging mode | Transactions | Median |
| --- | ---: | ---: |
| Previous per-page copy-on-write, 500 rows | 201 | 2.030 s |
| Unique staging roots, 500 rows | 201 | 1.385 s |
| Unique staging roots, 4,000 rows | 26 | 1.196 s |

The combined change reduced staging time by about 41% in this local macOS arm64 release run. The original 10,000-active-order fixture, keeping 500-row transactions in both runs, improved from 132 ms to 80 ms. In the mixed fixture, the remaining database row/index work took about 847–888 ms per optimized run, while database commit work took about 37–38 ms. Full bootstrap also includes download, journaling, finalization and publication; the production log's 333-second rebuild needs a deployed measurement after this change.

An additional allocation regression checks that applying an execution with a 2 MiB metadata payload to RAM does not duplicate that payload. Run it alone because allocation counters are process-wide:

```sh
cargo test -p market-replica --features profile-allocations execution_replay_allocation_regression -- --ignored --nocapture --test-threads=1
```

An allocation regression also loads 10,000 staged orders, injects a partially applied invalid batch, and verifies that aborting allocates less than 128 KiB instead of rebuilding all staged orders. A local optimized run allocated 3,113 bytes for that failure while releasing approximately 29 MB of staged indexes/payloads. Run it in isolation:

```sh
CARGO_INCREMENTAL=0 cargo test --release --locked -p market-replica --features profile-allocations failed_snapshot_batch_allocation_regression -- --ignored --nocapture --test-threads=1
```


## Remaining upstream improvements

Preserving replay cursors across hub restarts, or increasing their retention, could avoid full rebuilds entirely. Larger byte-bounded snapshot pages or bulk streaming could reduce round trips, but require confirming upstream support and limits. Neither is implemented here. First use phase timings from the next deployed rebuild to decide which change is justified.
