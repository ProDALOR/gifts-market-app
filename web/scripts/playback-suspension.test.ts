import assert from "node:assert/strict";
import { test } from "node:test";
import { PlaybackBudget } from "../src/previews/playback-budget.ts";

test("nested info popups pause detail playback and restore the remaining modal budget", () => {
  const budget = new PlaybackBudget();
  const active = new Set<string>();
  const subscribe = (id: string, priority: number) =>
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
      },
    ).cancel;
  subscribe("grid", 10);
  subscribe("detail", 100);
  const closeDetails = budget.suspendBackground();
  assert.deepEqual([...active], ["detail"]);
  const closeInfo = budget.suspendAll();
  assert.equal(active.size, 0);
  closeInfo();
  assert.deepEqual([...active], ["detail"]);
  closeInfo();
  assert.deepEqual(
    [...active],
    ["detail"],
    "Repeated cleanup does not resume the grid",
  );
  const closeAnotherInfo = budget.suspendAll();
  closeDetails();
  assert.equal(active.size, 0, "A remaining popup still blocks every player");
  closeAnotherInfo();
  assert.deepEqual([...active].sort(), ["detail", "grid"]);
});
