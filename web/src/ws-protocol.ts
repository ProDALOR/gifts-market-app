import type { components } from "./gen/api.ts";
export type LiveFrame = components["schemas"]["ServerFrame"];
export type RequestCall = components["schemas"]["ApiRequest"];
export type RequestResult = components["schemas"]["ApiResult"];
export type SubscriptionCommand = Extract<
  components["schemas"]["Command"],
  { type: "orders.watch" | "catalog.watch" | "valuation.watch" }
>;
const record = (value: unknown): value is Record<string, unknown> =>
  !!value && typeof value === "object" && !Array.isArray(value);
const text = (value: unknown): value is string => typeof value === "string";
const oneOf = (value: unknown, choices: readonly string[]) =>
  text(value) && choices.includes(value);
const nullableText = (value: unknown) => value == null || text(value);
const strings = (value: unknown) => Array.isArray(value) && value.every(text);
const legacyFields = [
  "sync",
  "revision",
  "cursor",
  "page_token",
  "next_page_token",
  "next",
  "base_sequence",
  "reset",
  "position",
  "sort_key",
] as const;
const hasNoLegacyFields = (value: Record<string, unknown>) =>
  legacyFields.every((field) => !Object.hasOwn(value, field));
const id = (value: unknown) =>
  text(value) && /^[A-Za-z0-9_.-]{1,64}$/.test(value);
const integer = (value: unknown) =>
  typeof value === "number" && Number.isSafeInteger(value) && value > 0;
const marketStatus = (value: unknown) =>
  oneOf(value, ["available", "delayed", "unavailable"]);
const traits = (value: unknown) =>
  record(value) &&
  [value.model, value.backdrop, value.pattern].every(nullableText);
const giftFacts = (value: unknown) =>
  record(value) &&
  text(value.id) &&
  (value.number == null || integer(value.number)) &&
  traits(value.traits);
const gift = (value: unknown) =>
  giftFacts(value) && record(value) && text(value.collection_id);
const valuationTarget = (value: unknown) => {
  if (
    !record(value) ||
    (value.kind !== "gift" &&
      value.kind !== "catalog" &&
      !text(value.collection_id))
  )
    return false;
  if (value.kind !== "criteria" && value.criteria != null) return false;
  switch (value.kind) {
    case "catalog":
      return (
        strings(value.collection_ids) &&
        value.collection_ids.length > 0 &&
        value.collection_ids.length <= 8 &&
        new Set(value.collection_ids).size === value.collection_ids.length &&
        (value.details == null ||
          (record(value.details) &&
            text(value.details.collection_id) &&
            value.collection_ids.includes(value.details.collection_id) &&
            (value.details.pair == null ||
              (record(value.details.pair) &&
                text(value.details.pair.model_id) &&
                text(value.details.pair.backdrop_id)))))
      );
    case "collection":
      return (
        value.model_id == null &&
        value.backdrop_id == null &&
        value.gift == null
      );
    case "model":
      return (
        text(value.model_id) && value.backdrop_id == null && value.gift == null
      );
    case "backdrop":
      return (
        text(value.backdrop_id) && value.model_id == null && value.gift == null
      );
    case "model_backdrop":
      return (
        text(value.model_id) && text(value.backdrop_id) && value.gift == null
      );
    case "gift":
      return (
        gift(value.gift) &&
        nullableText(value.slug) &&
        value.collection_id === undefined &&
        value.model_id == null &&
        value.backdrop_id == null
      );
    case "criteria":
      return (
        value.model_id == null &&
        value.backdrop_id == null &&
        value.gift == null &&
        criteria(value.criteria)
      );
    default:
      return false;
  }
};
const range = (value: unknown) =>
  value == null ||
  (record(value) &&
    [value.min, value.max].every((n) => n == null || integer(n)));
function criteria(value: unknown) {
  return (
    record(value) &&
    strings(value.models) &&
    strings(value.backdrops) &&
    strings(value.patterns) &&
    range(value.number)
  );
}
function valuationReport(d: unknown) {
  return (
    record(d) &&
    hasNoLegacyFields(d) &&
    valuationTarget(d.target) &&
    text(d.currency_id) &&
    text(d.evaluated_at) &&
    typeof d.stale === "boolean" &&
    Array.isArray(d.results) &&
    d.results.length > 0 &&
    d.results.length <= 8 &&
    d.results.every(
      (row) =>
        record(row) &&
        text(row.strategy) &&
        display(row.display) &&
        ((record(row.data) && !Object.hasOwn(row, "error")) ||
          (record(row.error) &&
            text(row.error.code) &&
            text(row.error.message) &&
            !Object.hasOwn(row, "data"))),
    ) &&
    new Set(d.results.map((row) => record(row) && row.strategy)).size ===
      d.results.length
  );
}

