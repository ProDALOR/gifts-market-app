# Replica bootstrap and recovery

On startup, a saved replica can serve reads immediately, while its durable cursor is used to resume Watch. An expired cursor or changed hub generation requires a full replacement snapshot. The old replica remains readable with delayed availability until replacement completes. An interrupted unpublished build is discarded; it is never treated as a durable replica.

## Why a rebuild can take minutes

The snapshot includes catalog, active orders and execution history. Reads request 500 rows per page, so a large history requires many sequential continuation requests. Downloading is only part of the work: each page builds in-memory indexes and writes indexed execution history to redb. Watch deliveries arriving during that work are journaled to the same staging database, then replayed before publication. A busy stream can build a substantial catch-up backlog.

Previously, each page download waited for the preceding page's staging transaction, and every journal delivery required a separate replay transaction. Transient errors during page streaming could also discard the whole build. Finalization used two durable transactions.

The production warning `cursor expired or hub restarted; restart Read` explains why a rebuild started. By itself, it does not identify which phase accounts for several minutes. Local storage measurements cannot establish the production network or disk bottleneck.

## Bounded changes

- Fetch one next page while the current page is staged. Continuations remain sequential; this adds at most one downloaded page to the pipeline.
- Replay up to 64 deliveries or 1 MiB of encoded journal data per transaction, preserving delivery order. ACK-only groups advance the checkpoint without rewriting state. Failed groups leave both staged state and storage unchanged.
- Commit the final checkpoint and remove the journal in one durable transaction. Publish only after finalization succeeds; the storage format and atomic replacement behavior are unchanged.
- Retry transient capacity, unavailable and deadline errors on the same page continuation, including errors after receiving a partial stream. Each attempt has a ten-second timeout, with at most six attempts. Expired continuations and other errors return immediately to session recovery.
- Reset reconnect backoff after a completed bootstrap, as well as after successfully applying a live batch.

Staging transactions remain non-durable until finalization. Live updates still durably commit changes and their checkpoint together. No protocol, configuration or public API changes are needed.

## Live updates

After receiving a Watch delivery, the worker drains only additional frames already ready, up to 64 deliveries or 1 MiB of encoded data per transaction. It does not wait on a timer or delay a lone frame to fill a group. A frame exceeding the remaining group budget is kept for the next transaction, so the worker may hold one additional decoded delivery. Changes retain their original order. A stream error or EOF after collected frames is handled after committing and publishing those frames; cancellation discards uncommitted buffered frames, which are replayed from the published checkpoint on reconnect.

An empty acknowledgement at the published cursor confirms live availability without a transaction, new view or data notification. A changed cursor still commits durably, even when no entities changed. Checkpoint-only writes open only metadata. Identical active entities retain their shared references and skip row replacement; execution retries still validate immutable contents but do not invalidate history. Actual writes reuse one serialization buffer per transaction. Revision invalidation groups affected books with borrowed keys rather than allocating two separate sets of book identities. Data-change notifications are independent of checkpoint/observation timestamps; availability transitions still wake consumers.

Publication recovery has three internal states: current, uncertain, and prepared. A prepared publication retains one shared immutable view, including its database snapshot and scoped revisions. Retrying publication reuses that exact view; it does not scan entities or invalidate the whole dataset. A repeated cursor cannot bypass a pending publication. An acknowledgement or another direct writer call completes pending recovery before applying more data. Delayed availability is preserved until Watch acknowledges the connection.

| Outcome | Recovery | Entity scan |
| --- | --- | --- |
| Ordinary reconnect | Compare the durable and published checkpoints | No |
| Validation/storage error before commit is attempted | Keep the published view; the transaction was rolled back | No |
| Committed update with a prepared view, publication fails | Retry the retained view and its original scoped revisions | No |
| Commit result is uncertain, or view preparation fails after commit | Rebuild from one consistent database snapshot and invalidate dependencies | Yes |

The fallback reads state, checkpoint metadata and history through the same read transaction. It remains necessary for uncertain commits, even when cursor and timestamp happen to equal the old values. Successful prepared retries adopt the view's backing database together with the view, so a failed snapshot publication cannot leave later writes targeting the old database. A fully prepared bootstrap publication is retried without downloading another snapshot; subsequent transient Watch setup failures resume its checkpoint. Old roots and database handles are released outside the published-view lock. The persisted format and hub protocol are unchanged.

The outer batch vector is reused within each connection. The storage job drops decoded payloads and returns the empty allocation after publication; cancellation continues to track that returned ownership. Pending read-ahead frames belong only to their connection. No read-ahead checkpoint is saved until its group is admitted and committed. Tests cancel both before admission and while an admitted write is blocked, then verify replay begins after the last committed group. A separate transport regression injects a publication failure with a buffered tail and checks that reconnect restores history and receives the remaining frames without a hub snapshot.

Checkpoint timestamps live in immutable views. Pinned and retained reads report the time of the selected view, even if a newer view advances the status channel while the read completes. Availability remains a connection status.

An isolated benchmark compares 256 individual update commits with four groups of 64 and measures repeated acknowledgements. It uses a 10,000-order fixture and no hub:

```sh
cargo test -p market-replica live_update_benchmark -- --ignored --nocapture --test-threads=1
cargo test -p market-replica --features profile-allocations live_update_benchmark -- --ignored --nocapture --test-threads=1
cargo test -p market-replica --features profile-allocations live_group_buffer_allocation_benchmark -- --ignored --nocapture --test-threads=1
```

One local debug run with allocation instrumentation reported zero allocations for 1,000 repeated acknowledgements, taking about 0.33 ms inside the writer. Applying 256 real updates individually took 1.53 s; applying them in four groups took 31.7 ms. This is an isolated storage/writer measurement, not end-to-end network processing. These synthetic measurements do not predict production grouping: a sparse stream still commits each changed delivery immediately.

