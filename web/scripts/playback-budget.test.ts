import assert from "node:assert/strict";
import { test } from "node:test";
import type { TestContext } from "node:test";
import {
  PlaybackBudget,
  playbackCapacity,
} from "../src/previews/playback-budget.ts";
import type {
  PlaybackGroup,
  PlaybackLease,
} from "../src/previews/playback-budget.ts";

function fixture(t: TestContext, limit: number) {
  t.mock.timers.enable({ apis: ["setTimeout", "Date"], now: 100000 });
  let clockReads = 0;
  const budget = new PlaybackBudget(limit, () => {
      clockReads++;
      return Date.now();
    }),
    active = new Set<string>(),
    seen = new Set<string>();
  const stops: (() => void)[] = [];
  let peak = 0;
  const leases = new Map<string, PlaybackLease>();
  t.after(() => stops.forEach((stop) => stop()));
  return {
    budget,
    active,
    seen,
    peak: () => peak,
    ready: (id: string) => leases.get(id)!.ready(),
    clockReads: () => clockReads,
    add(id: string, group: PlaybackGroup, priority = 20, immediate = true) {
      let lease: PlaybackLease | undefined;
      lease = budget.request({ group, priority }, (granted) => {
        if (granted) {
          active.add(id);
          seen.add(id);
          if (immediate) lease?.ready();
        } else active.delete(id);
        peak = Math.max(peak, active.size);
      });
      leases.set(id, lease);
      if (immediate) lease.ready();
      stops.push(lease.cancel);
      return lease.cancel;
    },
  };
}

test("device capacity is conservative for unknown, low-memory and low-core devices", () => {
  assert.equal(playbackCapacity(), 10);
  assert.equal(playbackCapacity({ cores: 2, memory: 16 }), 6);
  assert.equal(playbackCapacity({ cores: 16, memory: 2 }), 6);
  assert.equal(playbackCapacity({ cores: 4, memory: 8 }), 10);
  assert.equal(playbackCapacity({ cores: 16, memory: 4 }), 10);
  assert.equal(playbackCapacity({ cores: 8, memory: 8 }), 18);
});

test("a large selection cannot starve collection controls or pointer interactions", (t) => {
  const f = fixture(t, 10);
  for (let i = 0; i < 24; i++) f.add(`model-${i}`, "selection");
  f.add("collection", "collection", 10);
  f.add("hover", "interaction", 30);
  assert.equal(f.active.size, 10);
  assert.ok(f.active.has("collection"));
  assert.ok(f.active.has("hover"));
  assert.equal([...f.active].filter((id) => id.startsWith("model-")).length, 8);
  assert.ok(f.peak() <= 10);
});

test("busy visible sections share capacity instead of one consuming all remaining slots", (t) => {
  const f = fixture(t, 10);
  for (let i = 0; i < 24; i++) f.add(`selected-${i}`, "selection");
  for (let i = 0; i < 8; i++) f.add(`collection-${i}`, "collection", 10);
  assert.equal(
    [...f.active].filter((id) => id.startsWith("selected-")).length,
    5,
  );
  assert.equal(
    [...f.active].filter((id) => id.startsWith("collection-")).length,
    5,
  );
  t.mock.timers.tick(4000);
  assert.equal(f.active.size, 10);
  assert.equal(
    [...f.active].filter((id) => id.startsWith("collection-")).length,
    5,
  );
});

test("dialog eligibility follows its section rather than an arbitrary priority threshold", (t) => {
  const f = fixture(t, 2);
  f.add("interaction", "interaction", 1000);
  f.add("detail", "detail", 1);
  const resume = f.budget.suspendBackground();
  assert.deepEqual([...f.active], ["detail"]);
  resume();
  assert.equal(f.active.size, 2);
});

test("queued peers rotate without new requests, while other groups keep playing", (t) => {
  const f = fixture(t, 3);
  for (let i = 0; i < 6; i++) f.add(`model-${i}`, "selection");
  f.add("collection", "collection", 10);
  const original = [...f.active];
  t.mock.timers.tick(3999);
  assert.deepEqual([...f.active], original);
  t.mock.timers.tick(1);
  assert.notDeepEqual([...f.active], original);
  assert.ok(f.active.has("collection"));
  t.mock.timers.tick(8000);
  assert.equal(f.seen.size, 7, "Every queued model gets a turn");
  assert.equal(f.peak(), 3);
});

test("slow loads receive a full turn after readiness instead of being repeatedly aborted", (t) => {
  const f = fixture(t, 1);
  f.add("slow", "selection", 20, false);
  f.add("queued", "selection");
  t.mock.timers.tick(8000);
  assert.deepEqual([...f.active], ["slow"]);
  f.ready("slow");
  t.mock.timers.tick(3999);
  assert.deepEqual([...f.active], ["slow"]);
  t.mock.timers.tick(1);
  assert.deepEqual([...f.active], ["queued"]);
});

