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

test("atomic windows validate identities, positions, bounds and anchor membership", () => {
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
    position: "p.a",
  };
  const frame = {
    type: "window",
    id: "op.1",
    sync,
    revision: "r",
    window: {
      data: [row],
      anchor: "p.a",
      has_before: true,
      has_after: true,
      cursor: "r",
    },
  };
  assert.deepEqual(parseFrame(JSON.stringify(frame)), frame);
  for (const window of [
    { ...frame.window, data: [row, row] },
    { ...frame.window, anchor: "missing" },
    { ...frame.window, data: [{ ...row, position: "" }] },
    { ...frame.window, data: [] },
    { ...frame.window, data: Array(201).fill(row) },
    { ...frame.window, data: [{ ...row, kind: "executed" }] },
  ])
    assert.throws(() => parseFrame(JSON.stringify({ ...frame, window })));
});
