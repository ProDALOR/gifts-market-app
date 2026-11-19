import { collectionOptions } from "../catalog/selectors";
import { memo } from "react";
import type { TraitFilters, TraitKey, CatalogOption } from "../market";
import type { Collection } from "../orderbook";
import { TraitComposition } from "./trait-composition";
// Render only explicitly selected catalog traits.
export const TraitPreview = memo(function TraitPreview({
  collection,
  options,
  filters,
  reset,
}: {
  collection?: Collection;
  options: ReadonlyMap<string, CatalogOption>;
  filters: TraitFilters;
  reset: () => void;
}) {
  const selected = (id: TraitKey) => {
    const ids = new Set(filters[id] || []);
    return collectionOptions(collection, id, options).filter((option) =>
      ids.has(option.id),
    );
  };
  const models = selected("models"),
    backdrops = selected("backdrops"),
    patterns = selected("patterns");
  if (!models.length && !backdrops.length && !patterns.length) return null;
  return (
    <TraitComposition models={models} backdrops={backdrops} patterns={patterns}>
      <button className="reset-traits" onClick={reset}>
        Reset traits
      </button>
    </TraitComposition>
  );
});
