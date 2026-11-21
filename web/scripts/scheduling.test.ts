import assert from "node:assert/strict";
import { test } from "node:test";
import { TaskQueue, QueueFullError } from "../src/resources/task-queue.ts";
import { ResourceCache } from "../src/resources/cache.ts";
import { PlaybackBudget } from "../src/previews/playback-budget.ts";
const signal = () => new AbortController().signal;
test("twenty-four resource consumers eventually complete with bounded producers", async () => {
  let active = 0,
    peak = 0;
  const store = new ResourceCache(
    async (key) => {
      peak = Math.max(peak, ++active);
      await new Promise((resolve) => setTimeout(resolve, 2));
      active--;
      return { value: key, bytes: 1 };
    },
    1024,
    4,
  );
  const keys = Array.from({ length: 24 }, (_, i) => String(i));
  assert.deepEqual(
    await Promise.all(keys.map((key) => store.get(key, signal()))),
    keys,
  );
  assert.equal(peak, 4);
});
test("queued work honors priority, cancellation and the waiting bound", async () => {
  const queue = new TaskQueue(1, 2),
    order: string[] = [];
  let finish!: () => void;
  const first = queue.run(
    () =>
      new Promise<void>((resolve) => {
        finish = resolve;
      }),
    signal(),
  );
  const stop = new AbortController();
  const cancelled = queue.run(async () => {
    throw Error("abandoned work ran");
  }, stop.signal);
  stop.abort();
  await assert.rejects(cancelled);
  const low = queue.run(
    async () => {
      order.push("low");
    },
    signal(),
    1,
  );
  const high = queue.run(
    async () => {
      order.push("high");
    },
    signal(),
    100,
  );
  await assert.rejects(
    queue.run(async () => {}, signal()),
    QueueFullError,
  );
  finish();
  await Promise.all([first, low, high]);
  assert.deepEqual(order, ["high", "low"]);
});
test("playback preemption stops old players before starting replacements", () => {
  const budget = new PlaybackBudget(2),
    active = new Set<string>();
  let peak = 0;
  const request = (id: string, priority: number) =>
    budget.request(
      {
        priority,
        group:
          priority >= 100
            ? "detail"
            : priority >= 30
              ? "interaction"
              : priority >= 20
                ? "selection"
                : "collection",
      },
      (granted) => {
        if (granted) active.add(id);
        else active.delete(id);
        peak = Math.max(peak, active.size);
      },
    ).cancel;
  const low = request("rail", 10),
    selected = request("selected", 20),
    interaction = request("interaction", 30);
  assert.deepEqual([...active].sort(), ["interaction", "selected"]);
  const detail = request("detail", 100),
    resume = budget.suspendBackground();
  assert.deepEqual([...active], ["detail"]);
  const resumeNested = budget.suspendBackground();
  resume();
  resume();
  assert.deepEqual([...active], ["detail"]);
  resumeNested();
  assert.deepEqual([...active].sort(), ["detail", "interaction"]);
  detail();
  interaction();
  assert.deepEqual([...active].sort(), ["rail", "selected"]);
  selected();
  low();
  assert.equal(active.size, 0);
  assert.equal(peak, 2);
});
