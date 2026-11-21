import assert from "node:assert/strict";
import { test } from "node:test";
import { parseFrame } from "../src/ws-protocol.ts";
const sync = { ready: true, state: "live", updated_at: null };
const page = {
  type: "page",
  id: "op.1",
  revision: "r",
  sync,
  page: { data: [], cursor: "checkpoint", next_page_token: "", sync },
};
const result = {
  type: "result",
  id: "op.2",
  method: "valuation.query",
  data: {
    target: {
      kind: "gift",
      gift: {
        id: "1",
        collection_id: "gift",
        number: 1,
        traits: { model: null, backdrop: null, pattern: null },
      },
    },
    currency_id: "GRAM",
    evaluated_at: "2026-11-18T12:00:00Z",
    stale: false,
    results: [
      {
        strategy: "gift_floor",
        data: { final_mark: { value: "100000000000000000.000000001" } },
        revision: "r1",
      },
    ],
  },
};
test("typed envelopes preserve opaque revisions and exact decimal strings", () => {
  for (const frame of [
    result,
    {
      type: "catalog.versions",
      id: "op.3",
      versions: { directory: "d", collections: { gift: null } },
    },
    {
      type: "command_error",
      id: "",
      error: { code: "invalid_query", message: "Invalid command" },
    },
  ])
    assert.deepEqual(parseFrame(JSON.stringify(frame)), frame);
});
test("invalid JSON, envelope identities and malformed payloads are rejected before dispatch", () => {
  for (const invalid of [
    null,
    [],
    {},
    false,
    { ...page, id: "" },
    { ...page, id: "bad id" },
    { ...page, id: "x".repeat(65) },
    { ...page, type: "unknown" },
    {
      type: "error",
      id: "",
      error: { code: "invalid_query", message: "Uncorrelated" },
    },
    { ...page, sync: null },
    { ...page, sync: { ...sync, state: ["live"] } },
    { ...page, page: { ...page.page, data: null } },
    { ...page, page: { ...page.page, data: [null] } },
    { ...page, page: { ...page.page, data: [{ id: "bad", quantity: 0 }] } },
    { ...result, method: "unknown" },
    {
      ...result,
      data: { ...result.data, target: { kind: "gift", gift: null } },
    },
    {
      ...result,
      data: {
        ...result.data,
        results: [
          {
            strategy: "gift_floor",
            data: {},
            display: { fields: [{ value: { kind: "currency", amount: NaN } }] },
          },
        ],
      },
    },
    { type: "error", id: "op.1", error: { message: "Missing code" } },
  ])
    assert.throws(() => parseFrame(JSON.stringify(invalid)));
  for (const raw of ["{", new Uint8Array([1]), " ".repeat(8 * 1024 * 1024 + 1)])
    assert.throws(() => parseFrame(raw));
});

test("range snapshots and atomic changes validate identities, coverage and sequences", () => {
  const row = {
    id: "a",
    book: { venue_id: "v", collection_id: "gift", currency_id: "GRAM" },
    kind: "ask",
    quantity: 1,
    price: "1",
    observed_at: "2026-11-18T12:00:00Z",
    target: {
      kind: "criteria",
      models: [],
      backdrops: [],
      patterns: [],
      number: null,
    },
    key: "0NO00640080", // Identity key: asks, venue v, order a.
  };
  const frame = {
    type: "orders.snapshot",
    id: "op.1",
    sequence: 0,
    data: [row],
    range: { start: null, end: null },
  };
  assert.deepEqual(parseFrame(JSON.stringify(frame)), frame);
  for (const invalid of [
    { type: "status", id: "op.1", state: "connecting" },
    {
      type: "catalog",
      id: "op.1",
      versions: { directory: "old", collections: {} },
    },
    { ...frame, sequence: -1 },
    { ...frame, next: null },
    { ...frame, next: "invalid!" },
    { ...frame, snapshot: "unused" },
    { ...frame, sequence: Number.MAX_SAFE_INTEGER + 1 },
    { ...frame, data: [row, row] },
    { ...frame, data: [{ ...row, key: "" }] },
    {
      ...frame,
      data: [row],
      range: { start: "0NO00680080", end: null },
    },
    { ...frame, range: { start: null } },
  ])
    assert.throws(() => parseFrame(JSON.stringify(invalid)));
  const change = {
    type: "orders.changed",
    id: "op.1",
    sequence: 1,
    upserts: [row],
    removed: [],
  };
  assert.deepEqual(parseFrame(JSON.stringify(change)), change);
  for (const invalid of [
    { ...change, sequence: -1 },
    { ...change, next: null },
    { ...change, range: { start: null, end: null } },
    { ...change, snapshot: "unused" },
    { ...change, upserts: [row, row] },
    { ...change, removed: [{ venue_id: "v", id: "a" }] },
    { ...change, removed: [{ id: "a" }] },
  ])
    assert.throws(() => parseFrame(JSON.stringify(invalid)));
  const history = {
    ...frame,
    data: [{ ...row, kind: "executed", executed_at: "2026-11-18T12:00:00Z" }],
  };
  assert.deepEqual(parseFrame(JSON.stringify(history)), history);
  const finite = {
    type: "result",
    id: "q",
    method: "orders.query",
    data: frame.data,
    range: frame.range,
  };
  assert.deepEqual(parseFrame(JSON.stringify(finite)), finite);
});

test("market status has an app-facing contract and valuation results reject operational fields", () => {
  for (const status of ["available", "delayed", "unavailable"]) {
    const frame = { type: "market.status", status };
    assert.deepEqual(parseFrame(JSON.stringify(frame)), frame);
  }
  for (const frame of [
    { type: "replica.status", sync },
    { type: "market.status", status: "live" },
    { type: "market.status", id: "op.1", status: "available" },
    { ...result, data: { ...result.data, revision: "private-hub-checkpoint" } },
    { ...result, data: { ...result.data, sync } },
    { ...result, data: { ...result.data, stale: undefined } },
  ])
    assert.throws(() => parseFrame(JSON.stringify(frame)));
});

