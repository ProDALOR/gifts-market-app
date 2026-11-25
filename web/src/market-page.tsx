import {
  allCollections,
  aliasFilters,
  preferredSelector,
  resolveFilterSelectors,
} from "./catalog/aliases";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
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
import { SlugDetails } from "./orderbook/slug-details";
import { normalizeSlug } from "./valuation/slug";
import type { ValuationSubject } from "./order-target";
import { OrderDetails } from "./orderbook/order-details";
import { FloorsProvider, FloorsStatus } from "./valuation/floors";
import { orderKey } from "./orderbook/order-activity";
import { GiftGrid } from "./orderbook/gift-grid";
import { CurrencyPicker, SortPicker, marketSortOptions } from "./ui/pickers";
import { ScrollLoader } from "./orderbook/scroll-loader";
import { BrandMark } from "./brand/brand-mark";
import { BackToTop } from "./ui/back-to-top";
import type { TelegramContext } from "./telegram";
import { TelegramAccount } from "./telegram-ui";
import {
  AppNavigation,
  useNavigation,
  useNavigationBack,
  usePageSlot,
  useSharedCurrency,
} from "./app/navigation";
import { useSession } from "./app/session";
import {
  choiceSelection,
  selectionChoice,
  selectionFromParams,
  selectionParams,
  selectionAt,
  selectionHref,
  type MarketChoice as Choice,
  defaultSort,
} from "./market-selection";
function saved(): Choice {
  return selectionChoice(selectionFromParams(selectionAt(location)));
}
export function MarketPage({ context }: { context: TelegramContext }) {
  const controls = useRef<HTMLElement>(null);
  const telegram = useSession();
  const navigation = useNavigation();
  const [currency, setCurrency] = useSharedCurrency();
  const retainedFeed = usePageSlot("market.feed", () => ({
    signature: "",
    state: undefined as import("./order-feed").FeedState | undefined,
    bookmark: undefined as import("./order-feed").Bookmark | undefined,
  }));
  const shareValuation = context.app?.switchInlineQuery
    ? (slug: string, currency: string) =>
        context.app!.switchInlineQuery!(`${slug} ${currency}`, [
          "users",
          "groups",
          "channels",
        ])
    : undefined;
  const [accountOpen, setAccountOpen] = useState(false);
  const [giftSlug, setGiftSlug] = useState(() => {
    const value = selectionAt(location).get("gift");
    return normalizeSlug(value) ? value! : undefined;
  });
  const [slugSubject, setSlugSubject] = useState<ValuationSubject>();
  const closeSlug = () => {
    setGiftSlug(undefined);
    setSlugSubject(undefined);
    const url = new URL(location.href);
    const params = selectionAt(url);
    params.delete("gift");
    history.replaceState(
      history.state,
      "",
      selectionHref(url.pathname, params),
    );
    navigation?.rememberLocation();
  };
  const [storedChoice, setChoice] = useState(() => saved()),
    [reloadKey, setReloadKey] = useState(0),
    [selected, setSelected] = useState<Order>(),
    [openTrait, setOpenTrait] = useState<TraitKey>();
  const [previewIds, setPreviewIds] = useState<string[]>([]);
  const choice = { ...storedChoice, currency };
  const {
    types,
    directory,
    options,
    previewOptions,
    names,
    item,
    catalogLoading,
    startupError,
    optionsLoading,
    optionsError,
    retryOptions,
    retryCatalog,
  } = useCatalog(
    reloadKey,
    choice.collection,
    choice.collection ? [] : previewIds,
  );
  const filterAliases = useMemo(
    () => aliasFilters(choice.filters, options),
    [choice.filters, options],
  );
  const pickerFilters = useMemo(
    () => resolveFilterSelectors(choice.filters, options),
    [choice.filters, options],
  );
  useBackgroundPlaybackSuspension(accountOpen);
  useEffect(() => {
    if (!directory[0]) return;
    const collection = item?.id ?? choice.collection;
    const available = directory.filter(
      (book) => !collection || book.collection_id === collection,
    );
    const selectedCurrency = available.some(
      (book) => book.currency_id === currency,
    )
      ? currency
      : available.find((book) => book.currency_id === "GRAM")?.currency_id ||
        available[0]?.currency_id ||
        currency;
    if (selectedCurrency !== currency) setCurrency(selectedCurrency);
    setChoice((old) => {
      const venue = available.some(
        (book) =>
          book.currency_id === selectedCurrency && book.venue_id === old.venue,
      )
        ? old.venue
        : "";
      return old.collection === collection && old.venue === venue
        ? old
        : { ...old, collection, venue };
    });
  }, [directory, choice.collection, currency, item]);
  useEffect(() => {
    const p = selectionParams(
      choiceSelection(
        {
          ...choice,
          collection: preferredSelector(item) ?? choice.collection,
          filters: aliasFilters(choice.filters, options),
        },
        giftSlug,
      ),
    );
    history.replaceState(
      history.state,
      "",
      selectionHref(location.pathname, p),
    );
    navigation?.rememberLocation();
  }, [JSON.stringify(choice), item, options, giftSlug]);
  useEffect(() => setSelected(undefined), [JSON.stringify(choice)]);
  const base = {
    filters: {
      collection_id: choice.collection
        ? (preferredSelector(item) ?? choice.collection)
        : undefined,
      currency_id: choice.currency,
      venue_id: choice.venue || null,
      models: filterAliases.models || [],
      backdrops: filterAliases.backdrops || [],
      patterns: filterAliases.patterns || [],
      number: choice.number ?? null,
      price:
        choice.min || choice.max
          ? { min: choice.min || null, max: choice.max || null }
          : null,
    },
    sort: choice.sort,
  };
  const query: FeedQuery | null = !choice.currency
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
  } = useOrderFeed(query, false, reloadKey, retainedFeed.current);
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
  useNavigationBack(accountOpen || !!openTrait, () => {
    if (accountOpen) setAccountOpen(false);
    else if (openTrait) setOpenTrait(undefined);
  });
  const set = (next: Choice) => {
    closeSlug();
    setCurrency(next.currency);
    setChoice(next);
    setOpenTrait(undefined);
  };
  const selectedType = useCallback(
    (id: string) => {
      const available = directory.filter((b) => !id || b.collection_id === id);
      set({
        ...choice,
        collection: id,
        currency: available.some((b) => b.currency_id === choice.currency)
          ? choice.currency
          : available[0]?.currency_id || choice.currency,
        venue: "",
        filters: {
          backdrops: aliasFilters(choice.filters, options).backdrops,
          patterns: aliasFilters(choice.filters, options).patterns,
        },
        min: "",
        max: "",
        from: "",
        until: "",
      });
    },
    [directory, choice, options],
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
        .filter(
          (b) => !choice.collection || b.collection_id === choice.collection,
        )
        .map((b) => b.currency_id),
    ),
  ].sort();
  const venueIds = [
    ...new Set(
      directory
        .filter(
          (b) =>
            (!choice.collection || b.collection_id === choice.collection) &&
            b.currency_id === choice.currency,
        )
        .map((b) => b.venue_id),
    ),
  ].sort();
  const changeTrait = useCallback(
    (id: TraitKey, value?: string[]) => {
      setChoice((previous) => {
        const filters = { ...previous.filters };
        if (value?.length)
          filters[id] = aliasFilters({ [id]: value }, options)[id];
        else delete filters[id];
        return { ...previous, filters };
      });
    },
    [options],
  );
  const resetTraits = useCallback(
    () => setChoice((previous) => ({ ...previous, filters: {} })),
    [],
  );

  const selectedCollection = types.find(
    (collection) =>
      collection.id ===
      (selected?.book.collection_id ??
        (giftSlug ? choice.collection : undefined)),
  );

  return (
    <FloorsProvider
      currency={choice.currency}
      collection={choice.collection}
      collections={floorCollections}
      order={selected ?? slugSubject}
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
            <AppNavigation page="market" />
            <header className="account-bar">
              <TelegramAccount
                telegram={telegram}
                context={context}
                open={accountOpen}
                setOpen={setAccountOpen}
                selection={selectionAt(location)}
              />
            </header>
            <CollectionRail
              collections={[allCollections, ...availableTypes]}
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
              filters={pickerFilters}
              change={changeTrait}
              open={openTrait}
              setOpen={setOpenTrait}
            />
            <TraitPreview
              collection={item}
              options={options}
              filters={pickerFilters}
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
            {(choice.min ||
              choice.max ||
              choice.from ||
              choice.until ||
              choice.number) && (
              <div className="selection-note">
                <span>
                  {choice.min || choice.max
                    ? `Price ${choice.min || "0"}–${choice.max || "∞"} ${choice.currency}`
                    : choice.number
                      ? "Selected gift numbers"
                      : "Selected history period"}
                </span>
                <button
                  aria-label="Clear linked filters"
                  onClick={() =>
                    set({
                      ...choice,
                      min: "",
                      max: "",
                      from: "",
                      until: "",
                      number: null,
                    })
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
                  options={marketSortOptions}
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
              <h1
                aria-busy={catalogLoading && !item}
                aria-label={
                  catalogLoading && !item ? "Loading collection" : undefined
                }
              >
                {item?.name ||
                  (catalogLoading ? (
                    <span
                      className="collection-title-skeleton"
                      aria-hidden="true"
                    />
                  ) : choice.collection ? (
                    "Collection unavailable"
                  ) : (
                    "Select a collection"
                  ))}
              </h1>
              <div className="feed-status">
                <span
                  role="status"
                  className={`status${status === "Live" ? " live" : ""}`}
                >
                  <i />
                  {status}
                </span>
                <span aria-hidden="true">·</span>
                <span>{orders.length.toLocaleString()} loaded</span>
              </div>
            </header>
            <section className="orders" aria-label="Order book">
              <GiftGrid
                orders={displayOrders}
                busy={status === "Loading" || status === "Connecting"}
                visible={(start, end, anchor) => {
                  setVisibleRange(start, end, anchor);
                  if (!choice.collection) {
                    const ids = [
                      ...new Set(
                        displayOrders
                          .slice(start, end)
                          .map((o) => o.book.collection_id),
                      ),
                    ].slice(0, 32);
                    setPreviewIds((old) =>
                      JSON.stringify(old) === JSON.stringify(ids) ? old : ids,
                    );
                  }
                }}
                card={(order) => ({
                  options: choice.collection ? options : previewOptions,
                  collectionPreview:
                    types.find((c) => c.id === order.book.collection_id)
                      ?.preview ?? undefined,
                  activity: activity.get(orderKey(order)),
                  collectionName:
                    (choice.collection ? item?.name : undefined) ||
                    names.get(order.book.collection_id) ||
                    order.book.collection_id,
                  venueName:
                    names.get(order.book.venue_id) || order.book.venue_id,
                  selected:
                    selected?.id === order.id &&
                    selected.book.venue_id === order.book.venue_id,
                  open: (order) => {
                    closeSlug();
                    setSelected(order);
                  },
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
        <BackToTop
          target={controls}
          blocked={!!selected || !!giftSlug || accountOpen}
        />
        {giftSlug && choice.collection && choice.currency && (
          <SlugDetails
            slug={giftSlug}
            collection={choice.collection}
            currency={choice.currency}
            options={choice.collection ? options : previewOptions}
            names={names}
            preview={selectedCollection?.preview ?? undefined}
            onGift={setSlugSubject}
            close={closeSlug}
            share={shareValuation}
          />
        )}
        {selected && (
          <OrderDetails
            collectionPreview={selectedCollection?.preview ?? undefined}
            selected={selected}
            share={shareValuation}
            options={choice.collection ? options : previewOptions}
            names={names}
            collectionName={selectedCollection?.name || ""}
            collectionSlug={selectedCollection?.slug}
            removed={activity.get(orderKey(selected))?.state === "removed"}
            close={() => setSelected(undefined)}
          />
        )}
      </div>
    </FloorsProvider>
  );
}
