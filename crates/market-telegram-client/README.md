# market-telegram-client

Authenticated Telegram operations over Grammers. Owns credentials, session locking/persistence, the sender connection, native profile-gift mapping and media downloads. Update dispatch and market commands live in `market-telegram` and `market-telegram-bot`.

`Connection::open` requires a shared `market-invocations::Handle`; it never creates a private scheduler. Closing a connection cancels that generation, joins the sender and drains session writes before releasing storage. The caller retains the invocation runtime across reconnects so rate state survives. Grammers protocol maintenance, migration and update catch-up remain library-owned.

Explicit application calls use the common executor: account concurrency, per-operation groups, peer message pacing, full account flood waits and peer slow-mode waits. Reads and idempotent configuration can retry; message retries preserve the original random ID. Bot authorization is a single attempt. SDK errors are sanitized before reaching application handlers.

`TelegramActions` exposes native operations, with `GiftSource` scoped to the incoming peer. `TelegramGifts::page_operation(offset, limit)` also returns an unsubmitted typed operation for service adapters. Map its native page/error types with `InvocationExt::map`, then submit it exactly once to the shared scheduler. No service needs TL requests or credentials to value a page. The existing bot retains its bounded inventory collector until a separate portfolio integration step.

Document lookups coalesce across thumbnail/TGS misses, retain bounded short-lived references, and refresh expired references once. A download retry starts a fresh size-bounded iterator because Grammers consumes iterator state on RPC failure. Asset decoding/validation runs after the invocation releases its rate slot. Disk caching and HTTP serving remain in `market-media` and `market-api`.
