import type { components } from "../gen/api";
const record = (v: unknown): v is Record<string, unknown> =>
  !!v && typeof v === "object" && !Array.isArray(v);
function quote(v: unknown) {
  if (!record(v)) return false;
  if ("error" in v)
    return (
      !Object.hasOwn(v, "value") &&
      v.order == null &&
      record(v.error) &&
      typeof v.error.code === "string" &&
      typeof v.error.message === "string"
    );
  return (
    typeof v.value === "string" &&
    v.value.length <= 80 &&
    /[1-9]/.test(v.value) &&
    /^\d+(?:\.\d+)?$/.test(v.value) &&
    record(v.order) &&
    typeof v.order.id === "string" &&
    typeof v.order.venue_id === "string" &&
    Number.isSafeInteger(v.order.quantity) &&
    Number(v.order.quantity) > 0 &&
    (v.order.expires_at == null ||
      (typeof v.order.expires_at === "string" &&
        Number.isFinite(Date.parse(v.order.expires_at))))
  );
}
/** The typed consumer validates its strategy-owned payload before rendering nested quotes. */
export function isBestBid(
  value: unknown,
): value is components["schemas"]["BestBidValuation"] {
  if (!record(value) || !quote(value.best) || !record(value.references))
    return false;
  const references = value.references;
  return (
    quote(references.collection) &&
    ["model", "backdrop", "model_backdrop", "pattern"].every(
      (key) => references[key] == null || quote(references[key]),
    )
  );
}
