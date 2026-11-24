import { useCallback, useEffect, useMemo, useRef, useState } from "react";
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
import { FloorsProvider } from "../valuation/floors";
import { CurrencyPicker, SortPicker } from "../ui/pickers";
import { Quote } from "../ui/market-controls";
import { RelativeAge } from "../ui/relative-age";
import { BrandMark } from "../brand/brand-mark";
import { GiftCardView } from "../previews/gift-card";
import { GiftDetailsView } from "../previews/gift-details";
import { VirtualGiftGrid } from "../orderbook/virtual-gift-grid";
import { BackToTop } from "../ui/back-to-top";
import { useOrderValuation } from "../valuation/use-order-valuation";
import type { TraitKey, CatalogOption, GiftMedia } from "../market";
import type { ValuationSubject } from "../order-target";
import { useMarketStatus } from "../use-market-status";
import { usePortfolio } from "./use-portfolio";
import { selectItems, progressText, portfolioSummary } from "./model";
import type { PortfolioFilters, PortfolioItem, PortfolioState } from "./model";
import "./portfolio.css";
const keyOf = (item: PortfolioItem) => item.gift.gift.id;
const visible = () => {};
const emptyItems: PortfolioItem[] = [];
const initialFilters: PortfolioFilters = {
  collection: "",
  traits: {},
  number: "",
  min: "",
  max: "",
  sort: "value_desc",
};
function subject(
  item: PortfolioItem,
  currency: string,
): ValuationSubject | undefined {
  const g = item.resolved;
  return g
    ? {
        book: { collection_id: g.collection_id, currency_id: currency },
        target: { kind: "gift", id: g.id, number: g.number, traits: g.traits },
      }
    : undefined;
}
function modelMedia(item: PortfolioItem): GiftMedia | undefined {
  const id = item.resolved?.traits.model;
  if (!id) return;
  const base = `/media/models/${encodeURIComponent(id)}`;
  return {
    thumbnail_url: `${base}/thumbnail`,
    preview_url: `${base}/preview`,
    animation_url: `${base}/animation`,
  };
}
function PortfolioDetails({
  item,
  currency,
  options,
  names,
  close,
  share,
}: {
  item: PortfolioItem;
  currency: string;
  options: ReadonlyMap<string, CatalogOption>;
  names: ReadonlyMap<string, string>;
  close: () => void;
  share?: (slug: string, currency: string) => void;
}) {
  const gift = subject(item, currency)!;
  const valuation = useOrderValuation(gift);
  return (
    <GiftDetailsView
      subject={gift}
      valuation={valuation}
      collectionId={gift.book.collection_id}
      currency={currency}
      collectionName={item.gift.gift.slug.split("-")[0]}
      options={options}
      names={names}
      collectionPreview={modelMedia(item)}
      slug={item.gift.gift.slug}
      close={close}
      share={share}
    />
  );
}
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
    () => initialFilters,
  );
  const [open, setOpen] = useState<TraitKey>(),
    [selected, setSelected] = useState<PortfolioItem>(),
    [ownerOpen, setOwnerOpen] = useState(!initialUser && !context.initData);
  const controls = useRef<HTMLElement>(null);
  const selectedCollection =
    selected?.resolved?.collection_id || filters.collection;
  const catalog = useCatalog(0, selectedCollection),
    portfolio = usePortfolio(user, currency),
    availability = useMarketStatus(!!user);
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
      url.searchParams.set("currency", currency);
      history.replaceState(history.state, "", url);
      navigation?.rememberLocation();
    }
  }, [user, currency]);
  useNavigationBack(!!open, () => setOpen(undefined));
  const items = portfolio.state?.valuation?.items || emptyItems;
  const filtered = useMemo(() => selectItems(items, filters), [items, filters]);
  const summary = useMemo(() => portfolioSummary(filtered), [filtered]);
  const isFiltered = !!(
    filters.collection ||
    filters.number ||
    filters.min ||
    filters.max ||
    Object.values(filters.traits).some((v) => v?.length)
  );
  const collections = useMemo(() => {
    const owned = new Set(
      items.flatMap((i) => (i.resolved ? [i.resolved.collection_id] : [])),
    );
    return catalog.types.filter((c) => owned.has(c.id));
  }, [catalog.types, items]);
  const collectionIds = useMemo(
    () => collections.map((c) => c.id),
    [collections],
  );
  const changeTrait = useCallback(
    (id: TraitKey, value?: string[]) =>
      setFilters((old) => ({ ...old, traits: { ...old.traits, [id]: value } })),
    [setFilters],
  );
  const choose = (collection: string) => {
    setFilters((old) => ({ ...old, collection, traits: {} }));
    setOpen(undefined);
  };
  const state = portfolio.state,
    report = state?.valuation;
  const busy =
    !!state &&
    ["queued", "loading_inventory", "valuing"].includes(state.progress.stage);
  const partial = state?.inventory?.complete === false;
  const error = portfolio.error || state?.error;
  const total = isFiltered ? summary.total : report?.total;
  const sortKey = filters.sort.startsWith("number") ? "number" : "price";
  const ascending = filters.sort.endsWith("_asc");
  const share = context.app?.switchInlineQuery
    ? (slug: string, unit: string) =>
        context.app!.switchInlineQuery!(`${slug} ${unit}`, [
          "users",
          "groups",
          "channels",
        ])
    : undefined;
  return (
    <FloorsProvider
      currency={currency}
      collection={selectedCollection}
      collections={collectionIds}
      order={selected ? subject(selected, currency) : undefined}
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
            <div className="sidebar-heading">
              <BrandMark animated />
              <CurrencyPicker
                currencies={["GRAM", "XTR"]}
                value={currency}
                change={setCurrency}
              />
            </div>
            <AppNavigation page="portfolio" />
            <button
              className="portfolio-any"
              aria-pressed={!filters.collection}
              onClick={() => choose("")}
            >
              All gifts <span>{items.length || ""}</span>
            </button>
            <CollectionRail
              collections={collections}
              selected={filters.collection}
              select={choose}
            />
            {filters.collection && (
              <TraitPickers
                collection={catalog.item}
                options={catalog.options}
                filters={filters.traits}
                change={changeTrait}
                open={open}
                setOpen={setOpen}
                loading={catalog.optionsLoading}
                loadError={catalog.optionsError}
              />
            )}
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
              <label>
                Estimate from
                <input
                  type="text"
                  inputMode="decimal"
                  pattern="[0-9]+([.][0-9]+)?"
                  placeholder="Min"
                  value={filters.min}
                  onChange={(e) =>
                    setFilters({ ...filters, min: e.target.value })
                  }
                />
              </label>
              <label>
                Estimate to
                <input
                  type="text"
                  inputMode="decimal"
                  pattern="[0-9]+([.][0-9]+)?"
                  placeholder="Max"
                  value={filters.max}
                  onChange={(e) =>
                    setFilters({ ...filters, max: e.target.value })
                  }
                />
              </label>
            </div>
            <div className="sort-control portfolio-sort">
              <SortPicker
                value={sortKey}
                change={(value) =>
                  setFilters({
                    ...filters,
                    sort:
                      value === "number"
                        ? ascending
                          ? "number_asc"
                          : "number_desc"
                        : ascending
                          ? "value_asc"
                          : "value_desc",
                  })
                }
              />
              <button
                className="sort-direction"
                aria-label={ascending ? "Sort descending" : "Sort ascending"}
                onClick={() =>
                  setFilters({
                    ...filters,
                    sort:
                      sortKey === "number"
                        ? ascending
                          ? "number_desc"
                          : "number_asc"
                        : ascending
                          ? "value_desc"
                          : "value_asc",
                  })
                }
              >
                {ascending ? "↑" : "↓"}
              </button>
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
                {report && (
                  <div className="portfolio-meta">
                    <span>
                      {summary.count.toLocaleString()}
                      {isFiltered
                        ? ` / ${items.length.toLocaleString()}`
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
                      <span>{summary.collections} collections</span>
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
                busy={busy}
                visible={visible}
                label="Portfolio gifts"
                render={(item, index) => {
                  const gift = subject(item, currency);
                  return (
                    <GiftCardView
                      subject={gift}
                      options={catalog.options}
                      collectionName={
                        catalog.names.get(item.resolved?.collection_id || "") ||
                        item.gift.gift.slug.split("-")[0]
                      }
                      media={modelMedia(item)}
                      identity={`#${item.gift.gift.number.toLocaleString()}`}
                      price={item.estimate ?? undefined}
                      currency={currency}
                      format="rounded"
                      position={index + 1}
                      total={filtered.length}
                      className="portfolio-card"
                      selected={selected?.gift.gift.id === item.gift.gift.id}
                      action={
                        gift ? (
                          <button
                            className="card-open"
                            aria-label={`View ${item.gift.gift.slug}`}
                            onClick={() => setSelected(item)}
                            aria-haspopup="dialog"
                          />
                        ) : (
                          <a
                            className="card-open"
                            aria-label={`Open ${item.gift.gift.slug} in Telegram`}
                            href={`https://t.me/nft/${encodeURIComponent(item.gift.gift.slug)}`}
                            target="_blank"
                            rel="noreferrer"
                          />
                        )
                      }
                    />
                  );
                }}
              />
            )}
            {!busy && user && state?.inventory && filtered.length === 0 && (
              <p className="empty">
                {items.length
                  ? "No gifts match these filters."
                  : "No collectible gifts available."}
              </p>
            )}
          </section>
        </main>
        <BackToTop target={controls} blocked={!!selected || ownerOpen} />
        {selected?.resolved && (
          <PortfolioDetails
            item={selected}
            currency={currency}
            options={catalog.options}
            names={catalog.names}
            close={() => setSelected(undefined)}
            share={share}
          />
        )}
      </div>
    </FloorsProvider>
  );
}