test("a newly ready player cannot inherit another player's almost-expired turn", (t) => {
  const f = fixture(t, 2);
  f.add("first", "selection");
  f.add("new", "selection", 20, false);
  f.add("queued-one", "selection");
  f.add("queued-two", "selection");
  t.mock.timers.tick(3000);
  f.ready("new");
  t.mock.timers.tick(1000);
  assert.deepEqual([...f.active].sort(), ["new", "queued-one"]);
  t.mock.timers.tick(2999);
  assert.ok(f.active.has("new"));
  t.mock.timers.tick(1);
  assert.deepEqual([...f.active].sort(), ["queued-one", "queued-two"]);
});

test("one waiter replaces only one renderer instead of churning every expired winner", (t) => {
  const f = fixture(t, 4);
  for (let i = 0; i < 5; i++) f.add(String(i), "selection");
  const previous = new Set(f.active);
  t.mock.timers.tick(4000);
  assert.equal([...f.active].filter((id) => previous.has(id)).length, 3);
  assert.equal(f.active.size, 4);
});

test("late readiness after scrolling away cannot restart rotation or playback", (t) => {
  const f = fixture(t, 1);
  const cancel = f.add("scrolled-away", "selection", 20, false);
  f.add("visible", "selection");
  cancel();
  f.ready("scrolled-away");
  t.mock.timers.tick(12000);
  assert.deepEqual([...f.active], ["visible"]);
});

test("removing all visible work cancels the rotation timer rather than polling an empty budget", (t) => {
  const f = fixture(t, 1);
  const a = f.add("a", "selection"),
    b = f.add("b", "selection");
  a();
  b();
  const reads = f.clockReads();
  t.mock.timers.tick(12000);
  assert.equal(f.clockReads(), reads);
  assert.equal(f.active.size, 0);
});

test("invalid capacity is rejected at construction as well as during updates", () => {
  for (const limit of [0, -1, NaN, 1.5])
    assert.throws(() => new PlaybackBudget(limit));
});

test("interaction priority stays pinned while equal-priority peers take turns", (t) => {
  const f = fixture(t, 2);
  for (let i = 0; i < 4; i++) f.add(`model-${i}`, "selection");
  f.add("hover", "interaction", 30);
  for (let i = 0; i < 4; i++) {
    t.mock.timers.tick(4000);
    assert.ok(f.active.has("hover"));
  }
  assert.equal(f.seen.size, 5);
});

test("capacity changes are immediate and stop old players before admitting new ones", (t) => {
  const f = fixture(t, 4);
  for (let i = 0; i < 8; i++) f.add(String(i), "detail", 100);
  f.budget.setLimit(2);
  assert.equal(f.active.size, 2);
  f.budget.setLimit(8);
  assert.equal(f.active.size, 8);
  assert.equal(f.peak(), 8);
  for (const value of [0, -1, 1.5, NaN])
    assert.throws(() => f.budget.setLimit(value));
});

test("nested suspensions stop rotation and resume only eligible requests", (t) => {
  const f = fixture(t, 2);
  f.add("collection", "collection", 10);
  for (let i = 0; i < 5; i++) f.add(`detail-${i}`, "detail", 100);
  const background = f.budget.suspendBackground(),
    popup = f.budget.suspendAll();
  assert.equal(f.active.size, 0);
  const seen = f.seen.size;
  t.mock.timers.tick(12000);
  assert.equal(f.seen.size, seen);
  background();
  assert.equal(f.active.size, 0);
  popup();
  assert.equal(f.active.size, 2);
  assert.ok([...f.active].some((id) => id.startsWith("detail-")));
  assert.ok(f.active.has("collection"));
});

test("cancelling queued work cannot resurrect it on later rotations", (t) => {
  const f = fixture(t, 2);
  const a = f.add("a", "selection"),
    b = f.add("b", "selection"),
    c = f.add("cancelled", "selection");
  c();
  t.mock.timers.tick(12000);
  assert.ok(!f.seen.has("cancelled"));
  a();
  b();
  t.mock.timers.tick(12000);
  assert.equal(f.active.size, 0);
});

test("callbacks may suspend playback while grants are being reconciled", (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const budget = new PlaybackBudget(1),
    active = new Set<string>();
  let resume: (() => void) | undefined;
  const stop = budget.request(
    { priority: 20, group: "selection" },
    (granted) => {
      if (granted) {
        active.add("one");
        resume = budget.suspendAll();
      } else active.delete("one");
    },
  );
  assert.equal(active.size, 0);
  stop.cancel();
  resume?.();
  t.mock.timers.tick(8000);
  assert.equal(active.size, 0);
});
