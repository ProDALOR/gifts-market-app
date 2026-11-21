import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import type {
  Order,
  TraitFilters,
  TraitKey,
  Sort,
  FeedQuery,
  BidMatch,
} from "./market";
import { useCatalog } from "./catalog/use-catalog";
import { useOrderFeed } from "./use-order-feed";
import { useBackgroundPlaybackSuspension } from "./previews/use-playback";
import { CollectionRail } from "./collections/collection-picker";
import { TraitPickers } from "./collections/trait-pickers";
import { TraitPreview } from "./previews/trait-preview";
import { VenueIcon } from "./ui/icons";
import { OrderDetails } from "./orderbook/order-details";
import { FloorsProvider, FloorsStatus } from "./valuation/floors";
import { orderKey } from "./orderbook/order-activity";
import { GiftGrid } from "./orderbook/gift-grid";
import { CurrencyPicker, SortPicker } from "./ui/pickers";
import { ScrollLoader } from "./orderbook/scroll-loader";
import { BrandMark } from "./brand/brand-mark";
import { BackToTop } from "./ui/back-to-top";
import { initializeTelegram, launchSelection } from "./telegram";
import type { TelegramContext } from "./telegram";
import { useTelegram, useTelegramBack, TelegramAccount } from "./telegram-ui";
import "./style.css";
type Choice = {
  collection: string;
  currency: string;
  venue: string;
  mode: "ask" | "bid" | "executed";
  bid_match: BidMatch;
  sort: Exclude<Sort, "identity">;
  min: string;
  max: string;
  from: string;
  until: string;
  filters: TraitFilters;
};
const defaultSort = {
  ask: "price_asc",
  bid: "price_desc",
  executed: "time_desc",
} as const;
function saved(context: TelegramContext): Choice {
  const p = launchSelection(context, location.search);
  const mode =
    p.get("mode") === "bid"
      ? "bid"
      : p.get("mode") === "executed"
        ? "executed"
        : "ask";
  let filters: TraitFilters = {};
  try {
    const parsed = JSON.parse(p.get("filters") || "{}");
    if (parsed && typeof parsed === "object" && !Array.isArray(parsed))
      for (const key of ["models", "backdrops", "patterns"] as const) {
        if (
          Array.isArray(parsed[key]) &&
          parsed[key].every(
            (id: unknown) =>
              typeof id === "string" && id.length > 0 && id.length <= 256,
          )
        )
          filters[key] = [...new Set<string>(parsed[key])];
      }
  } catch {}
  return {
    collection: p.get("collection") || "",
    currency: p.get("currency") || "",
    venue: p.get("venue") || "",
    mode,
    bid_match: p.get("bid_match") === "any" ? "any" : "all",
    sort:
      (
        [
          "price_asc",
          "price_desc",
          "time_asc",
          "time_desc",
          "number_asc",
          "number_desc",
        ] as const
      ).find((sort) => sort === p.get("sort")) || defaultSort[mode],
    filters,
    min: p.get("min") || "",
    max: p.get("max") || "",
    from: p.get("from") || "",
    until: p.get("until") || "",
  };
}
function App({ context }: { context: TelegramContext }) {
  const controls = useRef<HTMLElement>(null);
  const telegram = useTelegram(context);
  const [accountOpen, setAccountOpen] = useState(false);
  const [choice, setChoice] = useState(() => saved(context)),
    [reloadKey, setReloadKey] = useState(0),
    [selected, setSelected] = useState<Order>(),
    [openTrait, setOpenTrait] = useState<TraitKey>();
  const {
    types,
    directory,
    options,
    names,
    item,
    startupError,
    optionsLoading,
    optionsError,
    retryOptions,
    retryCatalog,
  } = useCatalog(reloadKey, choice.collection);
  useBackgroundPlaybackSuspension(accountOpen);
  useEffect(() => {
    if (!directory[0]) return;
    setChoice((old) => {
      const selectedItem = old.collection || directory[0].collection_id;
      const available = directory.filter(
        (book) => book.collection_id === selectedItem,
      );
      const currency = available.some(
        (book) => book.currency_id === old.currency,
      )
        ? old.currency
        : available.find((book) => book.currency_id === "GRAM")?.currency_id ||
          available[0]?.currency_id ||
          old.currency;
      return {
        ...old,
        collection: selectedItem,
        currency,
        venue: available.some(
          (book) =>
            book.currency_id === currency && book.venue_id === old.venue,
        )
          ? old.venue
          : "",
      };
    });
  }, [directory]);
  useEffect(() => {
    const p = new URLSearchParams();
    for (const [key, value] of Object.entries(choice)) {
      if (key === "filters") {
        if (Object.keys(value as object).length)
          p.set(key, JSON.stringify(value));
      } else if (key === "bid_match") {
        if (choice.mode === "bid") p.set(key, String(value));
      } else if (value) p.set(key, String(value));
    }
    history.replaceState(null, "", `${location.pathname}?${p}`);
    setSelected(undefined);
  }, [JSON.stringify(choice)]);
  useEffect(() => {
    const pop = () => setChoice(saved(context));
    window.addEventListener("popstate", pop);
    return () => window.removeEventListener("popstate", pop);
  }, []);
  const base = {
    filters: {
      collection_id: choice.collection,
      currency_id: choice.currency,
      venue_id: choice.venue || null,
      models: choice.filters.models || [],
      backdrops: choice.filters.backdrops || [],
      patterns: choice.filters.patterns || [],
      number: null,
      price:
        choice.min || choice.max
          ? { min: choice.min || null, max: choice.max || null }
          : null,
    },
    sort: choice.sort,
  };
  const query: FeedQuery | null =
    !choice.collection || !choice.currency
      ? null
      : {
          ...base,
          bid_match: choice.mode === "bid" ? choice.bid_match : undefined,
          source:
            choice.mode === "executed"
              ? "history"
              : choice.mode === "bid"
                ? "bids"
                : "asks",
          period:
            choice.mode === "executed" && (choice.from || choice.until)
              ? { from: choice.from || null, until: choice.until || null }
              : undefined,
        };

  const {
    orders,
    displayOrders,
    activity,
    status,
    error,
    loadingMore,
    hasMore,
    loadMore,
    setVisibleRange,
  } = useOrderFeed(query, false, reloadKey);
  useEffect(
    () =>
      setSelected((current) =>
        current
          ? displayOrders.find(
              (order) =>
                order.id === current.id &&
                order.book.venue_id === current.book.venue_id,
            ) || current
          : undefined,
      ),
    [displayOrders],
  );
  useTelegramBack(context, accountOpen || !!selected || !!openTrait, () => {
    if (accountOpen) setAccountOpen(false);
    else if (selected) setSelected(undefined);
    else if (openTrait) setOpenTrait(undefined);
  });
  const set = (next: Choice) => {
    setChoice(next);
    setOpenTrait(undefined);
  };
  const selectedType = useCallback(
    (id: string) => {
      const available = directory.filter((b) => b.collection_id === id);
      set({
        ...choice,
        collection: id,
        currency: available.some((b) => b.currency_id === choice.currency)
          ? choice.currency
          : available[0]?.currency_id || choice.currency,
        venue: "",
        filters: {},
        min: "",
        max: "",
        from: "",
        until: "",
      });
    },
    [directory, choice],
  );
  const availableTypes = useMemo(
    () => types.filter((t) => directory.some((b) => b.collection_id === t.id)),
    [types, directory],
  );
  const floorCollections = useMemo(
    () => availableTypes.map((row) => row.id),
    [availableTypes],
  );
  const currencies = [
    ...new Set(
      directory
        .filter((b) => b.collection_id === choice.collection)
        .map((b) => b.currency_id),
    ),
  ].sort();
  const venueIds = [
    ...new Set(
      directory
        .filter(
          (b) =>
            b.collection_id === choice.collection &&
            b.currency_id === choice.currency,
        )
        .map((b) => b.venue_id),
    ),
  ].sort();
  const changeTrait = useCallback((id: TraitKey, value?: string[]) => {
    setChoice((previous) => {
      const filters = { ...previous.filters };
      if (value?.length) filters[id] = value;
      else delete filters[id];
      return { ...previous, filters };
    });
  }, []);
  const resetTraits = useCallback(
    () => setChoice((previous) => ({ ...previous, filters: {} })),
    [],
  );

  const selectedCollection = selected
    ? types.find((collection) => collection.id === selected.book.collection_id)
    : undefined;

  return (
    <FloorsProvider
      currency={choice.currency}
      collection={choice.collection}
      collections={floorCollections}
      order={selected}
    >
      <div className="workspace">
        <main className="orderbook market-shell">
          <aside
            ref={controls}
            id="market-filters"
            tabIndex={-1}
            className="market-sidebar"
            aria-label="Market controls"
            data-scroll-container
          >
            <div className="sidebar-heading">
              <BrandMark animated />
              <CurrencyPicker
                currencies={currencies}
                value={choice.currency}
                change={(currency) => {
                  if (currency !== choice.currency)
                    set({ ...choice, currency, venue: "", min: "", max: "" });
                }}
              />
            </div>
            <header className="account-bar">
              <TelegramAccount
                telegram={telegram}
                context={context}
                open={accountOpen}
                setOpen={setAccountOpen}
                selection={new URLSearchParams(location.search)}
              />
            </header>
            <CollectionRail
              collections={availableTypes}
              selected={choice.collection}
              select={selectedType}
            />
            <FloorsStatus />
            {(startupError || optionsError || error) && (
              <div role="alert" className="notice">
                {startupError || optionsError || error}
                <button
                  onClick={() => {
                    if (optionsError) retryOptions();
                    else if (startupError) retryCatalog();
                    else setReloadKey((v) => v + 1);
                  }}
                >
                  Reload
                </button>
              </div>
            )}
            <TraitPickers
              key={choice.collection}
              collection={item}
              options={options}
              loading={optionsLoading}
              loadError={optionsError}
              filters={choice.filters}
              change={changeTrait}
              open={openTrait}
              setOpen={setOpenTrait}
            />
            <TraitPreview
              collection={item}
              options={options}
              filters={choice.filters}
              reset={resetTraits}
            />
            <div className="market-controls">
              <div
                className="segments market-views"
                role="group"
                aria-label="Market view"
              >
                {(["ask", "bid", "executed"] as const).map((mode) => (
                  <button
                    key={mode}
                    aria-pressed={choice.mode === mode}
                    aria-controls={
                      mode === "bid" && choice.mode === "bid"
                        ? "bid-matching"
                        : undefined
                    }
                    className={choice.mode === mode ? "active" : ""}
                    onClick={() => {
                      if (mode !== choice.mode)
                        set({
                          ...choice,
                          mode,
                          sort: defaultSort[mode],
                        });
                    }}
                  >
                    {mode === "ask"
                      ? "Asks"
                      : mode === "bid"
                        ? "Bids"
                        : "History"}
                  </button>
                ))}
              </div>
              {choice.mode === "bid" && (
                <div className="bid-matching" id="bid-matching">
                  <div
                    className="segments"
                    role="group"
                    aria-label="Bid matching"
                  >
                    {(["all", "any"] as const).map((match) => (
                      <button
                        key={match}
                        aria-pressed={choice.bid_match === match}
                        className={choice.bid_match === match ? "active" : ""}
                        onClick={() => set({ ...choice, bid_match: match })}
                        title={
                          match === "any"
                            ? "Accepts any selected value in each trait; no selection shows all bids"
                            : "Exact bid conditions; unselected traits must be unrestricted"
                        }
                      >
                        {match === "any" ? "Compatible" : "Exact"}
                      </button>
                    ))}
                  </div>
                </div>
              )}
            </div>
            {(choice.min || choice.max || choice.from || choice.until) && (
              <div className="selection-note">
                <span>
                  {choice.min || choice.max
                    ? `Price ${choice.min || "0"}–${choice.max || "∞"} ${choice.currency}`
                    : "Selected history period"}
                </span>
                <button
                  aria-label="Clear linked filters"
                  onClick={() =>
                    set({ ...choice, min: "", max: "", from: "", until: "" })
                  }
                >
                  Clear ×
                </button>
              </div>
            )}
            <div className="market-toolbar">
              <div className="venues" role="group" aria-label="Markets">
                {["", ...venueIds].map((id) => {
                  const name = id ? names.get(id) || id : "All venues";
                  return (
                    <button
                      className="venue"
                      data-venue={name}
                      key={id}
                      title={name}
                      aria-label={name}
                      aria-pressed={choice.venue === id}
                      onClick={() => set({ ...choice, venue: id })}
                    >
                      <VenueIcon name={name} />
                    </button>
                  );
                })}
              </div>
              <div
                className="sort-control"
                role="group"
                aria-label="Sort listings"
              >
                <SortPicker
                  value={
                    choice.sort.split("_")[0] as "time" | "number" | "price"
                  }
                  change={(key) =>
                    set({
                      ...choice,
                      sort: `${key}_${choice.sort.endsWith("_asc") ? "asc" : "desc"}` as Choice["sort"],
                    })
                  }
                />
                <button
                  aria-label="Reverse sort direction"
                  title={
                    choice.sort.endsWith("_asc")
                      ? "Ascending; switch to descending"
                      : "Descending; switch to ascending"
                  }
                  onClick={() =>
                    set({
                      ...choice,
                      sort: `${choice.sort.split("_")[0]}_${choice.sort.endsWith("_asc") ? "desc" : "asc"}` as Choice["sort"],
                    })
                  }
                >
                  {choice.sort.endsWith("_asc") ? "↑" : "↓"}
                </button>
              </div>
            </div>
          </aside>
          <div className="market-results">
            <header className="market-heading">
              <div>
                <p className="market-eyebrow">
                  {choice.mode === "ask"
                    ? "Asks"
                    : choice.mode === "bid"
                      ? "Bids"
                      : "History"}
                </p>
                <h1>{item?.name || "Select a collection"}</h1>
              </div>
            </header>
            <section className="orders" aria-label="Order book">
              <div className="feed-status">
                <span
                  role="status"
                  className={`status${status === "Live" ? " live" : ""}`}
                >
                  <i />
                  {status}
                </span>
                <span>{orders.length} loaded</span>
              </div>

              <GiftGrid
                orders={displayOrders}
                busy={status === "Loading" || status === "Connecting"}
                visible={setVisibleRange}
                card={(order) => ({
                  options,
                  activity: activity.get(orderKey(order)),
                  collectionName:
                    item?.name ||
                    names.get(order.book.collection_id) ||
                    order.book.collection_id,
                  venueName:
                    names.get(order.book.venue_id) || order.book.venue_id,
                  selected:
                    selected?.id === order.id &&
                    selected.book.venue_id === order.book.venue_id,
                  open: setSelected,
                })}
              />
              {!displayOrders.length && (
                <div className="empty" role="status">
                  <span>◇</span>
                  <strong>
                    {status === "Loading" || status === "Connecting"
                      ? "Loading this market…"
                      : choice.mode === "executed"
                        ? "No confirmed executions"
                        : "No matching orders"}
                  </strong>
                  <p>
                    {Object.keys(choice.filters).length
                      ? "Adjust the traits to see more results."
                      : "Orders will appear here as venues publish them."}
                  </p>
                </div>
              )}
              <ScrollLoader
                loading={loadingMore}
                hasMore={hasMore}
                disabled={!!error}
                loadMore={loadMore}
              />
            </section>
            <footer className="app-footer">
              <BrandMark />
              <span>Every venue. One view.</span>
              <a
                className="api-link"
                href="/docs"
                target="_blank"
                rel="noreferrer"
              >
                API ↗
              </a>
            </footer>
          </div>
        </main>
        <BackToTop target={controls} blocked={!!selected || accountOpen} />
        {selected && (
          <OrderDetails
            collectionPreview={selectedCollection?.preview ?? undefined}
            selected={selected}
            options={options}
            names={names}
            collectionName={item?.name || ""}
            collectionSlug={selectedCollection?.slug}
            removed={activity.get(orderKey(selected))?.state === "removed"}
            close={() => setSelected(undefined)}
          />
        )}
      </div>
    </FloorsProvider>
  );
}

void initializeTelegram().then((context) =>
  createRoot(document.getElementById("root")!).render(
    <App context={context} />,
  ),
);
