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

Staging transactions remain non-durable until finalization. Normal live deliveries still durably commit changes and their checkpoint together. No protocol, configuration or public API changes are needed.

## Cancellation and memory ownership

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
