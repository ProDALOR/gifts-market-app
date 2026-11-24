import assert from "node:assert/strict";
import { test } from "node:test";
import {
  progressText,
  portfolioRelation,
  portfolioSort,
} from "../src/portfolio/model.ts";
import { parseFrame } from "../src/ws-protocol.ts";
import { PortfolioPager } from "../src/portfolio/pager.ts";
import {
  portfolioPage,
  portfolioSnapshot,
  portfolioVersion,
} from "./portfolio-fixture.mjs";
const item = (number: number, estimate?: string) => ({
  gift: {
    gift: {
      id: String(number),
      slug: `Gift-${number}`,
      number,
      collection: "1",
      model: "2",
    },
    relation: "owned",
  },
  resolved: {
    id: String(number),
    number,
    collection_id: "gift",
    traits: { model: "m", backdrop: "b" },
  },
  estimate,
});
const report = (count = 6000, refresh = 1) => ({
  user_id: 7,
  refresh_id: refresh,
  progress: {
    stage: "ready",
    checked: count,
    total: count,
    valued: count,
    collectibles: count,
  },
  inventory: {
    checked_at: "2026-11-24T00:00:00Z",
    count,
    scanned: count,
    regular: 0,
    skipped: 0,
    complete: true,
  },
  valuation: {
    currency_id: "GRAM",
    evaluated_at: "2026-11-24T00:00:00Z",
    stale: false,
    total: String(count * 2),
    priced: count,
    unpriced: 0,
    items: Array.from({ length: count }, (_, i) => item(i + 1, "2")),
  },
});
const query = {
  user_id: 7,
  currency_id: "GRAM",
  sort: "position_asc" as const,
  filters: {
    collection_id: undefined,
    models: [],
    backdrops: [],
    patterns: [],
    number: undefined,
  },
};
const eventually = async (predicate: () => boolean) => {
  const end = Date.now() + 2000;
  while (!predicate() && Date.now() < end)
    await new Promise((resolve) => setTimeout(resolve, 1));
  assert(predicate());
};
const settled = async (pager: PortfolioPager) =>
  eventually(() => !pager.current().loading);

