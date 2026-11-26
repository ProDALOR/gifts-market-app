import { portfolioFiltersFromParams, portfolioParams } from "./links";
import { selectionAt, selectionHref } from "../market-selection";
import { aliasFilters, resolveFilterSelectors } from "../catalog/aliases";
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import type { TelegramContext } from "../telegram";
import { SessionBoundary, useSession } from "../app/session";
import {
  AppNavigation,
  useNavigation,
  useNavigationBack,
  usePageState,
  useSharedCurrency,
} from "../app/navigation";
import { portfolioUser } from "../app/routes";
import { useCatalog } from "../catalog/use-catalog";
import { CollectionRail } from "../collections/collection-picker";
import { TraitPickers } from "../collections/trait-pickers";
import { TraitPreview } from "../previews/trait-preview";
import { GiftPlaceholder } from "../previews/gift-placeholder";
import { FloorsProvider } from "../valuation/floors";
import { SortPicker } from "../ui/pickers";
import { Quote } from "../ui/market-controls";
import { RelativeAge } from "../ui/relative-age";
import { ShareButton, ShareStatus, useSharing } from "../use-sharing";
import { AppHeader } from "../app/header";
import { useBackgroundPlaybackSuspension } from "../previews/use-playback";
import { VirtualGiftGrid } from "../orderbook/virtual-gift-grid";
import { BackToTop } from "../ui/back-to-top";
import type { TraitKey } from "../market";
import {
  PortfolioDetails,
  PortfolioGiftCard,
  portfolioSubject,
} from "./previews";
import { useMarketStatus } from "../use-market-status";
import { usePortfolio } from "./use-portfolio";
import { usePortfolioRows } from "./use-portfolio-rows";
import { ScrollLoader } from "../orderbook/scroll-loader";
import {
  progressText,
  portfolioQuery,
  portfolioSort,
  portfolioSortOptions,
  portfolioRelations,
} from "./model";
import type { PortfolioFilters, PortfolioItem, PortfolioState } from "./model";
import "./portfolio.css";
const keyOf = (item: PortfolioItem) => item.gift.gift.id;
const initialFilters: PortfolioFilters = {
  collection: "",
  traits: {},
  number: "",
  sort: "value_desc",
};
function RefreshButton({
  busy,
  after,
  refresh,
}: {
  busy: boolean;
  after?: string | null;
  refresh: () => void;
}) {
  const [now, setNow] = useState(Date.now());
  useEffect(() => setNow(Date.now()), [after]);
  const cooldown = after
    ? Math.max(0, Math.ceil((Date.parse(after) - now) / 1000))
    : 0;
  useEffect(() => {
    if (!cooldown || document.hidden) return;
    const timer = setTimeout(() => setNow(Date.now()), 1000);
    return () => clearTimeout(timer);
  }, [cooldown, after]);
  useEffect(() => {
    const tick = () => setNow(Date.now());
    document.addEventListener("visibilitychange", tick);
    return () => document.removeEventListener("visibilitychange", tick);
  }, []);
  return (
    <button
      className={`portfolio-refresh${busy ? " busy" : ""}`}
      disabled={busy || cooldown > 0}
      onClick={refresh}
      aria-label="Refresh gifts"
      title={
        busy
          ? "Checking gifts"
          : cooldown
            ? `Recheck available in ${cooldown}s`
            : "Recheck Telegram gifts"
      }
    >
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M20 7v5h-5M4 17v-5h5M6 7a7 7 0 0 1 12-1l2 6M4 12l2 6a7 7 0 0 0 12-1" />
      </svg>
      {cooldown > 0 && !busy && (
        <span className="portfolio-cooldown" aria-hidden="true">
          {cooldown}s
        </span>
      )}
    </button>
  );
}
function Owner({
  user,
  self,
  open,
  setOpen,
  choose,
}: {
  user?: number;
  self?: number;
  open: boolean;
  setOpen: (value: boolean) => void;
  choose: (value: number) => void;
}) {
  const [input, setInput] = useState(user?.toString() || "");
  const host = useRef<HTMLDivElement>(null),
    trigger = useRef<HTMLButtonElement>(null),
    field = useRef<HTMLInputElement>(null);
  const close = () => {
    setOpen(false);
    trigger.current?.focus({ preventScroll: true });
  };
  useNavigationBack(open, close, 15);
  useEffect(() => {
    if (!open) return;
    field.current?.focus({ preventScroll: true });
    const outside = (e: PointerEvent) => {
      if (!host.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("pointerdown", outside);
    return () => document.removeEventListener("pointerdown", outside);
  }, [open, setOpen]);
  return (
    <div
      className="portfolio-owner"
      ref={host}
      onKeyDown={(e) => {
        if (e.key === "Escape" && open) {
          e.preventDefault();
          e.stopPropagation();
          close();
        }
      }}
    >
      <h1>
        <button
          ref={trigger}
          className="portfolio-owner-trigger"
          aria-haspopup="dialog"
          aria-expanded={open}
          onClick={() => setOpen(!open)}
        >
          {user ? (user === self ? "My gifts" : `#${user}`) : "Open portfolio"}
          <svg viewBox="0 0 16 16" aria-hidden="true">
            <path d="m4 6 4 4 4-4" />
          </svg>
        </button>
      </h1>
      {open && (
        <form
          className="portfolio-owner-menu"
          role="dialog"
          aria-label="Open portfolio"
          onSubmit={(e) => {
            e.preventDefault();
            const id = portfolioUser(input);
            if (id) {
              choose(id);
              close();
            }
          }}
        >
          <label htmlFor="portfolio-user">Telegram user ID</label>
          <div>
            <input
              ref={field}
              id="portfolio-user"
              inputMode="numeric"
              pattern="[0-9]+"
              value={input}
              onChange={(e) => setInput(e.target.value)}
              required
            />
            <button type="submit">Open</button>
          </div>
          {self && self !== user && (
            <button
              type="button"
              className="portfolio-self"
              onClick={() => {
                choose(self);
                close();
              }}
            >
              My gifts
            </button>
          )}
        </form>
      )}
    </div>
  );
}
export function PortfolioPage(props: {
  context: TelegramContext;
  initialUser?: number;
}) {
  return (
    <SessionBoundary context={props.context}>
      <PortfolioView {...props} />
    </SessionBoundary>
  );
}
function PortfolioView({
  context,
  initialUser,
}: {
  context: TelegramContext;
  initialUser?: number;
}) {
  const telegram = useSession(),
    navigation = useNavigation();
  const [user, setUser] = useState(initialUser),
    [currency, setCurrency] = useSharedCurrency();
  const [filters, setFilters] = usePageState<PortfolioFilters>(
    `portfolio.filters.${user ?? "empty"}`,
    () => {
      try {
        return portfolioFiltersFromParams(selectionAt(location));
      } catch {
        return initialFilters;
      }
    },
  );
  const [accountOpen, setAccountOpen] = useState(false);
  useBackgroundPlaybackSuspension(accountOpen);
  const [open, setOpen] = useState<TraitKey>(),
    [selectedId, setSelected] = useState<string>(),
    [previewIds, setPreviewIds] = useState<string[]>([]),
    [ownerOpen, setOwnerOpen] = useState(!initialUser && !context.initData);
  const controls = useRef<HTMLElement>(null);
  const portfolio = usePortfolio(user, currency);
  const rows = usePortfolioRows(
    user,
    currency,
    filters,
    portfolio.state?.valuation,
    portfolio.resubscribe,
    portfolio.ready,
    !!selectedId,
  );
  const filterKey = rows.queryKey;
  const shareOffset = useRef(0);
  const sharingScope = useMemo(
    () => JSON.stringify([user, currency, filterKey, selectedId]),
    [user, currency, filterKey, selectedId],
  );
  const sharing = useSharing(context, sharingScope);
  const share = sharing.available
    ? (slug: string, currency_id: string) => {
        void sharing.share({ type: "gift", slug, currency_id });
      }
    : undefined;
  const previousFilter = useRef({ user, key: filterKey });
  useLayoutEffect(() => {
    shareOffset.current = 0;
    if (
      previousFilter.current.user === user &&
      previousFilter.current.key !== filterKey
    )
      window.scrollTo(0, 0);
    previousFilter.current = { user, key: filterKey };
  }, [filterKey, user]);
  const items = rows.items;
  const selected = useMemo(
    () =>
      selectedId ? items.find((item) => keyOf(item) === selectedId) : undefined,
    [items, selectedId],
  );
  useEffect(() => {
    if (selectedId && !selected) setSelected(undefined);
  }, [selectedId, selected]);
  const selectedCollection =
    selected?.resolved?.collection_id || filters.collection;
  const demanded = useMemo(
    () => [selectedCollection, ...previewIds].filter(Boolean),
    [selectedCollection, previewIds],
  );
  const catalog = useCatalog(0, filters.collection, demanded),
    availability = useMarketStatus(!!user);
  const pickerFilters = useMemo(
    () => resolveFilterSelectors(filters.traits, catalog.options),
    [filters.traits, catalog.options],
  );
  const self = portfolioUser(telegram.identity?.user.id);
  const chooseUser = (id: number, replace = false) => {
    setSelected(undefined);
    setOpen(undefined);
    setOwnerOpen(false);
    if (navigation) navigation.go("portfolio", id, replace);
    else {
      setUser(id);
      setFilters(initialFilters);
    }
  };
  useEffect(() => {
    if (!user && self) chooseUser(self, true);
  }, [self, user]);
  useEffect(() => {
    if (user) {
      const url = new URL(location.href);
      url.pathname = `/portfolio/${user}`;
      let params: URLSearchParams;
      try {
        params = portfolioParams(filters, currency);
      } catch {
        return;
      } // Partial/invalid number input stays editable without publishing a broken link.
      history.replaceState(
        history.state,
        "",
        selectionHref(url.pathname, params),
      );
      navigation?.rememberLocation();
    }
  }, [user, currency, filters]);
  useNavigationBack(!!open, () => setOpen(undefined));
  const filtered = items;
  const visible = useCallback(
    (start: number, end: number, anchor = start) => {
      shareOffset.current = Math.floor(Math.max(start, anchor) / 30) * 30;
      const ids = [
        ...new Set(
          [
            ...filtered.slice(Math.max(start, anchor), end),
            ...filtered.slice(start, Math.max(start, anchor)),
          ].flatMap((item) =>
            item.resolved ? [item.resolved.collection_id] : [],
          ),
        ),
      ];
      setPreviewIds((old) =>
        JSON.stringify(old) === JSON.stringify(ids) ? old : ids,
      );
    },
    [filtered],
  );
  const summary = rows.selection ?? {
    count: 0,
    total: undefined,
    unverified: 0,
    unpriced: 0,
    collections: [],
  };
  const isFiltered = !!(
    filters.collection ||
    filters.number ||
    Object.values(filters.traits).some((v) => v?.length)
  );
  const collections = useMemo(() => {
    const owned = new Set(portfolio.state?.valuation?.collections ?? []);
    return catalog.types.filter((c) => owned.has(c.id));
  }, [catalog.types, portfolio.state?.valuation?.collections]);
  const collectionIds = useMemo(
    () => collections.map((c) => c.id),
    [collections],
  );
  const changeTrait = useCallback(
    (id: TraitKey, value?: string[]) =>
      setFilters((old) => ({
        ...old,
        traits: {
          ...old.traits,
          [id]: value
            ? aliasFilters({ [id]: value }, catalog.options)[id]
            : undefined,
        },
      })),
    [setFilters, catalog.options],
  );
  const choose = (collection: string) => {
    setFilters((old) => ({
      ...old,
      collection,
      traits: {
        backdrops: aliasFilters(old.traits, catalog.options).backdrops,
        patterns: aliasFilters(old.traits, catalog.options).patterns,
      },
    }));
    setOpen(undefined);
  };
  const state = portfolio.state,
    report = state?.valuation;
  const giftCount = report?.count.toLocaleString();
  const busy =
    !!state &&
    ["queued", "loading_inventory", "valuing"].includes(state.progress.stage);
  const partial = state?.inventory?.complete === false;
  const error = portfolio.error || state?.error;
  const total = isFiltered ? summary.total : report?.total;
  const sortKey = filters.sort.startsWith("number")
    ? "number"
    : filters.sort.startsWith("position")
      ? "position"
      : "value";
  const ascending = filters.sort.endsWith("_asc");
  return (
    <FloorsProvider
      currency={currency}
      collection={selectedCollection}
      collections={collectionIds}
      order={selected ? portfolioSubject(selected, currency) : undefined}
    >
      <div className="workspace">
        <main className="orderbook market-shell portfolio-shell">
          <aside
            className="market-sidebar"
            id="portfolio-filters"
            tabIndex={-1}
            aria-label="Portfolio controls"
            ref={controls}
            data-scroll-container
          >
            <AppHeader
              context={context}
              currencies={["GRAM", "XTR"]}
              currency={currency}
              changeCurrency={setCurrency}
              accountOpen={accountOpen}
              setAccountOpen={setAccountOpen}
            />
            <AppNavigation page="portfolio" />
            <button
              className="collection-all"
              aria-label="All gifts"
              aria-describedby={report ? "portfolio-all-count" : undefined}
              aria-pressed={!filters.collection}
              title="Show all portfolio collections"
              onClick={() => choose("")}
            >
              <GiftPlaceholder active={false} loading={false} />
              <span className="collection-all-label">All gifts</span>
              {report && (
                <span
                  className="collection-all-count"
                  aria-hidden="true"
                  title={`${giftCount} gifts in this portfolio`}
                >
                  {giftCount}
                </span>
              )}
              <span className="collection-all-check" aria-hidden="true">
                ✓
              </span>
              {report && (
                <span className="sr" id="portfolio-all-count">
                  {giftCount} gifts in this portfolio
                </span>
              )}
            </button>
            <CollectionRail
              collections={collections}
              selected={filters.collection}
              select={choose}
            />
            {
              <>
                <TraitPickers
                  key={filters.collection}
                  collection={catalog.item}
                  options={catalog.options}
                  filters={resolveFilterSelectors(
                    filters.traits,
                    catalog.options,
                  )}
                  change={changeTrait}
                  open={open}
                  setOpen={setOpen}
                  loading={catalog.optionsLoading}
                  loadError={catalog.optionsError}
                />
                <TraitPreview
                  collection={catalog.item}
                  options={catalog.options}
                  filters={resolveFilterSelectors(
                    filters.traits,
                    catalog.options,
                  )}
                  reset={() => {
                    setFilters((old) => ({ ...old, traits: {} }));
                    setOpen(undefined);
                  }}
                />
              </>
            }
            <div className="portfolio-filters">
              <label>
                Gift #
                <input
                  type="number"
                  min="1"
                  value={filters.number}
                  onChange={(e) =>
                    setFilters({ ...filters, number: e.target.value })
                  }
                />
              </label>
              <div className="sort-control portfolio-sort">
                <SortPicker
                  options={portfolioSortOptions}
                  value={sortKey}
                  change={(key) =>
                    setFilters({
                      ...filters,
                      sort: portfolioSort(
                        key,
                        key === "position" ? true : ascending,
                      ),
                    })
                  }
                />
                <button
                  className="sort-direction"
                  aria-label={ascending ? "Sort descending" : "Sort ascending"}
                  onClick={() =>
                    setFilters({
                      ...filters,
                      sort: portfolioSort(sortKey, !ascending),
                    })
                  }
                >
                  {ascending ? "↑" : "↓"}
                </button>
              </div>
            </div>
          </aside>
          <section className="market-results portfolio-content">
            <header className="portfolio-heading">
              <Owner
                user={user}
                self={self}
                open={ownerOpen}
                setOpen={setOwnerOpen}
                choose={chooseUser}
              />
            </header>
            {user && (
              <section
                className="portfolio-summary"
                aria-label="Portfolio estimate"
              >
                <div className="portfolio-summary-main">
                  <div>
                    <p className="portfolio-estimate-label">
                      {isFiltered ? "Selection estimate" : "Estimate"}
                    </p>
                    <div className="portfolio-total">
                      {total != null ? (
                        <Quote
                          price={total}
                          currency={currency}
                          format="rounded"
                        />
                      ) : (
                        <span>—</span>
                      )}
                    </div>
                  </div>
                  <div className="portfolio-check">
                    {report && items.length > 0 && (
                      <ShareButton
                        sharing={sharing}
                        label="Share portfolio page"
                        click={() => {
                          const query = portfolioQuery(user, currency, {
                            ...filters,
                            traits: pickerFilters,
                          });
                          if (query)
                            void sharing.share({
                              type: "portfolio",
                              query: {
                                ...query,
                                offset: shareOffset.current,
                                limit: 30,
                                version: rows.version,
                              },
                            });
                        }}
                      />
                    )}

                    {state?.inventory && (
                      <RelativeAge
                        at={state.inventory.checked_at}
                        label="Inventory checked"
                        className="portfolio-checked"
                        ago
                      />
                    )}
                    <RefreshButton
                      busy={busy}
                      after={state?.refresh_after}
                      refresh={() => void portfolio.refresh()}
                    />
                  </div>
                </div>
                <ShareStatus sharing={sharing} />
                {report && (
                  <div className="portfolio-meta">
                    <span>
                      {summary.count.toLocaleString()}
                      {isFiltered
                        ? ` / ${(report?.count ?? 0).toLocaleString()}`
                        : ""}{" "}
                      gifts
                    </span>
                    {isFiltered && report.total != null ? (
                      <span className="portfolio-full-total">
                        of{" "}
                        <Quote
                          price={report.total}
                          currency={currency}
                          format="rounded"
                        />
                      </span>
                    ) : (
                      <span>{summary.collections.length} collections</span>
                    )}
                    {summary.unverified > 0 && (
                      <span title={portfolioRelations.unverified.description}>
                        {summary.unverified.toLocaleString()}{" "}
                        {portfolioRelations.unverified.label.toLowerCase()}
                      </span>
                    )}
                    {summary.unpriced > 0 && (
                      <span
                        className="portfolio-unpriced"
                        title="Gifts without an estimate are excluded from the total"
                      >
                        {summary.unpriced} unpriced
                      </span>
                    )}
                  </div>
                )}
                <div
                  className={`portfolio-progress${!!state && state.progress.stage === "ready" && !partial && !error ? " quiet" : ""}`}
                  role="status"
                  aria-live="polite"
                >
                  <span>
                    {error
                      ? report
                        ? "Update failed · previous data retained"
                        : "Unable to update portfolio"
                      : user
                        ? progressText(state)
                        : ""}
                  </span>
                  {busy && (
                    <progress
                      aria-label="Portfolio check progress"
                      max={
                        state?.progress.stage === "valuing"
                          ? state.progress.collectibles
                          : (state?.progress.total ?? undefined)
                      }
                      value={
                        state?.progress.stage === "valuing"
                          ? state.progress.valued
                          : state?.progress.total
                            ? state.progress.checked
                            : undefined
                      }
                    />
                  )}
                </div>
                {report &&
                  (report.stale || availability.status !== "available") && (
                    <p className="portfolio-delayed">Market updates delayed</p>
                  )}
              </section>
            )}
            {error && (
              <p className="error" role="alert">
                {error}
              </p>
            )}
            {!user ? (
              <p className="empty">
                Open your portfolio in Telegram, or enter a user ID.
              </p>
            ) : (
              <VirtualGiftGrid
                orders={filtered}
                keyOf={keyOf}
                busy={busy || rows.loading}
                visible={visible}
                label="Portfolio gifts"
                render={(item, index) => (
                  <PortfolioGiftCard
                    item={item}
                    currency={currency}
                    options={catalog.previewOptions}
                    names={catalog.names}
                    selected={selectedId === item.gift.gift.id}
                    select={setSelected}
                    position={index + 1}
                    total={summary.count}
                  />
                )}
              />
            )}
            {user && (
              <ScrollLoader
                loading={rows.loading}
                hasMore={rows.hasMore}
                disabled={!!rows.error}
                loadMore={rows.loadMore}
              />
            )}
            {rows.error && (
              <p className="error" role="alert">
                {rows.error} <button onClick={rows.retry}>Retry</button>
              </p>
            )}
            {!busy &&
              !rows.loading &&
              !rows.error &&
              user &&
              state?.inventory &&
              filtered.length === 0 && (
                <p className="empty">
                  {report?.count
                    ? "No gifts match these filters."
                    : "No collectible gifts available."}
                </p>
              )}
          </section>
        </main>
        <BackToTop
          target={controls}
          blocked={!!selected || ownerOpen || accountOpen}
        />
        {selected?.resolved && (
          <PortfolioDetails
            item={selected}
            currency={currency}
            options={catalog.previewOptions}
            names={catalog.names}
            close={() => setSelected(undefined)}
            share={share}
          />
        )}
      </div>
    </FloorsProvider>
  );
}
