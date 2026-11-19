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
  method: "valuation.evaluate",
  data: {
    strategy: "floor",
    evaluated_at: "2026-11-18T12:00:00Z",
    currency_id: "GRAM",
    gift: {
      id: "1",
      collection_id: "gift",
      number: 1,
      traits: { model: null, backdrop: null, pattern: null },
    },
    stale: false,
    display: {
      summary: null,
      note: "Exact amounts",
      fields: [
        {
          id: "final",
          label: "Estimate",
          primary: true,
          value: {
            kind: "currency",
            amount: "100000000000000000.000000001",
            currency_id: "GRAM",
          },
        },
      ],
    },
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
    { ...result, data: { ...result.data, gift: null } },
    {
      ...result,
      data: {
        ...result.data,
        display: { fields: [{ value: { kind: "currency", amount: NaN } }] },
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
    key: "1001TW00OG00W0", // v4 identity key: asks, venue v, order a.
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
    { ...frame, snapshot: "unused" },
    { ...frame, sequence: Number.MAX_SAFE_INTEGER + 1 },
    { ...frame, data: [row, row] },
    { ...frame, data: [{ ...row, key: "" }] },
    {
      ...frame,
      data: [row],
      range: { start: "1001TW00OW00W0", end: null },
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
    data: { data: frame.data, range: frame.range, next: null },
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
  const data = { data: [row], range: { start: null, end: null }, next: null };
  const finite = { type: "result", method: "orders.query", id: "q", data };
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
      { ...finite, data: { ...data, [field]: null } },
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
