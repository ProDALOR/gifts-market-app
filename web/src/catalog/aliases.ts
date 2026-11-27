import type { CatalogOption, Collection, TraitFilters } from "../market.ts";
type Named = {
  id: string;
  name: string;
  aliases?: { namespace: string; id: string }[];
};
export function aliasSelector(alias: { namespace: string; id: string }) {
  return `@${alias.namespace}:${alias.id}`;
}
export function matchesSelector(option: Named, selector: string) {
  if (!selector.startsWith("@")) return option.id === selector;
  const colon = selector.indexOf(":");
  if (colon < 2) return false;
  const namespace = selector.slice(1, colon),
    id = selector.slice(colon + 1);
  const kind = namespace === "tg.slug" ? "slug" : namespace;
  const equal = (a: string, b: string) =>
    ["name", "slug"].includes(kind)
      ? a.replace(/[A-Z]/g, (c) => c.toLowerCase()) ===
        b.replace(/[A-Z]/g, (c) => c.toLowerCase())
      : a === b;
  return (
    (kind === "name" && equal(option.name, id)) ||
    option.aliases?.some(
      (a) =>
        (a.namespace === "tg.slug" ? "slug" : a.namespace) === kind &&
        equal(a.id, id),
    ) === true
  );
}
export function resolveSelector<T extends Named>(
  options: readonly T[],
  selector: string,
) {
  const matches = options.filter((row) => matchesSelector(row, selector));
  return matches.length === 1 ? matches[0] : undefined;
}
export function preferredSelector(option: Named | undefined) {
  if (!option) return undefined;
  const alias =
    option.aliases?.find(
      (a) => a.namespace === "slug" || a.namespace === "tg.slug",
    ) ?? option.aliases?.find((a) => a.namespace === "name");
  return alias
    ? aliasSelector({
        ...alias,
        namespace: alias.namespace === "tg.slug" ? "slug" : alias.namespace,
      })
    : option.id;
}
export function aliasFilters(
  filters: TraitFilters,
  options: ReadonlyMap<string, CatalogOption>,
) {
  // Count names once per catalog, rather than allocating/scanning choices per selection.
  const names = new Map<string, string | null>();
  const encoder = new TextEncoder();
  const nameKey = (kind: string, name: string) =>
    `${kind}\0${name.replace(/[A-Z]/g, (c) => c.toLowerCase())}`;
  for (const option of options.values()) {
    if (
      !option.name ||
      option.name.trim() !== option.name ||
      /[\u0000-\u001f\u007f-\u009f]/.test(option.name) ||
      encoder.encode(option.name).length > 250
    )
      continue;
    const key = nameKey(option.kind, option.name);
    names.set(key, names.has(key) ? null : option.id);
  }
  return Object.fromEntries(
    Object.entries(filters).map(([key, ids]) => [
      key,
      ids?.map((id) => {
        const option = options.get(id);
        if (!option) return id;
        const preferred = preferredSelector(option)!;
        if (preferred !== option.id || option.id.startsWith("@"))
          return preferred;
        const name = `@name:${option.name}`;
        return names.get(nameKey(option.kind, option.name)) === option.id
          ? name
          : id;
      }),
    ]),
  ) as TraitFilters;
}
/** Resolve URL selectors only within the selected collection. Unknown/ambiguous values stay explicit. */
export function resolveFilterSelectors(
  filters: TraitFilters,
  options: ReadonlyMap<string, CatalogOption>,
): TraitFilters {
  const rows = [...options.values()];
  return Object.fromEntries(
    Object.entries(filters).map(([key, ids]) => [
      key,
      ids?.map(
        (id) =>
          resolveSelector(
            rows.filter(
              (row) =>
                row.kind ===
                { models: "model", backdrops: "backdrop", patterns: "pattern" }[
                  key
                ],
            ),
            id,
          )?.id ?? id,
      ),
    ]),
  );
}
/** Apply only after this collection's catalog is available, never to a loading catalog. */
export function availableFilterSelectors(
  filters: TraitFilters,
  options: ReadonlyMap<string, CatalogOption>,
): TraitFilters {
  if (!Object.values(filters).some((ids) => ids?.length)) return filters;
  const groups = {
    models: [] as CatalogOption[],
    backdrops: [] as CatalogOption[],
    patterns: [] as CatalogOption[],
  };
  for (const option of options.values())
    groups[
      { model: "models", backdrop: "backdrops", pattern: "patterns" }[
        option.kind
      ] as keyof typeof groups
    ].push(option);
  let changed = false;
  const available: TraitFilters = {};
  for (const key of Object.keys(groups) as (keyof typeof groups)[]) {
    const ids = filters[key];
    if (!ids) continue;
    const valid = ids.filter((id) => resolveSelector(groups[key], id));
    if (valid.length) available[key] = valid;
    if (valid.length !== ids.length) changed = true;
  }
  return changed ? available : filters;
}
export const allCollections: Collection = {
  id: "",
  name: "All gifts",
  slug: null,
  preview: null,
};