test("compact progress keeps counts without exposing inventory pages", () => {
  const state = portfolioSnapshot(report(2));
  assert.equal(
    progressText({
      ...state,
      progress: {
        ...state.progress,
        stage: "loading_inventory",
        total: undefined,
        checked: 100,
      },
    }),
    "100 gifts checked",
  );
  assert.equal(
    progressText({
      ...state,
      inventory: { ...state.inventory, complete: false },
    }),
    "Partial inventory",
  );
  assert.equal(
    progressText({
      ...state,
      progress: { ...state.progress, stage: "failed" },
      error: "Previous inventory retained",
    }),
    "Previous inventory retained",
  );
  assert.equal(portfolioSort("position", true), "position_asc");
});
test("summary frames reject malformed identities, counts and legacy full inventories", () => {
  const frame = {
    type: "portfolio.snapshot",
    id: "p",
    data: portfolioSnapshot(report(2)),
  };
  assert.equal(parseFrame(JSON.stringify(frame)).type, "portfolio.snapshot");
  for (const change of [
    (s) => {
      s.user_id = 0;
    },
    (s) => {
      s.progress.checked = -1;
    },
    (s) => {
      s.valuation.version = "bad";
    },
    (s) => {
      s.valuation.count = 10;
    },
    (s) => {
      s.valuation.items = [item(1, "2")];
    },
  ]) {
    const bad = structuredClone(frame);
    change(bad.data);
    assert.throws(() => parseFrame(JSON.stringify(bad)));
  }
  assert.equal(
    parseFrame(
      JSON.stringify({
        type: "portfolio.progress",
        id: "p",
        refresh_id: 2,
        progress: frame.data.progress,
      }),
    ).type,
    "portfolio.progress",
  );
});
test("page frames validate bounded items and keep unpriced/unverified gifts", () => {
  const source = report(2);
  source.valuation.items[1].estimate = undefined;
  source.valuation.items[1].gift.relation = "unverified";
  const page = portfolioPage(source, { ...query, limit: 100 });
  const frame = {
    type: "result",
    method: "portfolio.query",
    id: "p",
    data: page,
  };
  assert.equal(parseFrame(JSON.stringify(frame)).type, "result");
  assert.equal(page.selection.unpriced, 1);
  assert.equal(page.selection.unverified, 1);
  for (const change of [
    (p) => {
      p.items[0].estimate = "bad";
    },
    (p) => {
      p.items[0].gift.gift.number = 0;
    },
    (p) => {
      p.items = Array.from({ length: 101 }, (_, i) => item(i + 1));
    },
  ]) {
    const bad = structuredClone(frame);
    change(bad.data);
    assert.throws(() => parseFrame(JSON.stringify(bad)));
  }
  assert.equal(portfolioRelation("owned"), undefined);
  assert.equal(portfolioRelation("hosted")?.label, "Hosted");
  assert.equal(
    portfolioRelation("unverified")?.description,
    "Visible on this profile; ownership unverified.",
  );
});
test("first load is 100 rows, scroll demand appends without changing full totals", async () => {
  const source = report();
  const calls = [];
  const pager = new PortfolioPager({
    query,
    request: async (q) => {
      calls.push(q);
      return portfolioPage(source, q);
    },
    changed: () => {},
    refreshSummary: () => assert.fail(),
  });
  pager.update(portfolioVersion(source));
  await settled(pager);
  assert.equal(calls.length, 1);
  assert.equal(pager.current().items.length, 100);
  assert.equal(pager.current().selection!.count, 6000);
  assert.equal(pager.current().selection!.total, "12000");
  pager.loadMore();
  pager.loadMore();
  await settled(pager);
  assert.equal(calls.length, 2);
  assert.equal(calls[1].offset, 100);
  assert.deepEqual(
    pager.current().items.map((i) => i.gift.gift.number),
    Array.from({ length: 200 }, (_, i) => i + 1),
  );
  const initial = pager.current();
  pager.dispose();
  const restored = new PortfolioPager({
    query,
    initial,
    request: async () =>
      assert.fail("unchanged reconnect must reuse visited pages"),
    changed: () => {},
    refreshSummary: () => {},
  });
  restored.update(portfolioVersion(source));
  assert.equal(restored.current().items.length, 200);
  restored.dispose();
});
test("server filtering finds gifts beyond the first page", async () => {
  const source = report();
  const pager = new PortfolioPager({
    query: { ...query, filters: { ...query.filters, number: 5999 } },
    request: async (q) => portfolioPage(source, q),
    changed: () => {},
    refreshSummary: () => {},
  });
  pager.update(portfolioVersion(source));
  await settled(pager);
  assert.equal(pager.current().items[0].gift.gift.number, 5999);
  assert.equal(pager.current().selection!.count, 1);
  assert.equal(pager.current().hasMore, false);
  pager.dispose();
});
test("changed versions refresh only visited coverage atomically with bounded concurrency", async () => {
  let source = report();
  let active = 0,
    maximum = 0;
  const calls = [],
    committed = [];
  const pager = new PortfolioPager({
    query,
    request: async (q) => {
      calls.push(q);
      active++;
      maximum = Math.max(maximum, active);
      await new Promise((resolve) => setTimeout(resolve, 2));
      active--;
      return portfolioPage(source, q);
    },
    changed: (rows) => {
      if (!rows.loading) committed.push(rows);
    },
    refreshSummary: () => {},
  });
  pager.update(portfolioVersion(source));
  await settled(pager);
  for (let i = 0; i < 3; i++) {
    pager.loadMore();
    await settled(pager);
  }
  const previous = pager.current().items;
  source = report(6000, 2);
  source.valuation.items.reverse();
  const before = calls.length,
    commits = committed.length;
  pager.update(portfolioVersion(source));
  assert.equal(pager.current().items, previous);
  await settled(pager);
  assert.equal(calls.length - before, 4);
  assert.equal(committed.length - commits, 1);
  assert.equal(pager.current().items.length, 400);
  assert.equal(pager.current().items[0].gift.gift.number, 6000);
  assert(maximum <= 2);
  pager.dispose();
});
test("cancellation discards a late page and releases scroll intent on resume", async () => {
  const source = report();
  let finish;
  let signal;
  let calls = 0;
  const pager = new PortfolioPager({
    query,
    request: (q, stop) => {
      calls++;
      signal = stop;
      return calls === 1
        ? new Promise((resolve) => {
            finish = () => resolve(portfolioPage(source, q));
          })
        : Promise.resolve(portfolioPage(source, q));
    },
    changed: () => {},
    refreshSummary: () => {},
  });
  pager.update(portfolioVersion(source));
  pager.pause();
  assert(signal.aborted);
  finish();
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(pager.current().items.length, 0);
  pager.update(portfolioVersion(source));
  await settled(pager);
  assert.equal(pager.current().items.length, 100);
  pager.dispose();
});
test("expired page versions wait for a fresh summary without mixing rows", async () => {
  let source = report();
  let refreshes = 0;
  const pager = new PortfolioPager({
    query,
    request: async (q) => portfolioPage(source, q),
    changed: () => {},
    refreshSummary: () => {
      refreshes++;
    },
  });
  pager.update(portfolioVersion(source));
  await settled(pager);
  const previous = pager.current().items;
  source = report(6000, 2);
  pager.loadMore();
  await eventually(() => refreshes === 1);
  assert.equal(pager.current().items, previous);
  pager.update(portfolioVersion(report()));
  assert.equal(refreshes, 1);
  pager.update(portfolioVersion(source));
  await settled(pager);
  assert.equal(pager.current().version, portfolioVersion(source));
  assert.equal(pager.current().items.length, 100);
  pager.dispose();
});
test("failed next pages retain data and retry the same offset", async () => {
  const source = report();
  let fail = false;
  const offsets = [];
  const pager = new PortfolioPager({
    query,
    request: async (q) => {
      offsets.push(q.offset);
      if (fail) throw new Error("temporary");
      return portfolioPage(source, q);
    },
    changed: () => {},
    refreshSummary: () => {},
  });
  pager.update(portfolioVersion(source));
  await settled(pager);
  fail = true;
  pager.loadMore();
  await settled(pager);
  assert.equal(pager.current().items.length, 100);
  assert.equal(pager.current().error, "temporary");
  fail = false;
  pager.retry();
  await settled(pager);
  assert.deepEqual(offsets, [0, 100, 100]);
  assert.equal(pager.current().items.length, 200);
  pager.dispose();
});
