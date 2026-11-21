import type { CatalogOption, TraitFilters } from "../market.ts";
export const traits = [
  { id: "models", label: "Model", kind: "model" },
  { id: "backdrops", label: "Backdrop", kind: "backdrop" },
  { id: "patterns", label: "Symbol", kind: "pattern" },
] as const;

/** Preserve restricted IDs even when catalog entries are missing or have the wrong kind. */
export function resolveTraits(
  selection: TraitFilters,
  options: ReadonlyMap<string, CatalogOption>,
  names?: ReadonlyMap<string, string>,
) {
  return traits.map((trait) => {
    const entries = (selection[trait.id] ?? []).map((id) => {
      const candidate = options.get(id);
      const option = candidate?.kind === trait.kind ? candidate : undefined;
      const name =
        option?.name ||
        (!candidate ? names?.get(id) : undefined) ||
        `Unavailable ${trait.label.toLowerCase()}`;
      return { id, name, option };
    });
    const available = entries.flatMap((entry) =>
      entry.option ? [entry.option] : [],
    );
    return {
      ...trait,
      entries,
      available,
      missing: entries.length - available.length,
    };
  });
}
