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
    revision: "r",
    evaluated_at: "2026-11-18T12:00:00Z",
    currency_id: "GRAM",
    gift: {
      id: "1",
      collection_id: "gift",
      number: 1,
      traits: { model: null, backdrop: null, pattern: null },
    },
    sync,
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
      type: "catalog",
      id: "op.3",
      versions: { directory: "d", collections: { gift: null } },
    },
    {
      type: "error",
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
      criteria: { models: [], backdrops: [], patterns: [], number: null },
    },
    key: "000001",
  };
  const frame = {
    type: "orders.snapshot",
    id: "op.1",
    sequence: 0,
    reset: false,
    sync,
    revision: "r",
    data: [row],
    next: null,
    range: { start: null, end: null },
  };
  assert.deepEqual(parseFrame(JSON.stringify(frame)), frame);
  for (const invalid of [
    { ...frame, sequence: -1 },
    { ...frame, sequence: Number.MAX_SAFE_INTEGER + 1 },
    { ...frame, data: [row, row] },
    { ...frame, data: [{ ...row, key: "" }] },
    {
      ...frame,
      data: [row],
      range: { start: "000002", end: null },
    },
    { ...frame, range: { start: null } },
  ])
    assert.throws(() => parseFrame(JSON.stringify(invalid)));
  const change = {
    type: "orders.changed",
    id: "op.1",
    sequence: 1,
    base_sequence: 0,
    upserts: [row],
    removed: [],
    sync,
    revision: "r2",
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