function order(value: unknown) {
  if (
    !record(value) ||
    !hasNoLegacyFields(value) ||
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
  const target = value.target;
  if (
    ["gift", "criteria", "reason", "collection_id"].some((field) =>
      Object.hasOwn(target, field),
    )
  )
    return false;
  switch (target.kind) {
    case "gift":
      return giftFacts(target);
    case "criteria": {
      return criteria(target);
    }
    case "unsupported":
      return text(target.code) && text(target.message);
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
      (field.basis != null && typeof field.basis !== "boolean") ||
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
const sequence = (value: unknown): value is number =>
  typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
const orderingKey = (value: unknown): value is string =>
  text(value) &&
  value.length >= 2 &&
  value.length <= 8192 &&
  value.length % 4 !== 1 &&
  /^[0-9A-Za-z_~]+$/.test(value);
const boundary = (value: unknown) => value === null || orderingKey(value);
function orderedRows(value: unknown): value is Record<string, unknown>[] {
  if (
    !Array.isArray(value) ||
    !value.every((row) => order(row) && record(row) && orderingKey(row.key))
  )
    return false;
  const rows = value as Record<string, unknown>[];
  return (
    new Set(
      rows.map((row) =>
        JSON.stringify([
          (row.book as Record<string, unknown>).venue_id,
          row.id,
        ]),
      ),
    ).size === rows.length &&
    new Set(rows.map((row) => row.key)).size === rows.length &&
    rows.every(
      (row, i) => i === 0 || (rows[i - 1].key as string) < (row.key as string),
    )
  );
}
function snapshot(value: unknown) {
  if (
    !record(value) ||
    !hasNoLegacyFields(value) ||
    !orderedRows(value.data) ||
    !record(value.range)
  )
    return false;
  const { start, end } = value.range;
  return (
    boundary(start) &&
    boundary(end) &&
    !(text(start) && text(end) && start > end) &&
    value.data.every(
      (row) =>
        (start === null || (row.key as string) >= (start as string)) &&
        (end === null || (row.key as string) < (end as string)),
    )
  );
}
function readResult(value: unknown) {
  return snapshot(value) && record(value) && nullableText(value.snapshot);
}
/** Validate envelopes and the data the GUI consumes; calculations remain server-owned typed results. */
export function parseFrame(raw: unknown): LiveFrame {
  if (!text(raw) || raw.length > 8 * 1024 * 1024)
    throw new Error("Invalid WebSocket message");
  const value: unknown = JSON.parse(raw);
  if (
    !record(value) ||
    !hasNoLegacyFields(value) ||
    !(
      id(value.id) ||
      (value.type === "command_error" && value.id === "") ||
      (value.type === "market.status" && value.id === undefined)
    )
  )
    throw new Error("Invalid operation envelope");
  let valid = false;
  switch (value.type) {
    case "command_error":
    case "error":
      valid =
        record(value.error) &&
        text(value.error.code) &&
        text(value.error.message) &&
        (value.error.details == null || record(value.error.details));
      break;
    case "market.status":
      valid = value.id === undefined && marketStatus(value.status);
      break;
    case "orders.snapshot":
      valid =
        sequence(value.sequence) &&
        snapshot(value) &&
        value.snapshot === undefined;
      break;
    case "orders.changed": {
      const removed = value.removed;
      const upserts = value.upserts;
      valid =
        sequence(value.sequence) &&
        value.range === undefined &&
        value.snapshot === undefined &&
        orderedRows(upserts) &&
        Array.isArray(removed) &&
        removed.every(
          (row) => record(row) && text(row.venue_id) && text(row.id),
        ) &&
        new Set(removed.map((row) => JSON.stringify([row.venue_id, row.id])))
          .size === removed.length &&
        !removed.some((row) =>
          (upserts as Record<string, unknown>[]).some(
            (upsert) =>
              row.id === upsert.id &&
              row.venue_id ===
                (upsert.book as Record<string, unknown>).venue_id,
          ),
        );
      break;
    }
    case "catalog.versions":
      valid =
        record(value.versions) &&
        text(value.versions.directory) &&
        record(value.versions.collections) &&
        Object.values(value.versions.collections).every(nullableText);
      break;
    case "valuation.snapshot":
      valid = valuationReport(value.data);
      break;
    case "result": {
      const d = value.data;
      valid =
        value.method === "orders.query"
          ? readResult(value)
          : value.method === "valuation.query"
            ? valuationReport(d)
            : false;
      break;
    }
  }
  if (!valid) throw new Error("Invalid WebSocket frame");
  return value as LiveFrame;
}
