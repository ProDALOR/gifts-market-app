import assert from "node:assert/strict";
// Shared by disposable hub smoke and the supervised API regression.
const base = process.env.PUBLIC_URL;
if (!base || !process.env.PUBLIC_QUERY)
  throw Error("Run through the supervised smoke harness");
const input = JSON.parse(process.env.PUBLIC_QUERY);
const query = {
  filters: input.filters,
  source: input.side === "bid" ? "bids" : "asks",
  sort: input.sort ?? "price_asc",
  navigation: { type: "head" },
  limit: 4,
};
const leases = new Set();
const socket = new WebSocket(new URL("/ws", base).href.replace(/^http/, "ws"));
async function api(method, body, expected = 200) {
  const response = await fetch(new URL("/orders/query", base), {
    method,
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
    signal: AbortSignal.timeout(5000),
  });
  const data = response.status === 204 ? null : await response.json();
  assert.equal(response.status, expected, JSON.stringify(data));
  return data;
}
async function freeze(query, snapshot = null) {
  const data = await api("POST", {
    ...query,
    view: { kind: "snapshot", id: snapshot },
  });
  leases.add(data.snapshot);
  return data;
}
async function release(snapshot) {
  await api("DELETE", { snapshot }, 204);
  leases.delete(snapshot);
}
async function timeout(work) {
  let timer;
  try {
    return await Promise.race([
      work,
      new Promise((_, reject) => {
        timer = setTimeout(() => reject(Error("Range probe timed out")), 5000);
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}
function receive(id, send) {
  return timeout(
    new Promise((resolve, reject) => {
      const cleanup = () => {
        socket.removeEventListener("message", message);
        socket.removeEventListener("close", closed);
      };
      const closed = () => {
        cleanup();
        reject(Error("Socket closed"));
      };
      const message = (event) => {
        const frame = JSON.parse(event.data);
        if (frame.id !== id) return;
        if (frame.type === "error") {
          cleanup();
          reject(Error(frame.error.message));
        } else if (
          frame.type === "orders.snapshot" ||
          frame.type === "result"
        ) {
          cleanup();
          resolve(frame);
        }
      };
      socket.addEventListener("message", message);
      socket.addEventListener("close", closed);
      send();
    }),
  );
}
const watch = (id, query, range) =>
  receive(id, () =>
    socket.send(
      JSON.stringify({ type: "orders.watch", id, params: { query, range } }),
    ),
  );
const identity = (row) => [row.book.venue_id, row.id];
try {
  await timeout(
    new Promise((resolve, reject) => {
      socket.addEventListener("open", resolve, { once: true });
      socket.addEventListener("error", reject, { once: true });
    }),
  );
  const latest = await api("POST", query);
  assert.equal(latest.snapshot, undefined);
  assert.ok(latest.data.length >= 3, "Seed at least three listings");
  const oneShot = await receive("read", () =>
    socket.send(
      JSON.stringify({
        type: "request",
        id: "read",
        method: "orders.query",
        params: query,
      }),
    ),
  );
  assert.deepEqual(oneShot.data, latest);
  const first = await freeze(query);
  const selected = first.data[1];
  await release(first.snapshot);
  const expired = await api(
    "POST",
    { ...query, view: { kind: "snapshot", id: first.snapshot } },
    409,
  );
  assert.equal(expired.error.code, "restart_read");
  const selection = {
    ...query,
    navigation: { type: "around", key: selected.key },
  };
  const live = await watch("live", selection);
  assert.equal(live.sequence, 0);
  assert.equal(live.reset, undefined);
  assert.ok(
    live.data.some(
      (row) =>
        JSON.stringify(identity(row)) === JSON.stringify(identity(selected)),
    ),
  );
  socket.send(JSON.stringify({ type: "cancel", id: "live" }));
  const exact = await watch("range", selection, live.range);
  assert.deepEqual(exact.data, live.data);
  assert.deepEqual(exact.range, live.range);
  socket.send(JSON.stringify({ type: "cancel", id: "range" }));
  const paused = await freeze(selection);
  const nextPosition = paused.data.at(-1).key;
  const moved = await freeze(
    { ...query, navigation: { type: "around", key: nextPosition } },
    paused.snapshot,
  );
  assert.equal(moved.snapshot, paused.snapshot);
  assert.equal(moved.revision, undefined);
  assert.ok(moved.data.some((row) => row.key === nextPosition));
  await release(paused.snapshot);
  const resumed = await watch("resume", {
    ...query,
    navigation: { type: "around", key: nextPosition },
  });
  assert.ok(resumed.data.some((row) => row.key === nextPosition));
  socket.send(JSON.stringify({ type: "cancel", id: "resume" }));
  console.log(
    "Range probe passed: REST/WS parity, query-bound coverage, frozen reads, release and resume.",
  );
} finally {
  await Promise.allSettled([...leases].map(release));
  socket.close();
}