The reusable collector separately measured zero allocations and reallocations while grouping 1,024 already-decoded frames into 16 groups with preallocated capacity. This excludes decoding, initial buffer allocation and storage-task scheduling; frame payloads are still released as groups complete.

## Snapshot replacement and restart

Replacement follows a fixed sequence:

1. Finalize staging with an explicit immediate-durability transaction containing its checkpoint and journal removal.
2. Prepare the immutable view, read its checkpoint/history, and open the parent directory before renaming anything.
3. Rename `building.redb` over `replica.redb` in the same directory. There is no delete-then-rename gap.
4. Retain the new view/database and directory handle, then sync the directory. A failed sync remains a pending barrier.
5. Publish the view and adopt its database handle together, then release old handles outside the view lock.

Failure before rename preserves the canonical old database. After rename, recovery retains the new database lock and retries the directory barrier before publication. Both live writes and another snapshot build must finish that recovery first; they cannot continue writing to the unlinked old database. A successful barrier is not repeated if publication later fails. Existing pinned readers can continue reading the old inode without seeing new data mixed into it.

On restart, the canonical `replica.redb` is authoritative. Redb recovers its committed transactions; state, history and checkpoint are read from one snapshot. Any `building.redb` is discarded, including a finalized build that was never installed. The app never guesses that an uninstalled staging file should replace the canonical database.

Regression tests cover rename and directory-sync failures, retained database locks, live writes after recovery, concurrent readers, and subprocess exits at six boundaries: during building, after finalization, after rename before directory sync, after directory sync before publication, after publication, and after a subsequent live commit. The subprocess exits skip Rust destructors and redb shutdown. These test process crashes, not power loss. Before the directory barrier completes, power-loss recovery may retain the old or new file name according to filesystem guarantees; the app relies on same-directory atomic rename, redb's durable commit and successful directory sync.

## Cancellation and memory ownership

Writer admission uses an OS lock on the stable `replica.lock` file, acquired before opening the database or cleaning staging files. Database locks follow individual file handles and are insufficient across replacement: another opener could retain the old inode while the canonical name changes. The directory lock stays held across every generation and is released after the writer's database/publication handles, or automatically on process exit. The empty lock file persists and needs no stale-lock deletion; it must not be removed while a writer runs. Reader views do not own this admission lock and can retain older immutable database handles.

The replication worker owns cancellation once, covering page requests, retries, replay and reconnect backoff. Dropping a network future does not stop an already running blocking transaction. On session failure, recovery drains admitted storage jobs before starting another session; shutdown does the same before marking the replica stale. Tracking includes destruction of abandoned job results, which may still own the snapshot builder, staging database or prepared snapshot. This prevents an old job from retaining a build lease or publishing after a new session starts. Transactions already admitted at cancellation finish atomically; cancellation does not interrupt their disk writes.

Only one continuation page is prefetched. Page-to-change conversion runs inside the storage worker. Replay borrows each decoded batch's changes for its state and storage passes instead of allocating another flattened vector, and stops at the collected journal position without an extra empty read. Execution payloads are not cloned into the in-memory state: only their active-order identities need removal, while the original payload is indexed in storage. Active entities still need owned data in immutable published views; the transactional state clone uses persistent indexes and remains necessary for rollback on invalid input.

## Timing logs

INFO logs mark bootstrap start, snapshot download completion and publication. Progress is logged at phase boundaries when at least five seconds have elapsed, with `pages`, `rows`, `queued`, `replayed`, `backlog` and `elapsed_ms`. A blocked operation does not emit periodic progress until it returns.

Completion includes `download_ms`, `staging_ms`, `replay_ms`, `finalize_ms` and `publish_ms`. Download and staging overlap: their totals must not be added to estimate wall time. The download duration includes page retries; staging and replay durations measure work inside the storage worker. Overall elapsed time also includes stream setup, journaling and scheduling. New logs do not include authentication or continuation tokens.

Large download time suggests page/transport latency; large staging time suggests index construction or disk work. A growing backlog or large replay time points to catch-up throughput. Large finalization/publication time points to the durable flush or replacement. Watch is not drained during final flush/publication, so this interval should remain short.

## Local replay benchmark

Run without a hub or credentials:

```sh
cargo test -p market-replica bootstrap_storage_benchmark -- --ignored --nocapture
```

The fixture stages 10,000 active orders in 500-row chunks, journals 2,000 one-order deliveries, replays them, and finalizes the replica. These are single local debug-build measurements, not a production throughput guarantee or a network benchmark.

| Phase | Before | After |
| --- | ---: | ---: |
| Stage snapshot | 580 ms | 560 ms |
| Write journal | 468 ms | 507 ms |
| Replay journal | 965 ms | 125 ms |
| Finalize and publish | 41 ms | 32 ms |

Grouped replay was about 7.7 times faster in this fixture. Page overlap needs measurements against the deployed hub; this storage benchmark cannot quantify its benefit.

An additional allocation regression checks that applying an execution with a 2 MiB metadata payload to RAM does not duplicate that payload. Run it alone because allocation counters are process-wide:

```sh
cargo test -p market-replica --features profile-allocations execution_replay_allocation_regression -- --ignored --nocapture --test-threads=1
```

## Remaining upstream improvements

Preserving replay cursors across hub restarts, or increasing their retention, could avoid full rebuilds entirely. Larger byte-bounded snapshot pages or bulk streaming could reduce round trips, but require confirming upstream support and limits. Neither is implemented here. First use phase timings from the next deployed rebuild to decide which change is justified.
