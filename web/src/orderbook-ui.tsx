import { useCallback, useEffect, useRef, useState } from "react";
import type { CSSProperties, ReactNode } from "react";
import { amount } from "./market";
import type { Order, Predicate } from "./market";
import { GiftPreview } from "./gift-preview";
import { SelectPicker } from "./select-picker";
// Emit a same-origin file: the API's image policy does not allow data URLs.
import mrktLogo from "./icons/mrkt.svg?no-inline";
import { GiftBackdrop } from "./gift-backdrop";
import { backdropPalette } from "./gift-appearance";
import { balancedRows, previewLayout } from "./preview-layout";
import { observePreviewSize } from "./preview-environment";
import {
  usePreviewEnvironment,
  usePreviewVisibility,
} from "./preview-environment";
import {
  collectionOptions,
  modelName,
  optionPredicate,
  selectedOptions,
  serial,
  traits,
} from "./orderbook";
import type { CatalogOption, Collection } from "./orderbook";

const noOptions: ReadonlyMap<string, CatalogOption> = new Map();

export function CurrencySymbol({ currency }: { currency: string }) {
  if (currency !== "GRAM" && currency !== "XTR")
    return <small>{currency}</small>;
  return (
    <svg
      className={`currency-symbol ${currency === "XTR" ? "star" : "gram"}`}
      viewBox="0 0 24 24"
      role="img"
      aria-label={currency === "XTR" ? "Stars" : "GRAM"}
    >
      {currency === "XTR" ? (
        <path
          fill="currentColor"
          d="m12 2.5 2.93 5.94 6.56.95-4.75 4.63 1.12 6.53L12 17.47l-5.86 3.08 1.12-6.53L2.51 9.39l6.56-.95Z"
        />
      ) : (
        <path
          d="M5 4h14a2 2 0 0 1 1.7 3L12 21 3.3 7A2 2 0 0 1 5 4Zm7 0v17"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.7"
          strokeLinejoin="round"
        />
      )}
    </svg>
  );
}

