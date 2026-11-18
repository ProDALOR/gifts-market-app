import type { components } from "./gen/api.ts";
export type LiveFrame = components["schemas"]["ServerFrame"];
export type RequestCall = components["schemas"]["ApiRequest"];
export type RequestResult = components["schemas"]["ApiResult"];
export type SubscriptionCommand = Extract<
  components["schemas"]["Command"],
  { type: "subscribe_window" | "watch_catalog" }
>;
const record = (value: unknown): value is Record<string, unknown> =>
  !!value && typeof value === "object" && !Array.isArray(value);
const text = (value: unknown): value is string => typeof value === "string";
const oneOf = (value: unknown, choices: readonly string[]) =>
  text(value) && choices.includes(value);
const nullableText = (value: unknown) => value == null || text(value);
const strings = (value: unknown) => Array.isArray(value) && value.every(text);
const id = (value: unknown) =>
  text(value) && /^[A-Za-z0-9_.-]{1,64}$/.test(value);
const integer = (value: unknown) =>
  typeof value === "number" && Number.isSafeInteger(value) && value > 0;
const sync = (value: unknown) =>
  record(value) &&
  typeof value.ready === "boolean" &&
  oneOf(value.state, ["starting", "syncing", "live", "stale"]) &&
  nullableText(value.updated_at);
const traits = (value: unknown) =>
  record(value) &&
  [value.model, value.backdrop, value.pattern].every(nullableText);
const gift = (value: unknown) =>
  record(value) &&
  text(value.id) &&
  text(value.collection_id) &&
  (value.number == null || integer(value.number)) &&
  traits(value.traits);
const range = (value: unknown) =>
  value == null ||
  (record(value) &&
    [value.min, value.max].every((n) => n == null || integer(n)));
function order(value: unknown) {
  if (
    !record(value) ||
    !text(value.id) ||
    !record(value.book) ||
    ![
      value.book.collection_id,
      value.book.currency_id,
      value.book.venue_id,
    ].every(text) ||
    !text(value.price) ||
    !text(value.observed_at) ||
    ![value.executed_at, value.expires_at, value.market_url].every(
      nullableText,
    ) ||
    !integer(value.quantity) ||
    !oneOf(value.kind, ["ask", "bid", "executed"]) ||
    !record(value.target)
  )
    return false;
  switch (value.target.kind) {
    case "gift":
      return gift(value.target.gift);
    case "criteria": {
      const c = value.target.criteria;
      return (
        record(c) &&
        strings(c.models) &&
        strings(c.backdrops) &&
        strings(c.patterns) &&
        range(c.number)
      );
    }
    case "unsupported":
      return (
        record(value.target.reason) &&
        text(value.target.reason.code) &&
        text(value.target.reason.message)
      );
    default:
      return false;
  }
}
function display(value: unknown) {
  if (
    !record(value) ||
    !Array.isArray(value.fields) ||
    !text(value.note) ||
    !nullableText(value.summary)
  )
    return false;
  return value.fields.every((field) => {
    if (
      !record(field) ||
      !text(field.id) ||
      !text(field.label) ||
      typeof field.primary !== "boolean" ||
      !(
        field.attribute == null ||
        oneOf(field.attribute, ["model", "backdrop", "pattern"])
      ) ||
      !record(field.value)
    )
      return false;
    const v = field.value;
    switch (v.kind) {
      case "currency":
        return text(v.amount) && text(v.currency_id);
      case "number":
        return text(v.amount) && text(v.suffix);
      case "unavailable":
        return text(v.label) && text(v.message) && nullableText(v.detail);
      default:
        return false;
    }
  });
}
/** Validate envelopes and the data the GUI consumes; calculations remain server-owned typed results. */
export function parseFrame(raw: unknown): LiveFrame {
  if (!text(raw) || raw.length > 8 * 1024 * 1024)
    throw new Error("Invalid WebSocket message");
  const value: unknown = JSON.parse(raw);
  if (
    !record(value) ||
    !(id(value.id) || (value.type === "error" && value.id === ""))
  )
    throw new Error("Invalid operation envelope");
  let valid = false;
  switch (value.type) {
    case "status":
      valid = value.state === "connecting";
      break;
    case "command_error":
    case "error":
      valid =
        record(value.error) &&
        text(value.error.code) &&
        text(value.error.message) &&
        (value.error.details == null || record(value.error.details));
      break;
    case "sync":
      valid = sync(value.sync) && text(value.revision);
      break;
    case "window": {
      const w = value.window;
      if (
        record(w) &&
        Array.isArray(w.data) &&
        w.data.length <= 200 &&
        w.data.every(
          (row) =>
            order(row) &&
            record(row) &&
            oneOf(row.kind, ["ask", "bid"]) &&
            text(row.position) &&
            row.position.length > 0 &&
            row.position.length <= 8192,
        )
      ) {
        const rows = w.data as Record<string, unknown>[];
        const positions = new Set(rows.map((row) => row.position));
        const identities = new Set(
          rows.map((row) => {
            const book = row.book as Record<string, unknown>;
            return JSON.stringify([book.venue_id, row.id]);
          }),
        );
        valid =
          positions.size === rows.length &&
          identities.size === rows.length &&
          (rows.length
            ? text(w.anchor) && positions.has(w.anchor)
            : w.anchor == null && !w.has_before && !w.has_after) &&
          typeof w.has_before === "boolean" &&
          typeof w.has_after === "boolean" &&
          text(w.cursor) &&
          sync(value.sync) &&
          text(value.revision);
      }
      break;
    }
    case "catalog":
      valid =
        record(value.versions) &&
        text(value.versions.directory) &&
        record(value.versions.collections) &&
        Object.values(value.versions.collections).every(nullableText);
      break;
    case "result": {
      const d = value.data;
      valid =
        value.method === "valuation.evaluate" &&
        record(d) &&
        text(d.strategy) &&
        text(d.revision) &&
        text(d.evaluated_at) &&
        text(d.currency_id) &&
        gift(d.gift) &&
        sync(d.sync) &&
        display(d.display);
      break;
    }
  }
  if (!valid) throw new Error("Invalid WebSocket frame");
  return value as LiveFrame;
}
