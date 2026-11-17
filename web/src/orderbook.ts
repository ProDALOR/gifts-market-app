import type { Entry, Order, Predicate } from "./market.ts";
export type CatalogOption = Extract<Entry, { kind: "attribute_option" }>;
export type Collection = Extract<Entry, { kind: "item_type" }>;
export const traits = [
  { id: "tg.model", label: "Model" },
  { id: "tg.backdrop", label: "Backdrop" },
  { id: "tg.pattern", label: "Symbol" },
] as const;
export function selectedOptions(predicate?: Predicate): string[] {
  if (predicate?.op === "eq" && predicate.value?.type === "enum")
    return [predicate.value.value];
  if (predicate?.op === "one_of" && Array.isArray(predicate.values))
    return predicate.values.flatMap((value) =>
      value?.type === "enum" && typeof value.value === "string"
        ? [value.value]
        : [],
    );
  return [];
}
export function optionPredicate(ids: string[]): Predicate | undefined {
  const values = [...new Set(ids)].map((value) => ({
    type: "enum" as const,
    value,
  }));
  return values.length === 0
    ? undefined
    : values.length === 1
      ? { op: "eq", value: values[0] }
      : { op: "one_of", values };
}
export function collectionOptions(
  collection: Collection | undefined,
  id: string,
  options: ReadonlyMap<string, CatalogOption>,
) {
  const rule = collection?.attributes[id];
  if (!rule) return [];
  const allowed = rule.allowed_values
    ? new Set(rule.allowed_values)
    : undefined;
  return [...options.values()]
    .filter(
      (option) =>
        option.attribute_id === id && (!allowed || allowed.has(option.id)),
    )
    .sort((a, b) => a.name.localeCompare(b.name));
}
export function serial(order: Order): string | undefined {
  const value = order.attributes["tg.number"];
  return value?.op === "eq" && ["integer", "text"].includes(value.value.type)
    ? String(value.value.value)
    : undefined;
}
export function modelName(
  order: Order,
  options: ReadonlyMap<string, CatalogOption>,
  fallback: string,
) {
  const id = selectedOptions(order.attributes["tg.model"]);
  return id.length === 1 ? options.get(id[0])?.name || fallback : fallback;
}
export function listingLink(order: Order): string | undefined {
  for (const key of ["url", "listing_url", "market_url", "link"]) {
    const value = order.meta[key];
    if (!value) continue;
    try {
      const url = new URL(value);
      if (
        ["http:", "https:"].includes(url.protocol) &&
        !url.username &&
        !url.password
      )
        return url.href;
    } catch {}
  }
}