export function SearchIcon() {
  return (
    <svg className="search-icon" viewBox="0 0 24 24" aria-hidden="true">
      <circle cx="10" cy="10" r="6.5" />
      <path d="m15 15 6 6" />
    </svg>
  );
}
export function VenueIcon({ name }: { name: string }) {
  const venue = name.toLowerCase();
  if (venue.includes("mrkt"))
    return (
      <img className="venue-logo" src={mrktLogo} alt="" aria-hidden="true" />
    );
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      {venue.includes("telegram") ? (
        <path fill="currentColor" d="M2 11 21 3l-4 18-6-5-3 3v-6l9-7-11 6z" />
      ) : venue.includes("portal") ? (
        <path
          fill="none"
          stroke="currentColor"
          strokeWidth="3"
          strokeLinejoin="round"
          d="M4 20V8l10-5 6 3v12l-6 3V9L4 14"
        />
      ) : venue === "all venues" ? (
        <g fill="currentColor">
          <rect x="3" y="3" width="8" height="8" rx="2" />
          <rect x="13" y="3" width="8" height="8" rx="2" />
          <rect x="3" y="13" width="8" height="8" rx="2" />
          <rect x="13" y="13" width="8" height="8" rx="2" />
        </g>
      ) : (
        <text
          x="12"
          y="17"
          textAnchor="middle"
          fill="currentColor"
          fontSize="16"
          fontWeight="700"
        >
          {name[0]?.toUpperCase()}
        </text>
      )}
    </svg>
  );
}
export function CollectionRail({
  collections,
  selected,
  select,
}: {
  collections: Collection[];
  selected: string;
  select: (id: string) => void;
}) {
  const [searching, setSearching] = useState(false),
    [query, setQuery] = useState("");
  const { reducedMotion } = usePreviewEnvironment();
  const rail = useRef<HTMLDivElement>(null),
    input = useRef<HTMLInputElement>(null),
    selectedButton = useRef<HTMLButtonElement>(null);
  const matches = collections.filter((item) =>
    item.name.toLowerCase().includes(query.trim().toLowerCase()),
  );
  const alignSelected = useCallback(() => {
    if (rail.current && selectedButton.current) {
      const left =
        selectedButton.current.getBoundingClientRect().left -
        rail.current.getBoundingClientRect().left +
        rail.current.scrollLeft;
      rail.current.scrollTo({
        left,
        behavior: reducedMotion ? "instant" : "smooth",
      });
    }
  }, [reducedMotion]);
  useEffect(() => {
    if (searching) {
      rail.current?.scrollTo({ left: 0 });
      input.current?.focus();
    } else {
      alignSelected();
    }
  }, [searching, selected, collections.length, alignSelected]);
  function close() {
    setSearching(false);
    setQuery("");
  }
  return (
    <nav
      className={`collection-rail${searching ? " searching" : ""}`}
      aria-label="Collections"
      onTransitionEnd={(event) => {
        // Width changes move the selected tile; align again at its final position.
        if (
          !searching &&
          event.propertyName === "width" &&
          (event.target as HTMLElement).classList.contains("collection-tile")
        )
          alignSelected();
      }}
      onKeyDown={(event) => {
        if (event.key === "Escape" && searching) {
          close();
          event.currentTarget
            .querySelector<HTMLButtonElement>(".search-toggle")
            ?.focus();
        }
      }}
    >
      <div className={`search-tile${searching ? " open" : ""}`}>
        <button
          className="search-toggle"
          aria-label="Search collections"
          aria-expanded={searching}
          aria-controls="collection-query"
          onClick={() => setSearching(true)}
          tabIndex={searching ? -1 : 0}
        >
          <SearchIcon />
        </button>
        <input
          ref={input}
          id="collection-query"
          type="search"
          hidden={!searching}
          aria-label="Filter collections"
          placeholder="Search"
          autoComplete="off"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          onKeyDown={(event) => {
            if (
              event.key === "Enter" &&
              !event.nativeEvent.isComposing &&
              matches[0]
            ) {
              event.preventDefault();
              select(matches[0].id);
              close();
            }
          }}
        />
        {searching && (
          <button
            className="search-close"
            aria-label="Close collection search"
            onClick={close}
          >
            ×
          </button>
        )}
      </div>
      <div className="collection-list" ref={rail}>
        {matches.map((item, index) => {
          const model = item.attributes["tg.model"]?.allowed_values?.[0];
          return (
            <button
              ref={item.id === selected ? selectedButton : undefined}
              className={`collection-tile tile-color-${index % 6}`}
              key={item.id}
              aria-label={item.name}
              aria-pressed={item.id === selected}
              title={item.name}
              onClick={() => {
                select(item.id);
                close();
              }}
            >
              <span className="collection-art">
                {model ? (
                  <GiftPreview
                    order={{
                      attributes: {
                        "tg.model": {
                          op: "eq",
                          value: { type: "enum", value: model },
                        },
                      },
                    }}
                    options={noOptions}
                    fallback={item.name[0]}
                    autoplay
                  />
                ) : (
                  <span>{item.name[0]}</span>
                )}
              </span>
              <b>{item.name}</b>
            </button>
          );
        })}
        {!matches.length && (
          <span className="rail-empty" role="status">
            No collections found
          </span>
        )}
      </div>
    </nav>
  );
}
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
export function TraitPickers({
  collection,
  options,
  filters,
  disabled,
  change,
  open,
  setOpen,
}: {
  collection?: Collection;
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
            {!available.length && (
              <span className="rail-empty">Loading choices…</span>
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
}
// Render only explicitly selected catalog traits.
export function TraitPreview({
  collection,
  options,
  filters,
  reset,
}: {
  collection?: Collection;
  options: ReadonlyMap<string, CatalogOption>;
  filters: Record<string, Predicate>;
  reset: () => void;
}) {
  const selected = (id: string) => {
    const ids = new Set(selectedOptions(filters[id]));
    return collectionOptions(collection, id, options).filter((option) =>
      ids.has(option.id),
    );
  };
  const models = selected("tg.model"),
    backdrops = selected("tg.backdrop"),
    patterns = selected("tg.pattern");
  if (!models.length && !backdrops.length && !patterns.length) return null;
  return (
    <SelectedTraitPreview
      models={models}
      backdrops={backdrops}
      patterns={patterns}
      options={options}
      reset={reset}
    />
  );
}
function SelectedTraitPreview({
  models,
  backdrops,
  patterns,
  options,
  reset,
}: {
  models: CatalogOption[];
  backdrops: CatalogOption[];
  patterns: CatalogOption[];
  options: ReadonlyMap<string, CatalogOption>;
  reset: () => void;
}) {
  const host = useRef<HTMLElement>(null),
    visible = usePreviewVisibility(host);
  const [width, setWidth] = useState(358);
  useEffect(() => {
    const element = host.current;
    if (!element) return;
    const measure = () => setWidth(element.clientWidth);
    measure();
    return observePreviewSize(element, measure);
  }, []);
  const layout = previewLayout(
    width,
    models.length,
    backdrops.length,
    patterns.length,
  );
  const modelRows = balancedRows(models, layout.modelColumns),
    backdropRows = balancedRows(backdrops, layout.backdropColumns);
  const palette = backdropPalette(backdrops[0]) || {
    center: "#302d36",
    edge: "#302d36",
    pattern: "#c5afce",
  };
  return (
    <figure
      className="trait-preview"
      ref={host}
      aria-label="Virtual gift preview"
      style={
        {
          height: layout.height,
          "--preview-model-size": `${layout.modelSize}px`,
          "--preview-model-row-height": `${layout.modelRowHeight}px`,
        } as CSSProperties
      }
    >
      <div className="preview-bands" aria-hidden="true">
        {backdropRows.map((row, index) => (
          <div className="preview-backdrop-row" key={index}>
            {row.map((backdrop) => {
              const colors = backdropPalette(backdrop);
              return (
                <span className="preview-band" key={backdrop.id}>
                  {colors && (
                    <GiftBackdrop palette={colors} visible={visible} />
                  )}
                </span>
              );
            })}
          </div>
        ))}
      </div>
      {patterns.length > 0 && (
        <div className="preview-symbols" aria-hidden="true">
          <GiftBackdrop
            palette={palette}
            patternUrls={patterns.map(
              (option) => `/media/patterns/${encodeURIComponent(option.id)}`,
            )}
            visible={visible}
            layout="banner"
            background={false}
          />
        </div>
      )}
      {models.length > 0 && (
        <div className="preview-models">
          {modelRows.map((row, index) => (
            <div className="preview-model-row" key={index}>
              {row.map((model) => (
                <div className="preview-model" key={model.id}>
                  <GiftPreview
                    autoplay
                    order={{
                      attributes: {
                        "tg.model": {
                          op: "eq",
                          value: { type: "enum", value: model.id },
                        },
                      },
                    }}
                    options={options}
                    fallback={model.name[0] || "◇"}
                  />
                  <span title={model.name}>{model.name}</span>
                </div>
              ))}
            </div>
          ))}
        </div>
      )}
      <button className="reset-traits" onClick={reset}>
        Reset traits
      </button>
    </figure>
  );
}
export function Quote({
  price,
  currency,
}: {
  price: string;
  currency: string;
}) {
  return (
    <>
      <CurrencySymbol currency={currency} />
      <span className={`quote-amount${price.length > 9 ? " precise" : ""}`}>
        {amount(price)}
      </span>
    </>
  );
}
export function GiftCard({
  order,
  options,
  collectionName,
  venueName,
  selected,
  open,
}: {
  order: Order;
  options: ReadonlyMap<string, CatalogOption>;
  collectionName: string;
  venueName: string;
  selected: boolean;
  open: () => void;
}) {
  const number = serial(order);
  return (
    <li className={`gift-card${selected ? " selected" : ""}`}>
      <GiftPreview
        order={order}
        options={options}
        fallback={collectionName[0] || "◇"}
        className="card-preview"
      >
        <div className="card-head">
          <span className="serial">
            {number
              ? `#${number}`
              : order.quantity !== "1"
                ? `×${order.quantity}`
                : "Gift"}
          </span>
          <span className="venue-icon" title={venueName}>
            <VenueIcon name={venueName} />
          </span>
        </div>
        <div className="card-copy">
          <strong>{modelName(order, options, collectionName)}</strong>
          <div className={`quote price${order.mode === "bid" ? " bid" : ""}`}>
            <Quote price={order.price} currency={order.book.currency_id} />
          </div>
        </div>
        <button
          className="card-open"
          aria-label={`View ${collectionName}${number ? ` #${number}` : ""} on ${venueName}`}
          data-order-id={order.id}
          data-venue-id={order.book.venue_id}
          onClick={open}
          aria-haspopup="dialog"
        />
      </GiftPreview>
    </li>
  );
}
export function DetailSheet({
  close,
  children,
}: {
  close: () => void;
  children: ReactNode;
}) {
  const dialog = useRef<HTMLDialogElement>(null),
    closeAction = useRef(close);
  closeAction.current = close;
  useEffect(() => {
    const element = dialog.current;
    const previous = document.activeElement;
    element?.showModal();
    return () => {
      element?.close();
      if (previous instanceof HTMLElement && previous.isConnected)
        previous.focus({ preventScroll: true });
    };
  }, []);
  return (
    <dialog
      ref={dialog}
      className="detail"
      aria-label="Order details"
      aria-describedby="detail-title"
      onCancel={(event) => {
        event.preventDefault();
        closeAction.current();
      }}
      onClick={(event) => {
        if (event.target !== event.currentTarget) return;
        const box = event.currentTarget.getBoundingClientRect();
        if (
          event.clientY < box.top ||
          event.clientY > box.bottom ||
          event.clientX < box.left ||
          event.clientX > box.right
        )
          closeAction.current();
      }}
    >
      {children}
    </dialog>
  );
}

export function CurrencyPicker({
  currencies,
  value,
  change,
}: {
  currencies: string[];
  value: string;
  change: (currency: string) => void;
}) {
  return (
    <SelectPicker
      label="Currency"
      className="currency-picker"
      options={currencies.map((value) => ({
        value,
        label: value === "XTR" ? "Stars (XTR)" : value,
      }))}
      value={value}
      change={change}
      render={(currency) => <CurrencySymbol currency={currency} />}
    />
  );
}
export function SortPicker({
  value,
  change,
}: {
  value: "time" | "number" | "price";
  change: (value: "time" | "number" | "price") => void;
}) {
  return (
    <SelectPicker
      label="Sort by"
      className="sort-picker"
      options={[
        { value: "time", label: "Time" },
        { value: "number", label: "Gift #" },
        { value: "price", label: "Price" },
      ]}
      value={value}
      change={change}
    />
  );
}
export function ScrollLoader({
  loading,
  hasMore,
  disabled,
  loadMore,
}: {
  loading: boolean;
  hasMore: boolean;
  disabled: boolean;
  loadMore: () => void;
}) {
  const sentinel = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (loading || !hasMore || disabled || !sentinel.current) return;
    const observer = new IntersectionObserver(
      ([entry]) => {
        if (entry.isIntersecting) loadMore();
      },
      { rootMargin: "300px" },
    );
    observer.observe(sentinel.current);
    return () => observer.disconnect();
  }, [loading, hasMore, disabled, loadMore]);
  if (!loading && !hasMore) return null;
  return (
    <div ref={sentinel} className="scroll-loader" role="status">
      {loading
        ? "Loading more gifts…"
        : disabled
          ? "Loading paused — retry above"
          : "Scroll for more"}
    </div>
  );
}
