import assert from "node:assert/strict";
import { test } from "node:test";
import { FloorsController } from "../src/valuation/floors-controller.ts";
import type { LiveFrame, SubscriptionCommand } from "../src/ws-protocol.ts";
import { parseFrame } from "../src/ws-protocol.ts";
import {
  giftBasisChanged,
  evaluatedAfter,
} from "../src/valuation/freshness.ts";
import type { ValuationResponse } from "../src/market.ts";
import type { FloorsEntry } from "../src/valuation/floors-controller.ts";
const settle = () => new Promise((resolve) => setTimeout(resolve, 60));
test("gift estimates become stale when prices, evidence or catalog membership change", () => {
  const evidence = { listings: 3, gifts: 3 },
    price = { value: "10" };
  const report: ValuationResponse = {
    target: {
      kind: "gift",
      gift: {
        id: "1",
        collection_id: "gift",
        number: 1,
        traits: { model: "model", backdrop: "backdrop" },
      },
    },
    currency_id: "GRAM",
    evaluated_at: "2026-11-20T12:00:00Z",
    stale: false,
    results: [
      {
        strategy: "gift_floor",
        data: {
          collection_floor: price,
          model_floor: price,
          backdrop_floor: price,
          model_backdrop_floor: price,
          model_multiplier: price,
          backdrop_multiplier: price,
          final_mark: price,
          evidence: {
            collection: evidence,
            model: evidence,
            backdrop: evidence,
            model_backdrop: evidence,
            minimum_multiplier_gifts: 3,
          },
        },
        display: { fields: [], note: "", summary: null },
      },
    ],
  };
  const floor = { price, evidence };
  const shared: Pick<FloorsEntry, "floor" | "traits"> = {
    floor,
    traits: {
      models: { model: floor },
      backdrops: { backdrop: floor },
      pair: { model_id: "model", backdrop_id: "backdrop", floor },
    },
  };
  assert.equal(giftBasisChanged(report, shared), false);
  assert.equal(
    giftBasisChanged(report, {
      ...shared,
      floor: { ...floor, price: { value: "11" } },
    }),
    true,
  );
  assert.equal(
    giftBasisChanged(report, {
      ...shared,
      floor: { ...floor, evidence: { listings: 4, gifts: 3 } },
    }),
    true,
  );
  const removed = structuredClone(shared);
  delete removed.traits!.models.model;
  assert.equal(giftBasisChanged(report, removed), true);
  const pair = structuredClone(shared);
  pair.traits!.pair!.floor.price = { value: "12" };
  assert.equal(giftBasisChanged(report, pair), true);
  assert.equal(
    giftBasisChanged(report, { floor }),
    false,
    "Awaiting a trait baseline does not invent a difference",
  );
});
function fixture() {
  const commands: SubscriptionCommand[] = [];
  let closed = 0,
    retries = 0;
  let receive!: (frame: LiveFrame, command: SubscriptionCommand) => void,
    status!: (value: string) => void;
  const controller = new FloorsController({
    subscribe(command, frame, changed) {
      commands.push(command);
      receive = frame;
      status = changed!;
      return () => {
        closed++;
      };
    },
    update(command) {
      commands.push(command);
    },
    retry() {
      retries++;
    },
  });
  const emit = (
    command = commands.at(-1)!,
    price = "3",
    times: Record<string, string> = {},
  ) => {
    assert.equal(command.type, "floors.watch");
    if (command.type !== "floors.watch") throw Error();
    const floor = {
      price: { value: price },
      evidence: { listings: 3, gifts: 3 },
    };
    const details = command.params.details;
    const frame = {
      type: "floors.snapshot" as const,
      id: command.id,
      data: {
        currency_id: command.params.currency_id,
        stale: false,
        collections: command.params.collection_ids.map((collection_id) => ({
          collection_id,
          evaluated_at:
            times[collection_id] ?? "2026-11-20T12:00:00.000000001Z",
          floor,
          ...(details?.collection_id === collection_id
            ? {
                traits: {
                  models: { model: floor },
                  backdrops: { backdrop: floor },
                  ...(details.pair ? { pair: { ...details.pair, floor } } : {}),
                },
              }
            : {}),
        })),
      },
    };
    receive(frame, command);
    return frame;
  };
  return {
    controller,
    commands,
    emit,
    status: (value: string) => status(value),
    closed: () => closed,
    retries: () => retries,
  };
}
test("visible floors are batched once and scope changes reject old frames before debounce", async () => {
  const f = fixture();
  try {
    f.controller.select("GRAM", "a", null);
    f.controller.visibleCollections([
      "a",
      "b",
      "c",
      "d",
      "e",
      "f",
      "g",
      "h",
      "i",
    ]);
    await settle();
    assert.equal(f.commands.length, 1);
    const first = f.commands[0];
    assert.equal(first.type, "floors.watch");
    if (first.type !== "floors.watch") return;
    assert.equal(first.params.collection_ids.length, 8);
    f.emit();
    assert.equal(
      f.controller.getSnapshot().entries.get("a")?.floor.price.value,
      "3",
    );
    f.controller.visibleCollections([
      "a",
      "b",
      "c",
      "d",
      "e",
      "f",
      "g",
      "h",
      "i",
    ]);
    await settle();
    assert.equal(f.commands.length, 1);
    f.controller.select("GRAM", "b", null);
    f.emit(first, "999");
    assert.equal(
      f.controller.getSnapshot().entries.get("a")?.floor.price.value,
      "3",
    );
    assert.equal(f.controller.getSnapshot().entries.get("a")?.stale, true);
    await settle();
    f.emit();
    assert.equal(f.commands.length, 2);
    assert.equal(
      f.controller.getSnapshot().entries.get("a")?.traits,
      undefined,
    );
    assert.ok(f.controller.getSnapshot().entries.get("b")?.traits);
    f.controller.select("XTR", "b", null);
    assert.equal(f.controller.getSnapshot().entries.size, 0);
    f.emit(first);
    await settle();
    f.emit();
    assert.equal(
      f.controller.getSnapshot().entries.get("b")?.floor.price.value,
      "3",
    );
  } finally {
    f.controller.close();
  }
  assert.equal(f.closed(), 1);
});
test("reconnects retain stale values until a baseline and retries reuse the one watch", async () => {
  const f = fixture();
  try {
    f.controller.select("GRAM", "gift", {
      model_id: "model",
      backdrop_id: "backdrop",
    });
    await settle();
    f.emit();
    f.status("Reconnecting");
    assert.equal(f.controller.getSnapshot().entries.get("gift")?.stale, true);
    f.status("Live");
    assert.equal(f.controller.getSnapshot().entries.get("gift")?.stale, true);
    f.emit();
    assert.equal(f.controller.getSnapshot().entries.get("gift")?.stale, false);
    f.controller.retry();
    assert.equal(f.retries(), 1);
    assert.equal(f.commands.length, 1);
    f.controller.close();
    f.emit(undefined, "999");
    assert.equal(
      f.controller.getSnapshot().entries.get("gift")?.floor.price.value,
      "3",
    );
  } finally {
    f.controller.close();
  }
});
test("market floor frames reject duplicate scopes, impossible evidence and mixed metrics", async () => {
  const f = fixture();
  try {
    f.controller.select("GRAM", "gift", null);
    await settle();
    const frame = f.emit();
    assert.ok(parseFrame(JSON.stringify(frame)));
    const invalid = structuredClone(frame);
    invalid.data.collections.push(invalid.data.collections[0]);
    assert.throws(() => parseFrame(JSON.stringify(invalid)));
    for (const metric of [
      { value: "NaN" },
      { value: "-1" },
      { value: "1", error: { code: "not_enough_data", message: "empty" } },
    ]) {
      const copy: any = structuredClone(frame);
      copy.data.collections[0].floor.price = metric;
      assert.throws(() => parseFrame(JSON.stringify(copy)));
    }
    const impossible = structuredClone(frame);
    impossible.data.collections[0].floor.evidence.gifts = 4;
    assert.throws(() => parseFrame(JSON.stringify(impossible)));
  } finally {
    f.controller.close();
  }
});

test("other collections do not advance selected freshness, including nanosecond changes", async () => {
  const f = fixture();
  try {
    f.controller.select("GRAM", "gift", null);
    f.controller.visibleCollections(["other"]);
    await settle();
    f.emit();
    const before = f.controller.getSnapshot().entries.get("gift")!.evaluatedAt;
    f.emit(undefined, "3", { other: "2026-11-20T12:01:00Z" });
    assert.equal(
      f.controller.getSnapshot().entries.get("gift")!.evaluatedAt,
      before,
    );
    assert.equal(evaluatedAfter(before, "2026-11-20T12:00:01Z"), false);
  } finally {
    f.controller.close();
  }
  assert.equal(
    evaluatedAfter(
      "2026-11-20T12:00:00.123000001Z",
      "2026-11-20T12:00:00.123Z",
    ),
    true,
  );
  assert.equal(
    evaluatedAfter(
      "2026-11-20T12:00:00.123Z",
      "2026-11-20T12:00:00.123000001Z",
    ),
    false,
  );
});