test("retired fields are rejected in envelopes, query results and order rows", () => {
  const row = {
    id: "order",
    key: "0001",
    book: { venue_id: "v", collection_id: "g", currency_id: "GRAM" },
    kind: "ask",
    quantity: 1,
    price: "1",
    observed_at: "2026-11-19T00:00:00Z",
    target: {
      kind: "criteria",
      models: [],
      backdrops: [],
      patterns: [],
      number: null,
    },
  };
  const data = { data: [row], range: { start: null, end: null } };
  const finite = { type: "result", method: "orders.query", id: "q", ...data };
  const changed = {
    type: "orders.changed",
    id: "q",
    sequence: 1,
    upserts: [row],
    removed: [],
  };
  for (const field of [
    "sync",
    "revision",
    "cursor",
    "page_token",
    "next_page_token",
    "base_sequence",
    "reset",
    "position",
    "sort_key",
  ]) {
    for (const frame of [
      { ...changed, [field]: null },
      { ...finite, [field]: null },
      { ...changed, upserts: [{ ...row, [field]: null }] },
    ])
      assert.throws(() => parseFrame(JSON.stringify(frame)), field);
  }
  assert.deepEqual(parseFrame(JSON.stringify(finite)), finite);
  assert.deepEqual(parseFrame(JSON.stringify(changed)), changed);
});

test("flat targets accept sparse description and reject retired nested payloads", () => {
  const row = {
    id: "a",
    book: { venue_id: "v", collection_id: "gift", currency_id: "GRAM" },
    kind: "ask",
    quantity: 1,
    price: "1",
    observed_at: "2026-11-18T12:00:00Z",
    key: "Az_~00",
  };
  const frame = (target: unknown, key = row.key) => ({
    type: "orders.snapshot",
    id: "flat",
    sequence: 0,
    data: [{ ...row, target, key }],
    range: { start: null, end: null },
  });
  for (const target of [
    { kind: "gift", id: "gift", traits: {} },
    { kind: "gift", id: "gift", number: 42, traits: { model: "m" } },
    { kind: "criteria", models: [], backdrops: [], patterns: [] },
    {
      kind: "unsupported",
      code: "invalid_gift_id",
      message: "Gift unavailable",
    },
  ])
    assert.deepEqual(parseFrame(JSON.stringify(frame(target))), frame(target));
  for (const target of [
    { kind: "gift", gift: { id: "gift", collection_id: "gift", traits: {} } },
    { kind: "gift", id: "gift", collection_id: "gift", traits: {} },
    { kind: "gift", id: "gift", number: 0, traits: {} },
    { kind: "criteria", criteria: { models: [], backdrops: [], patterns: [] } },
    {
      kind: "unsupported",
      reason: { code: "invalid_gift_id", message: "Gift unavailable" },
    },
  ])
    assert.throws(() => parseFrame(JSON.stringify(frame(target))));
  for (const key of ["A", "AAAAA", "AA==", "a-b", "ab+c", "ab/c", "ab{10}"]) {
    assert.throws(() =>
      parseFrame(
        JSON.stringify(frame({ kind: "gift", id: "gift", traits: {} }, key)),
      ),
    );
  }
});

test("target valuations validate scope identity, independent outcomes and unique strategies", () => {
  const value = {
    type: "result",
    id: "targets.1",
    method: "valuation.query",
    data: {
      target: { kind: "model", collection_id: "gift", model_id: "gold" },
      currency_id: "GRAM",
      evaluated_at: "2026-11-20T00:00:00Z",
      stale: false,
      results: [
        {
          strategy: "ask_floor",
          data: { price: { value: "1.25" } },
          revision: "r1",
        },
        {
          strategy: "execution_mean",
          error: { code: "capacity_exceeded", message: "Shorten the period" },
          revision: "r1",
        },
      ],
    },
  };
  assert.deepEqual(parseFrame(JSON.stringify(value)), value);
  const snapshot = {
    type: "valuation.snapshot",
    id: value.id,
    data: value.data,
  };
  assert.deepEqual(parseFrame(JSON.stringify(snapshot)), snapshot);
  for (const revision of [null, "", "x".repeat(65)]) {
    const data = {
      ...value.data,
      results: [{ ...value.data.results[0], revision }],
    };
    assert.throws(() => parseFrame(JSON.stringify({ ...value, data })));
    assert.throws(() => parseFrame(JSON.stringify({ ...snapshot, data })));
  }
  const legacy = {
    ...value,
    data: {
      ...value.data,
      results: [{ ...value.data.results[0], display: { fields: [] } }],
    },
  };
  assert.throws(() => parseFrame(JSON.stringify(legacy)));
  for (const invalid of [
    {
      ...value,
      data: { ...value.data, target: { kind: "model", collection_id: "gift" } },
    },
    {
      ...value,
      data: {
        ...value.data,
        results: [value.data.results[0], value.data.results[0]],
      },
    },
    { ...value, data: { ...value.data, results: [] } },
    {
      ...value,
      data: {
        ...value.data,
        results: [
          { ...value.data.results[0], error: { code: "bad", message: "bad" } },
        ],
      },
    },
    { ...value, data: { ...value.data, revision: "private" } },
  ]) {
    assert.throws(() => parseFrame(JSON.stringify(invalid)));
    assert.throws(() =>
      parseFrame(JSON.stringify({ ...snapshot, data: invalid.data })),
    );
  }
});
