Connect to `/ws` on the same host as the HTTP API (`wss` with HTTPS). No Telegram login or hub credentials are needed for public market reads. Browser origins must match the application or its configured CORS allowlist.

Send JSON text messages. `subscribe` starts a live page of asks or bids; collection and currency are required. Use `/catalog` for IDs and `/collections/{id}` for trait choices. Reusing an ID replaces that subscription. IDs contain 1–64 ASCII letters, digits, underscores, dots or hyphens. `unsubscribe` releases the ID without an acknowledgement. Execution history uses HTTP `/history/query`.

`page` replaces the requested page. The frame's `sync` is authoritative over the cached page's freshness. `sync` frames keep orders unchanged. Compare opaque `revision` strings for equality: when the revision changes, relevant HTTP continuation pages may need refreshing even if the head is unchanged. Identical subscriptions share computation.

Use `/orders/query` with the same filters, side, sort and `consistency: live` to follow `next_page_token`. Snapshot HTTP reads expire after 60 seconds of inactivity; an expired snapshot returns `restart_read`. Changing filters or sort starts a new traversal. A page cursor is a replica checkpoint, not a continuation token.

Server WebSocket pings keep the connection alive; browsers reply automatically. Defaults: 16 subscriptions per connection, 64 KiB input JSON, 4 MiB encoded pages, 15-second heartbeats and 35-second idle timeout. Embedded deployments may override these limits. Slow consumers close with code 1013. On disconnection, reconnect with backoff and resubscribe using the same queries; subscription IDs belong to each connection. The test console reconnects only when you click Connect.

An error with an empty ID applies to the connection or malformed command. Setup/query/encoding errors terminate that subscription; replace or resubmit its ID after correcting the cause. Market freshness can remain stale while the last replicated data is still available. Close code 1001 may indicate shutdown; 1011 indicates a subscription service failure.
