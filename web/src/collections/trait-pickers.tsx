import { collectionOptions } from "../catalog/selectors";
import { memo, useEffect, useRef, useState } from "react";
import type { Predicate } from "../market";
import { optionPredicate, selectedOptions, traits } from "../orderbook";
import type { CatalogOption, Collection } from "../orderbook";
function OptionArt({ option }: { option: CatalogOption }) {
  if (option.attribute_id === "tg.backdrop") {
    const colors = [
      option.meta["colors.center_color"],
      option.meta["colors.edge_color"],
    ];
    return (
      <span
        className="option-art swatch"
        style={
          colors.every((color) => /^#[\da-f]{6}$/i.test(color || ""))
            ? {
                background: `radial-gradient(ellipse at 40% 30%,${colors[0]},${colors[1]})`,
              }
            : undefined
        }
      />
    );
  }
  return (
    <span
      className={`option-art${option.attribute_id === "tg.pattern" ? " symbol-thumb" : ""}`}
    >
      <img
        alt=""
        loading="lazy"
        decoding="async"
        src={`/media/${option.attribute_id === "tg.pattern" ? "patterns" : "models"}/${encodeURIComponent(option.id)}`}
        onError={(event) => {
          event.currentTarget.style.visibility = "hidden";
        }}
      />
    </span>
  );
}
export const TraitPickers = memo(function TraitPickers({
  collection,
  loading = false,
  loadError = "",
  options,
  filters,
  disabled,
  change,
  open,
  setOpen,
}: {
  collection?: Collection;
  loading?: boolean;
  loadError?: string;
  options: ReadonlyMap<string, CatalogOption>;
  filters: Record<string, Predicate>;
  disabled: boolean;
  open?: string;
  setOpen: (id: string | undefined) => void;
  change: (id: string, value?: Predicate) => void;
}) {
  const [query, setQuery] = useState("");
  const host = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const pointer = (event: PointerEvent) => {
      if (!host.current?.contains(event.target as Node)) setOpen(undefined);
    };
    const key = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setOpen(undefined);
        document.getElementById(`pick-${open}`)?.focus();
      }
    };
    document.addEventListener("pointerdown", pointer);
    document.addEventListener("keydown", key);
    return () => {
      document.removeEventListener("pointerdown", pointer);
      document.removeEventListener("keydown", key);
    };
  }, [open]);
  const trait = traits.find((trait) => trait.id === open);
  const available = open ? collectionOptions(collection, open, options) : [];
  const chosen = open ? selectedOptions(filters[open]) : [];
  return (
    <div className="trait-controls" ref={host}>
      <div className="pickers">
        {traits.map((trait) => {
          const count = selectedOptions(filters[trait.id]).length;
          return (
            <button
              key={trait.id}
              id={`pick-${trait.id}`}
              className={`pick-trigger${count ? " active" : ""}`}
              aria-label={trait.label}
              aria-expanded={open === trait.id}
              aria-controls="trait-panel"
              disabled={disabled || !collection?.attributes[trait.id]}
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
                  aria-pressed={chosen.includes(option.id)}
                  onClick={() =>
                    change(
                      open,
                      optionPredicate(
                        chosen.includes(option.id)
                          ? chosen.filter((id) => id !== option.id)
                          : [...chosen, option.id],
                      ),
                    )
                  }
                >
                  <OptionArt option={option} />
                  {option.name}
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
