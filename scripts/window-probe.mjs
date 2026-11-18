import assert from "node:assert/strict";

// Shared by the disposable hub smoke run and the supervised API regression test.
const base = process.env.PUBLIC_URL;
if (!base || !process.env.PUBLIC_QUERY)
  throw Error(
    "Run this probe through scripts/smoke.py or the supervised API regression test",
  );
const input = JSON.parse(process.env.PUBLIC_QUERY);
const query = {
  filters: input.filters,
  side: input.side ?? "ask",
  sort: input.sort ?? "price_asc",
  anchor: null,
  before: 1,
  after: 2,
};
const leases = new Set();
const socket = new WebSocket(new URL("/ws", base).href.replace(/^http/, "ws"));
async function api(method, body, expected = 200) {
  const response = await fetch(new URL("/orders/window", base), {
    method,
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
    signal: AbortSignal.timeout(5000),
  });
  const value = response.status === 204 ? null : await response.json();
  assert.equal(response.status, expected, JSON.stringify(value));
  return value;
}
async function freeze(query, snapshot = null) {
  const value = await api("POST", { query, snapshot });
  leases.add(value.snapshot);
  return value;
}
async function release(snapshot) {
  await api("DELETE", { snapshot }, 204);
  leases.delete(snapshot);
}
async function withTimeout(work) {
  let timer;
  try {
    return await Promise.race([
      work,
      new Promise((_, reject) => {
        timer = setTimeout(() => reject(Error("Window probe timed out")), 5000);
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}
function subscribe(id, query) {
  return withTimeout(
    new Promise((resolve, reject) => {
      const cleanup = () => {
        socket.removeEventListener("message", receive);
        socket.removeEventListener("close", closed);
      };
      const closed = () => {
        cleanup();
        reject(Error("Window socket closed"));
      };
      const receive = (event) => {
        const frame = JSON.parse(event.data);
        if (frame.type === "error" && (!frame.id || frame.id === id)) {
          cleanup();
          reject(Error(frame.error.message));
        } else if (frame.type === "window" && frame.id === id) {
          cleanup();
          resolve(frame.window);
        }
      };
      socket.addEventListener("message", receive);
      socket.addEventListener("close", closed);
      socket.send(JSON.stringify({ type: "subscribe_window", id, query }));
    }),
  );
}
const identity = (row) => [row.book.venue_id, row.id];
const anchored = (window) => {
  const row = window.data.find((row) => row.position === window.anchor);
  assert.ok(row, "The anchor must identify a row in the atomic window");
  return row;
};
try {
  await withTimeout(
    new Promise((resolve, reject) => {
      socket.addEventListener("open", resolve, { once: true });
      socket.addEventListener("error", reject, { once: true });
    }),
  );
  const first = await freeze(query);
  assert.ok(first.window.data.length >= 3, "Seed at least three listings");
  const selected = first.window.data[1];
  const selection = { ...query, anchor: selected.position };
  // Position tokens outlive the snapshot lease that produced them.
  await release(first.snapshot);
  const expired = await api(
    "POST",
    { query: selection, snapshot: first.snapshot },
    409,
  );
  assert.equal(expired.error.code, "restart_read");
  const live = await subscribe("probe.live", selection);
  assert.deepEqual(identity(anchored(live)), identity(selected));
  assert.equal(anchored(live).price, selected.price);
  socket.send(JSON.stringify({ type: "cancel", id: "probe.live" }));

  const paused = await freeze({ ...query, anchor: live.anchor });
  assert.deepEqual(identity(anchored(paused.window)), identity(selected));
  const nextAnchor = paused.window.data.at(-1).position;
  const moved = await freeze({ ...query, anchor: nextAnchor }, paused.snapshot);
  assert.equal(moved.snapshot, paused.snapshot);
  assert.equal(moved.revision, paused.revision);
  assert.equal(moved.window.anchor, nextAnchor);
  await release(paused.snapshot);
  const resumed = await subscribe("probe.resume", {
    ...query,
    anchor: moved.window.anchor,
  });
  assert.deepEqual(
    identity(anchored(resumed)),
    identity(anchored(moved.window)),
  );
  socket.send(JSON.stringify({ type: "cancel", id: "probe.resume" }));
  console.log(
    "Window probe passed: anchored live views, frozen navigation, lease release and resume.",
  );
} finally {
  await Promise.allSettled([...leases].map(release));
  socket.close();
}
