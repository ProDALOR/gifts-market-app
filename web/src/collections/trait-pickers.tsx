import { collectionOptions } from "../catalog/selectors";
import {
  rarityGroups,
  groupSelection,
  toggleRarityGroup,
} from "../catalog/rarity";
import { memo, useEffect, useMemo, useRef, useState } from "react";
import type { TraitFilters, TraitKey } from "../market";
import { traits } from "../orderbook";
import type { CatalogOption, Collection } from "../orderbook";
import { OptionArt } from "../previews/option-art";
import { TraitFloor } from "../valuation/floors";
import { usePickerMotion } from "./picker-motion";
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
  const { shown, shell, panel, content } = usePickerMotion(open, query);
  const host = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    // This panel participates in layout. Close after the outside click has
    // reached its control, so collapsing it cannot move that control mid-click.
    const outside = (event: MouseEvent) => {
      if (!host.current?.contains(event.target as Node)) setOpen(undefined);
    };
    document.addEventListener("click", outside);
    return () => {
      document.removeEventListener("click", outside);
    };
  }, [open, setOpen]);
  const trait = traits.find((trait) => trait.id === shown);
  const available = shown ? collectionOptions(collection, shown, options) : [];
  const chosen = shown ? filters[shown] || [] : [];
  const groups = useMemo(
    () => rarityGroups(available, query),
    [available, query],
  );
  return (
    <div
      className="trait-controls"
      ref={host}
      onKeyDown={(event) => {
        if (
          open &&
          event.key === "Escape" &&
          !event.defaultPrevented &&
          !event.nativeEvent.isComposing
        ) {
          event.preventDefault();
          event.stopPropagation();
          setOpen(undefined);
          host.current
            ?.querySelector<HTMLButtonElement>(`#pick-${open}`)
            ?.focus({ preventScroll: true });
        }
      }}
    >
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
                const next = open === trait.id ? undefined : trait.id;
                setOpen(next);
                if (next) setQuery("");
              }}
            >
              {trait.label}
              {count > 0 && (
                <span key={count} className="pick-count">
                  {count}
                </span>
              )}
            </button>
          );
        })}
      </div>
      {shown && (
        <div
          className="trait-panel-shell"
          ref={shell}
          inert={!open}
          aria-hidden={!open}
        >
          <section
            ref={panel}
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
              ref={content}
              className="trait-options"
              role="group"
              aria-label={`${trait?.label} options`}
            >
              <button
                type="button"
                className="trait-option"
                aria-pressed={!chosen.length}
                onClick={() => change(shown, undefined)}
              >
                Any
              </button>
              {groups.map((group) => (
                <div
                  className="trait-group"
                  role="group"
                  aria-label={`${group.label} rarity`}
                  key={group.key}
                >
                  <button
                    type="button"
                    className="trait-group-toggle"
                    aria-pressed={groupSelection(group.options, chosen)}
                    aria-label={`${group.label} rarity`}
                    title="Select or deselect all matching attributes in this group"
                    onClick={() =>
                      change(shown, toggleRarityGroup(group.options, chosen))
                    }
                  >
                    <span className="trait-group-label">{group.label}</span>
                    <span className="trait-group-count">
                      {group.options.length}
                    </span>
                  </button>
                  <div className="trait-group-options">
                    {group.options.map((option) => (
                      <button
                        key={option.id}
                        className="trait-option"
                        aria-label={option.name}
                        aria-pressed={chosen.includes(option.id)}
                        onClick={() =>
                          change(
                            shown,
                            chosen.includes(option.id)
                              ? chosen.filter((id) => id !== option.id)
                              : [...chosen, option.id],
                          )
                        }
                      >
                        <OptionArt option={option} />
                        <span className="trait-option-copy">
                          <span className="trait-option-name">
                            {option.name}
                          </span>
                          {option.kind !== "pattern" && (
                            <span className="trait-option-value">
                              <TraitFloor
                                rounded
                                collection={collection?.id}
                                id={option.id}
                                kind={option.kind}
                              />
                            </span>
                          )}
                        </span>
                      </button>
                    ))}
                  </div>
                </div>
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
              {!!available.length && !groups.length && (
                <span className="rail-empty">No matching traits</span>
              )}
            </div>
          </section>
        </div>
      )}
    </div>
  );
});
