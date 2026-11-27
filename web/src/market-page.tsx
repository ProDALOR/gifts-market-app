import {
  aliasFilters,
  availableFilterSelectors,
  preferredSelector,
  resolveFilterSelectors,
} from "./catalog/aliases";
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
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
import { SortPicker, marketSortOptions } from "./ui/pickers";
import { ScrollLoader } from "./orderbook/scroll-loader";
import { ShareStatus, useSharing } from "./use-sharing";
import { AppHeader } from "./app/header";
import { LoadingScreen } from "./app/loading-screen";
import { BackToTop } from "./ui/back-to-top";
import type { TelegramContext } from "./telegram";
import {
  AppNavigation,
  useNavigation,
  useNavigationBack,
  usePageSlot,
  useHistorySlot,
  useHistoryState,
  useSharedCurrency,
} from "./app/navigation";
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
  const navigation = useNavigation();
  const [currency, setCurrency] = useSharedCurrency();
  const retainedFeed = usePageSlot("market.feed", () => ({
    signature: "",
    state: undefined as import("./order-feed").FeedState | undefined,
    bookmark: undefined as import("./order-feed").Bookmark | undefined,
  }));
  const entryFeed = useHistorySlot("market.feed", () => ({
    ...retainedFeed.current,
  }));
  useLayoutEffect(() => {
    retainedFeed.current = entryFeed.current;
  }, [retainedFeed, entryFeed]);
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
  const [selected, setSelected] = useHistoryState<Order | undefined>(
    "market.preview",
    () => undefined,
  );
  const [storedChoice, setChoice] = useState(() => saved()),
    [reloadKey, setReloadKey] = useState(0),
    [openTrait, setOpenTrait] = useState<TraitKey>();
  const choice = { ...storedChoice, currency };
  const sharingScope = useMemo(
    () => JSON.stringify([storedChoice, currency, selected?.id, giftSlug]),
    [storedChoice, currency, selected?.id, giftSlug],
  );
  const sharing = useSharing(context, sharingScope);
  const shareValuation = sharing.available
    ? (slug: string, currency_id: string) => {
        void sharing.share({ type: "gift", slug, currency_id });
      }
    : undefined;

  const {
    types,
    directory,
    options,
    names,
    item: catalogItem,
    catalogLoading,
    startupError,
    optionsLoading,
    optionsError,
    retryOptions,
    retryCatalog,
  } = useCatalog(choice.collection);
  const item = catalogItem?.id ? catalogItem : undefined;
  const catalogReady =
    !!item && (options.size > 0 || (!optionsLoading && !optionsError));
  const availableFilters = useMemo(
    () =>
      catalogReady
        ? availableFilterSelectors(choice.filters, options)
        : choice.filters,
    [catalogReady, choice.filters, options],
  );
  // Keep using known traits during a catalog refresh, but do not drop an
  // unknown selector until the new catalog has successfully finished loading.
  const filtersReady =
    catalogReady &&
    (availableFilters === choice.filters || (!optionsLoading && !optionsError));
  const marketFilters = filtersReady ? availableFilters : choice.filters;
  useEffect(() => {
    if (marketFilters !== choice.filters)
      setChoice((previous) =>
        previous.filters === choice.filters
          ? { ...previous, filters: marketFilters }
          : previous,
      );
  }, [marketFilters, choice.filters]);
  const availableTypes = useMemo(
    () => types.filter((t) => directory.some((b) => b.collection_id === t.id)),
    [types, directory],
  );
  const filterAliases = useMemo(
    () => aliasFilters(marketFilters, options),
    [marketFilters, options],
  );
  const shareTarget = useMemo(
    () => ({
      type: "market" as const,
      selection: choiceSelection({
        ...storedChoice,
        currency,
        filters: filterAliases,
        collection: preferredSelector(item) ?? storedChoice.collection,
      }),
    }),
    [storedChoice, currency, filterAliases, item],
  );
  const pickerFilters = useMemo(
    () => resolveFilterSelectors(marketFilters, options),
    [marketFilters, options],
  );
  useEffect(() => {
    if (!directory[0]) return;
    // Market reads stay scoped, including startup and old cross-collection links.
    const collection =
      item?.id ||
      choice.collection ||
      availableTypes.find((type) =>
        directory.some(
          (book) =>
            book.collection_id === type.id && book.currency_id === currency,
        ),
      )?.id ||
      availableTypes[0]?.id;
    if (!collection) return;
    const available = directory.filter(
      (book) => book.collection_id === collection,
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
  }, [directory, availableTypes, choice.collection, currency, item]);
  useEffect(() => {
    const p = selectionParams(
      choiceSelection(
        {
          ...choice,
          collection: preferredSelector(item) ?? choice.collection,
          filters: filterAliases,
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
  }, [JSON.stringify(choice), item, filterAliases, giftSlug]);
  const choiceKey = JSON.stringify({
    ...choice,
    collection: item?.id || choice.collection,
    filters: pickerFilters,
  });
  const previousChoice = useRef<string | undefined>(undefined);
  useEffect(() => {
    if (!filtersReady) return;
    if (
      previousChoice.current !== undefined &&
      previousChoice.current !== choiceKey
    )
      setSelected(undefined);
    previousChoice.current = choiceKey;
  }, [choiceKey, filtersReady]);
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
  const query: FeedQuery | null =
    !choice.collection ||
    !choice.currency ||
    !item ||
    (!filtersReady && Object.values(choice.filters).some((ids) => ids?.length))
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
  } = useOrderFeed(query, false, reloadKey, entryFeed.current);
  const compactStatus =
    status === "Live" || status === "Connecting" || status === "Reconnecting";
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
  useNavigationBack(!!openTrait, () => setOpenTrait(undefined));
  const set = (next: Choice) => {
    closeSlug();
    setCurrency(next.currency);
    setChoice(next);
    setOpenTrait(undefined);
  };
  const selectedType = useCallback(
    (id: string) => {
      if (!id) return;
      const available = directory.filter((b) => b.collection_id === id);
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

  if (catalogLoading && !types.length && !startupError)
    return <LoadingScreen />;
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
            <AppHeader
              context={context}
              currencies={currencies}
              currency={choice.currency}
              share={
                choice.collection
                  ? {
                      controller: sharing,
                      target: shareTarget,
                      label: "Share market selection",
                    }
                  : undefined
              }
              changeCurrency={(currency) => {
                if (currency !== choice.currency)
                  set({ ...choice, currency, venue: "", min: "", max: "" });
              }}
            />
            <AppNavigation page="market" />
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
                    title={
                      choice.mode === mode
                        ? `${status} · ${orders.length.toLocaleString()} loaded`
                        : undefined
                    }
                    aria-description={
                      choice.mode === mode
                        ? `${status} · ${orders.length.toLocaleString()} loaded`
                        : undefined
                    }
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
                    {choice.mode === mode && (
                      <span
                        className={`status market-view-status${status === "Live" ? " live" : ""}`}
                        aria-hidden="true"
                      >
                        <i />
                      </span>
                    )}
                  </button>
                ))}
              </div>
              {compactStatus && (
                <span className="sr mobile-feed-status" role="status">
                  {status}
                </span>
              )}
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
            <ShareStatus sharing={sharing} />
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
              <div className="feed-status" data-compact={compactStatus}>
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
                visible={setVisibleRange}
                card={(order) => ({
                  options,
                  collectionPreview:
                    types.find((c) => c.id === order.book.collection_id)
                      ?.preview ?? undefined,
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
                  open: (order) => {
                    closeSlug();
                    setSelected(order);
                  },
                })}
              />
              {!displayOrders.length &&
                status !== "Loading" &&
                status !== "Connecting" && (
                  <div className="empty" role="status">
                    <span>◇</span>
                    <strong>
                      {choice.mode === "executed"
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
          </div>
        </main>
        <BackToTop target={controls} blocked={!!selected || !!giftSlug} />
        {giftSlug && choice.collection && choice.currency && (
          <SlugDetails
            slug={giftSlug}
            collection={choice.collection}
            currency={choice.currency}
            options={options}
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
            options={options}
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
