import { collectionOptions } from "../catalog/selectors";
import { memo, useEffect, useRef, useState } from "react";
import type { TraitFilters, TraitKey } from "../market";
import { traits } from "../orderbook";
import type { CatalogOption, Collection } from "../orderbook";
import { OptionArt } from "../previews/option-art";
import { TraitFloor } from "../valuation/floors";
export const TraitPickers = memo(function TraitPickers({
  collection,
  loading = false,
  loadError = "",
  options,
  filters,
  change,
  open,
  setOpen,
}: {
  collection?: Collection;
  loading?: boolean;
  loadError?: string;
  options: ReadonlyMap<string, CatalogOption>;
  filters: TraitFilters;
  open?: TraitKey;
  setOpen: (id: TraitKey | undefined) => void;
  change: (id: TraitKey, value?: string[]) => void;
}) {
  const [query, setQuery] = useState("");
  const host = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    // This panel participates in layout. Close after the outside click has
    // reached its control, so collapsing it cannot move that control mid-click.
    const outside = (event: MouseEvent) => {
      if (!host.current?.contains(event.target as Node)) setOpen(undefined);
    };
    const key = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setOpen(undefined);
        document.getElementById(`pick-${open}`)?.focus();
      }
    };
    document.addEventListener("click", outside);
    document.addEventListener("keydown", key);
    return () => {
      document.removeEventListener("click", outside);
      document.removeEventListener("keydown", key);
    };
  }, [open]);
  const trait = traits.find((trait) => trait.id === open);
  const available = open ? collectionOptions(collection, open, options) : [];
  const chosen = open ? filters[open] || [] : [];
  return (
    <div className="trait-controls" ref={host}>
      <div className="pickers">
        {traits.map((trait) => {
          const count = (filters[trait.id] || []).length;
          return (
            <button
              key={trait.id}
              id={`pick-${trait.id}`}
              className={`pick-trigger${count ? " active" : ""}`}
              aria-label={trait.label}
              aria-expanded={open === trait.id}
              aria-controls="trait-panel"
              disabled={
                !collection ||
                collectionOptions(collection, trait.id, options).length === 0
              }
              onClick={() => {
                setOpen(open === trait.id ? undefined : trait.id);
                setQuery("");
              }}
            >
              {trait.label}
              {count > 0 && <span className="pick-count">{count}</span>}
            </button>
          );
        })}
      </div>
      {open && (
        <section
          id="trait-panel"
          className="picker-panel"
          aria-label={`${trait?.label} choices`}
        >
          <input
            className="trait-search"
            aria-label={`Search ${trait?.label.toLowerCase()} choices`}
            type="search"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder={`Search ${trait?.label.toLowerCase()}…`}
          />
          <div
            className="trait-options"
            role="group"
            aria-label={`${trait?.label} options`}
          >
            <button
              className="trait-option"
              aria-pressed={!chosen.length}
              onClick={() => change(open, undefined)}
            >
              Any
            </button>
            {available
              .filter((option) =>
                option.name.toLowerCase().includes(query.toLowerCase()),
              )
              .map((option) => (
                <button
                  key={option.id}
                  className="trait-option"
                  aria-label={option.name}
                  aria-pressed={chosen.includes(option.id)}
                  onClick={() =>
                    change(
                      open,
                      chosen.includes(option.id)
                        ? chosen.filter((id) => id !== option.id)
                        : [...chosen, option.id],
                    )
                  }
                >
                  <OptionArt option={option} />
                  {option.name}
                  <TraitFloor
                    collection={collection?.id}
                    id={option.id}
                    kind={option.kind}
                  />
                </button>
              ))}
            {loading && !available.length && (
              <span className="rail-empty">Loading choices…</span>
            )}
            {!loading && !available.length && (
              <span className="rail-empty">
                {loadError
                  ? "Could not load choices — retry above"
                  : "No choices"}
              </span>
            )}
            {!!available.length &&
              !available.some((option) =>
                option.name.toLowerCase().includes(query.toLowerCase()),
              ) && <span className="rail-empty">No matching traits</span>}
          </div>
        </section>
      )}
    </div>
  );
});
