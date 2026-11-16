import React, { useEffect, useMemo, useState } from "react";
import { createRoot } from "react-dom/client";
import {
  amount,
  ApiFailure,
  books,
  catalog,
  request,
  text,
  dateInput,
  timestamp,
} from "./market";
import type { Book, Entry, Order, Page, Predicate, Query } from "./market";
import { Field } from "./filters";
import { initializeTelegram, launchSelection } from "./telegram";
import type { TelegramContext } from "./telegram";
import {
  useTelegram,
  useTelegramBack,
  TelegramAccount,
} from "./telegram-ui";
import "./style.css";
type ItemType = Extract<Entry, { kind: "item_type" }>;
type Attribute = Extract<Entry, { kind: "attribute" }>;
type Option = Extract<Entry, { kind: "attribute_option" }>;
type Choice = {
  item: string;
  currency: string;
  venue: string;
  mode: "ask" | "bid" | "executed";
  sort: "price_asc" | "price_desc";
  filters: Record<string, Predicate>;
  min: string;
  max: string;
  from: string;
  until: string;
};
function saved(context: TelegramContext): Choice {
  const p = launchSelection(context, location.search);
  let filters = {};
  try {
    const parsed = JSON.parse(p.get("filters") || "{}");
    if (parsed && typeof parsed === "object" && !Array.isArray(parsed))
      filters = parsed;
  } catch {}
  return {
    item: p.get("item") || "",
    currency: p.get("currency") || "",
    venue: p.get("venue") || "",
    mode:
      p.get("mode") === "bid"
        ? "bid"
        : p.get("mode") === "executed"
          ? "executed"
          : "ask",
    sort: p.get("sort") === "price_desc" ? "price_desc" : "price_asc",
    filters,
    min: p.get("min") || "",
    max: p.get("max") || "",
    from: p.get("from") || "",
    until: p.get("until") || "",
  };
}
function usePage(
  query: Query,
  executed: boolean,
  period: { from: string; until: string },
  paused: boolean,
  refresh: number,
) {
  const signature = JSON.stringify({ query, executed, period });
  const [page, setPage] = useState<Page<Order>>({
      data: [],
      next_page_token: "",
      cursor: "",
    }),
    [status, setStatus] = useState("Connecting"),
    [error, setError] = useState("");
  useEffect(() => {
    const abort = new AbortController();
    let socket: WebSocket | undefined, timer: ReturnType<typeof setTimeout>;
    setPage({ data: [], next_page_token: "", cursor: "" });
    setError("");
    if (!query.scope.item_type_id || !query.scope.currency_id) {
      setStatus("Choose a collection");
      return () => abort.abort();
    }
    const failed = (e: unknown) => {
      if (!abort.signal.aborted) {
        setError(e instanceof Error ? e.message : String(e));
        setStatus("Unavailable");
      }
    };
    if (executed || paused) {
      setStatus("Loading");
      const body = executed
        ? {
            scope: query.scope,
            limit: query.limit,
            page_token: query.page_token,
            period: { from: period.from || null, until: period.until || null },
          }
        : query;
      void request<Page<Order>>(
        executed ? "/executions/query" : "/orders/query",
        body,
        abort.signal,
      )
        .then((page) => {
          if (!abort.signal.aborted) {
            setPage(page);
            setStatus(paused ? "Paused" : "Snapshot");
          }
        })
        .catch(failed);
    } else {
      const connect = () => {
        if (abort.signal.aborted) return;
        setStatus("Connecting");
        socket = new WebSocket(
          `${location.protocol === "https:" ? "wss:" : "ws:"}//${location.host}/ws`,
        );
        socket.onopen = () =>
          socket?.send(
            JSON.stringify({ type: "subscribe", id: "offers", query }),
          );
        socket.onmessage = (e) => {
          if (abort.signal.aborted) return;
          try {
            const frame = JSON.parse(e.data);
            if (frame.type === "page") {
              setPage(frame.page);
              setStatus(frame.page.sync?.state === "live" ? "Live" : "Stale");
              setError("");
            } else if (frame.type === "error") {
              setError(frame.error.message);
              setStatus("Unavailable");
              if (
                frame.error.code === "unavailable" ||
                frame.error.code === "capacity_exceeded"
              )
                socket?.close();
            }
          } catch {
            failed(new Error("Invalid market response"));
          }
        };
        socket.onclose = () => {
          if (!abort.signal.aborted) {
            setStatus("Reconnecting");
            timer = setTimeout(connect, 1000);
          }
        };
        socket.onerror = () => socket?.close();
      };
      connect();
    }
    return () => {
      abort.abort();
      clearTimeout(timer);
      if (socket) {
        socket.onclose = null;
        socket.close();
      }
    };
  }, [signature, paused, refresh]);
  return { page, status, error };
}
function App({ context }: { context: TelegramContext }) {
  const telegram = useTelegram(context);
  const [accountOpen, setAccountOpen] = useState(false);
  const [choice, setChoice] = useState(() => saved(context)),
    [draft, setDraft] = useState(() => saved(context)),
    [types, setTypes] = useState<ItemType[]>([]),
    [directory, setDirectory] = useState<Book[]>([]),
    [venues, setVenues] = useState<Entry[]>([]),
    [attributes, setAttributes] = useState(new Map<string, Attribute>()),
    [options, setOptions] = useState(new Map<string, Option>()),
    [startupError, setStartupError] = useState(""),
    [search, setSearch] = useState(""),
    [refresh, setRefresh] = useState(0),
    [paused, setPaused] = useState(false),
    [pageSize, setPageSize] = useState(50),
    [anchors, setAnchors] = useState<string[]>([""]),
    [selected, setSelected] = useState<Order>(),
    [showFilters, setShowFilters] = useState(false);
  useEffect(() => {
    const c = new AbortController();
    let retry: ReturnType<typeof setTimeout>;
    void Promise.all([
      catalog("item_type", c.signal),
      catalog("venue", c.signal),
      catalog("attribute", c.signal),
      books(c.signal),
    ])
      .then(([items, venues, attrs, books]) => {
        if (c.signal.aborted) return;
        setTypes(
          items
            .filter((e): e is ItemType => e.kind === "item_type")
            .sort((a, b) => a.name.localeCompare(b.name)),
        );
        setVenues(venues);
        setAttributes(
          new Map(
            attrs
              .filter((e): e is Attribute => e.kind === "attribute")
              .map((e) => [e.id, e]),
          ),
        );
        setDirectory(books);
        setStartupError("");
        if (books[0])
          setChoice((old) =>
            old.item && old.currency
              ? old
              : {
                  ...old,
                  item: old.item || books[0].item_type_id,
                  currency: old.currency || books[0].currency_id,
                },
          );
      })
      .catch((e) => {
        if (!c.signal.aborted) {
          setStartupError(e.message);
          if (
            e instanceof ApiFailure &&
            ["unavailable", "capacity_exceeded"].includes(e.code)
          )
            retry = setTimeout(() => setRefresh((v) => v + 1), 2000);
        }
      });
    return () => {
      c.abort();
      clearTimeout(retry);
    };
  }, [refresh]);
  const item = types.find((t) => t.id === choice.item);
  const names = useMemo(
    () =>
      new Map<string, string>(
        [...types, ...venues, ...attributes.values(), ...options.values()].map(
          (v) => [v.id, v.name],
        ),
      ),
    [types, venues, attributes, options],
  );
  useEffect(() => {
    setDraft(choice);
    const p = new URLSearchParams();
    for (const [key, value] of Object.entries(choice)) {
      if (key === "filters") {
        if (Object.keys(value as object).length)
          p.set(key, JSON.stringify(value));
      } else if (value) p.set(key, String(value));
    }
    history.replaceState(null, "", `${location.pathname}?${p}`);
    setAnchors([""]);
    setSelected(undefined);
  }, [JSON.stringify(choice)]);
  useEffect(() => {
    const pop = () => setChoice(saved(context));
    window.addEventListener("popstate", pop);
    return () => window.removeEventListener("popstate", pop);
  }, []);
  useEffect(() => {
    if (!item) return;
    const c = new AbortController();
    const ids = [
      ...new Set(
        Object.values(item.attributes).flatMap(
          (rule) => rule.allowed_values || [],
        ),
      ),
    ];
    setOptions(new Map());
    void (async () => {
      const rows: Entry[] = [];
      for (let n = 0; n < ids.length && !c.signal.aborted; n += 500)
        rows.push(
          ...(await catalog(
            "attribute_option",
            c.signal,
            ids.slice(n, n + 500),
          )),
        );
      if (!c.signal.aborted)
        setOptions(
          new Map(
            rows
              .filter((v): v is Option => v.kind === "attribute_option")
              .map((v) => [v.id, v]),
          ),
        );
    })().catch(() => {});
    return () => c.abort();
  }, [item]);
  const query: Query = {
    scope: {
      item_type_id: choice.item,
      currency_id: choice.currency,
      venue_id: choice.venue || null,
      modes: [choice.mode],
      attributes: choice.mode === "bid" ? {} : choice.filters,
      price:
        choice.min || choice.max
          ? { min: choice.min || null, max: choice.max || null }
          : null,
    },
    sort: choice.sort,
    limit: pageSize,
    page_token: anchors.at(-1) || "",
  };
  const { page, status, error } = usePage(
    query,
    choice.mode === "executed",
    { from: choice.from, until: choice.until },
    paused,
    refresh,
  );
  useEffect(
    () =>
      setSelected((current) =>
        current
          ? page.data.find(
              (order) =>
                order.id === current.id &&
                order.book.venue_id === current.book.venue_id,
            )
          : undefined,
      ),
    [page.data],
  );
  useTelegramBack(context, accountOpen || !!selected || showFilters, () => {
    if (accountOpen) setAccountOpen(false);
    else if (selected) setSelected(undefined);
    else setShowFilters(false);
  });
  const set = (next: Choice) => {
    setChoice(next);
    setShowFilters(false);
  };
  const selectedType = (id: string) => {
    const available = directory.filter((b) => b.item_type_id === id);
    set({
      ...choice,
      item: id,
      currency: available.some((b) => b.currency_id === choice.currency)
        ? choice.currency
        : available[0]?.currency_id || choice.currency,
      venue: "",
      filters: {},
    });
  };
  const availableTypes = types.filter((t) =>
    directory.some((b) => b.item_type_id === t.id),
  );
  const currencies = [
    ...new Set(
      directory
        .filter((b) => b.item_type_id === choice.item)
        .map((b) => b.currency_id),
    ),
  ].sort();
  const venueIds = [
    ...new Set(
      directory
        .filter(
          (b) =>
            b.item_type_id === choice.item && b.currency_id === choice.currency,
        )
        .map((b) => b.venue_id),
    ),
  ].sort();
  const filterCount =
    Object.keys(choice.filters).length + (choice.min || choice.max ? 1 : 0);
  return (
    <div className={`workspace ${selected ? "with-detail" : ""}`}>
      <header className="topbar">
        <a className="brand" href="/">
          ◈{" "}
          <span>
            GIFT<span className="brand-light">MARKETS</span>
          </span>
        </a>
        <span className="top-caption">One view. Every venue.</span>
        <TelegramAccount
          telegram={telegram}
          context={context}
          open={accountOpen}
          setOpen={setAccountOpen}
          selection={new URLSearchParams(location.search)}
        />
        <a
          className="api-link"
          href="/openapi.json"
          target="_blank"
          rel="noreferrer"
        >
          API ↗
        </a>
      </header>
      <aside className="collections">
        <div className="sidebar-title">
          COLLECTIONS <span>{availableTypes.length}</span>
        </div>
        <input
          className="search"
          aria-label="Search collections"
          placeholder="Search collections…"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
        />
        <nav aria-label="Collections">
          {availableTypes
            .filter((v) => v.name.toLowerCase().includes(search.toLowerCase()))
            .map((type) => (
              <button
                key={type.id}
                className={
                  type.id === choice.item ? "collection selected" : "collection"
                }
                onClick={() => selectedType(type.id)}
              >
                <span className="collection-icon">{type.name.slice(0, 1)}</span>
                <span>{type.name}</span>
                <span className="chevron">›</span>
              </button>
            ))}
        </nav>
        <div className="sidebar-foot">
          <span className="small-dot" />
          Independent markets, shared books
        </div>
      </aside>
      <main>
        <div className="market-title">
          <div>
            <div className="eyebrow">MARKET EXPLORER</div>
            <h1>{item?.name || "Select a collection"}</h1>
            <span className="market-subtitle">
              {choice.venue
                ? names.get(choice.venue) || choice.venue
                : "All venues"}{" "}
              <span>·</span> {choice.currency || "Choose a currency"}
            </span>
          </div>
          <button
            className="refresh"
            aria-label="Refresh market"
            onClick={() => {
              setAnchors([""]);
              setRefresh((v) => v + 1);
            }}
          >
            ↻
          </button>
        </div>
        {(startupError || error) && (
          <div role="alert" className="notice">
            {startupError || error}
            <button
              onClick={() => {
                setAnchors([""]);
                setRefresh((v) => v + 1);
              }}
            >
              Reload
            </button>
          </div>
        )}
        <div className="controls">
          <div className="segments" role="group" aria-label="Market view">
            {(["ask", "bid", "executed"] as const).map((mode) => (
              <button
                key={mode}
                aria-pressed={choice.mode === mode}
                className={choice.mode === mode ? "active" : ""}
                onClick={() =>
                  set({
                    ...choice,
                    mode,
                    sort: mode === "bid" ? "price_desc" : "price_asc",
                    filters: mode === "bid" ? {} : choice.filters,
                  })
                }
              >
                {mode === "ask"
                  ? "Offers"
                  : mode === "bid"
                    ? "Bids"
                    : "Executions"}
              </button>
            ))}
          </div>
          <label>
            Currency
            <select
              aria-label="Currency"
              value={choice.currency}
              onChange={(e) =>
                set({ ...choice, currency: e.target.value, venue: "" })
              }
            >
              {!currencies.includes(choice.currency) && (
                <option value={choice.currency}>
                  {choice.currency || "Unavailable"}
                </option>
              )}
              {currencies.map((v) => (
                <option key={v}>{v}</option>
              ))}
            </select>
          </label>
          <label>
            Venue
            <select
              aria-label="Venue"
              value={choice.venue}
              onChange={(e) => set({ ...choice, venue: e.target.value })}
            >
              <option value="">All venues</option>
              {venueIds.map((v) => (
                <option key={v} value={v}>
                  {names.get(v) || v}
                </option>
              ))}
            </select>
          </label>
          <button
            className={`filter-button ${filterCount ? "applied" : ""}`}
            aria-expanded={showFilters}
            onClick={() => setShowFilters((v) => !v)}
          >
            ☷ Filters {filterCount > 0 && <b>{filterCount}</b>}
          </button>
        </div>
        {showFilters && (
          <form
            className="filters"
            onSubmit={(e) => {
              e.preventDefault();
              set(draft);
            }}
          >
            <div className="filter-heading">
              <strong>Refine this market</strong>
              <button
                type="button"
                className="text-button"
                onClick={() =>
                  setDraft({
                    ...draft,
                    filters: {},
                    min: "",
                    max: "",
                    from: "",
                    until: "",
                  })
                }
              >
                Clear
              </button>
            </div>
            <div className="filter-grid">
              {choice.mode !== "bid" &&
                Object.entries(item?.attributes || {}).map(([id, rule]) => (
                  <Field
                    key={id}
                    id={id}
                    rule={rule}
                    attributes={attributes}
                    options={options}
                    value={draft.filters[id]}
                    change={(v) =>
                      setDraft((old) => {
                        const filters = { ...old.filters };
                        if (v) filters[id] = v;
                        else delete filters[id];
                        return { ...old, filters };
                      })
                    }
                  />
                ))}
              <label>
                Price ({choice.currency})
                <span className="range">
                  <input
                    aria-label="Minimum price"
                    inputMode="decimal"
                    placeholder="Min"
                    value={draft.min}
                    onChange={(e) =>
                      setDraft({ ...draft, min: e.target.value })
                    }
                  />
                  <input
                    aria-label="Maximum price"
                    inputMode="decimal"
                    placeholder="Max"
                    value={draft.max}
                    onChange={(e) =>
                      setDraft({ ...draft, max: e.target.value })
                    }
                  />
                </span>
              </label>
              {choice.mode === "executed" && (
                <>
                  <label>
                    Executed from
                    <input
                      aria-label="Executed from"
                      type="datetime-local"
                      step="1"
                      value={dateInput(draft.from)}
                      onChange={(e) =>
                        setDraft({ ...draft, from: timestamp(e.target.value) })
                      }
                    />
                  </label>
                  <label>
                    Executed before
                    <input
                      aria-label="Executed before"
                      type="datetime-local"
                      step="1"
                      value={dateInput(draft.until)}
                      onChange={(e) =>
                        setDraft({ ...draft, until: timestamp(e.target.value) })
                      }
                    />
                  </label>
                </>
              )}
            </div>
            <button className="primary" type="submit">
              Apply filters →
            </button>
          </form>
        )}
        <section className="orders">
          <div className="table-toolbar">
            <div>
              <span
                className={`status ${status === "Live" ? "live" : ""}`}
                title={
                  page.sync?.updated_at
                    ? `Last synchronized ${new Date(page.sync.updated_at).toLocaleString()}`
                    : undefined
                }
              >
                <i />
                {status}
              </span>
              <span className="rows">{page.data.length} on this page</span>
            </div>
            <div className="table-actions">
              {choice.mode !== "executed" && (
                <>
                  <select
                    aria-label="Price order"
                    value={choice.sort}
                    onChange={(e) =>
                      set({ ...choice, sort: e.target.value as Choice["sort"] })
                    }
                  >
                    <option value="price_asc">Price: low to high</option>
                    <option value="price_desc">Price: high to low</option>
                  </select>
                  <button onClick={() => setPaused((v) => !v)}>
                    {paused ? "▶ Resume" : "Ⅱ Pause"}
                  </button>
                </>
              )}
            </div>
          </div>
          <div className="table-scroll">
            <table>
              <thead>
                <tr>
                  <th>Item</th>
                  <th>Attributes</th>
                  <th>Venue</th>
                  <th>Quantity</th>
                  <th className="price-col">
                    Price <small>{choice.currency}</small>
                  </th>
                  <th>
                    {choice.mode === "executed" ? "Executed" : "Observed"}
                  </th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {page.data.map((order) => (
                  <tr
                    key={JSON.stringify([order.book.venue_id, order.id])}
                    className={
                      selected?.id === order.id &&
                      selected.book.venue_id === order.book.venue_id
                        ? "selected"
                        : ""
                    }
                    onClick={() => setSelected(order)}
                  >
                    <td>
                      <div className="item-cell">
                        <span className="item-icon">
                          {item?.name[0] || "◈"}
                        </span>
                        <span>
                          <strong>
                            {item?.name ||
                              names.get(order.book.item_type_id) ||
                              order.book.item_type_id}
                          </strong>
                          <small className="item-id">
                            {order.item_id || "Units"}
                          </small>
                          <small className="mobile-venue">
                            {names.get(order.book.venue_id) ||
                              order.book.venue_id}
                          </small>
                        </span>
                      </div>
                    </td>
                    <td className="attributes-cell">
                      {Object.entries(order.attributes)
                        .slice(0, 4)
                        .map(([id, p]) => (
                          <span className="attribute" key={id}>
                            <small>{names.get(id) || id}</small>{" "}
                            {p.op === "eq"
                              ? text(p.value, names)
                              : p.op === "range"
                                ? `${p.min ? text(p.min, names) : "Any"} – ${p.max ? text(p.max, names) : "Any"}`
                                : p.op === "one_of"
                                  ? p.values
                                      .map((v) => text(v, names))
                                      .join(" / ")
                                  : "Target conditions"}
                          </span>
                        ))}
                    </td>
                    <td>
                      {names.get(order.book.venue_id) || order.book.venue_id}
                    </td>
                    <td className="mono">{order.quantity}</td>
                    <td
                      className={`price-col price ${choice.mode === "bid" ? "bid" : ""}`}
                    >
                      {amount(order.price)}
                      <small>{order.book.currency_id}</small>
                    </td>
                    <td
                      className="time"
                      title={order.executed_at || order.observed_at}
                    >
                      {new Date(
                        order.executed_at || order.observed_at,
                      ).toLocaleTimeString([], {
                        hour: "2-digit",
                        minute: "2-digit",
                        second: "2-digit",
                      })}
                    </td>
                    <td>
                      <button
                        className="row-open"
                        aria-label={`Inspect ${order.book.venue_id} ${order.id}`}
                        onClick={() => setSelected(order)}
                      >
                        ↗
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
            {!page.data.length && (
              <div className="empty">
                <span>◇</span>
                <strong>
                  {status === "Loading" || status === "Connecting"
                    ? "Loading this market…"
                    : choice.mode === "executed"
                      ? "No confirmed executions"
                      : "No matching orders"}
                </strong>
                <p>
                  {filterCount
                    ? "Adjust the filters to see more results."
                    : "Orders will appear here as venues publish them."}
                </p>
              </div>
            )}
          </div>
          <div className="pagination">
            <span>Page {anchors.length}</span>
            <label>
              Rows{" "}
              <select
                aria-label="Rows per page"
                value={pageSize}
                onChange={(e) => {
                  setPageSize(Number(e.target.value));
                  setAnchors([""]);
                }}
              >
                {[25, 50, 100, 200].map((v) => (
                  <option key={v}>{v}</option>
                ))}
              </select>
            </label>
            <button
              disabled={anchors.length === 1}
              onClick={() => setAnchors((v) => v.slice(0, -1))}
            >
              ← Previous
            </button>
            <button
              disabled={!page.next_page_token}
              onClick={() => {
                setAnchors((v) => [...v, page.next_page_token]);
                setSelected(undefined);
              }}
            >
              Next →
            </button>
          </div>
        </section>
        <footer>
          Quotes stay in their original currency. Executions require source
          confirmation.
        </footer>
      </main>
      {selected && (
        <aside className="detail" role="dialog" aria-label="Order details">
          <div className="detail-heading">
            <div className="eyebrow">ORDER DETAILS</div>
            <button
              aria-label="Close details"
              onClick={() => setSelected(undefined)}
            >
              ×
            </button>
          </div>
          <h2>
            {names.get(selected.book.item_type_id) ||
              selected.book.item_type_id}
          </h2>
          <div className="detail-price">
            {amount(selected.price)} <small>{selected.book.currency_id}</small>
          </div>
          <dl>
            {[
              ["Order ID", selected.id],
              ["Item ID", selected.item_id || "—"],
              [
                "Venue",
                names.get(selected.book.venue_id) || selected.book.venue_id,
              ],
              ["Venue ID", selected.book.venue_id],
              ["Item type ID", selected.book.item_type_id],
              ["Currency", selected.book.currency_id],
              ["Mode", selected.mode],
              ["Quantity", selected.quantity],
              ["Observed", new Date(selected.observed_at).toLocaleString()],
              ...(selected.executed_at
                ? [
                    [
                      "Executed",
                      new Date(selected.executed_at).toLocaleString(),
                    ],
                  ]
                : []),
              ...(selected.parent_order_id
                ? [["Parent order", selected.parent_order_id]]
                : []),
            ].map(([k, v]) => (
              <div key={k}>
                <dt>{k}</dt>
                <dd>{v}</dd>
              </div>
            ))}
          </dl>
          <h3>Attributes</h3>
          <dl>
            {Object.entries(selected.attributes).map(([id, p]) => (
              <div key={id}>
                <dt>{names.get(id) || id}</dt>
                <dd>
                  {p.op === "eq" ? text(p.value, names) : describe(p, names)}
                </dd>
              </div>
            ))}
          </dl>
          {Object.keys(selected.meta).length > 0 && (
            <>
              <h3>Source details</h3>
              <dl>
                {Object.entries(selected.meta).map(([k, v]) => (
                  <div key={k}>
                    <dt>{k}</dt>
                    <dd>{v}</dd>
                  </div>
                ))}
              </dl>
            </>
          )}
        </aside>
      )}
    </div>
  );
}
function describe(p: Predicate, names: Map<string, string>): string {
  switch (p.op) {
    case "eq":
      return text(p.value, names);
    case "one_of":
      return p.values.map((v) => text(v, names)).join(" / ");
    case "range":
      return `${p.min ? text(p.min, names) : "Any"} – ${p.max ? text(p.max, names) : "Any"}`;
    case "object":
      return Object.entries(p.fields || {})
        .map(([k, v]) => `${names.get(k) || k}: ${describe(v, names)}`)
        .join(" · ");
    case "contains":
      return `Contains ${p.min_count ?? 1}${p.max_count == null ? "+" : "–" + p.max_count}: ${describe(p.predicate, names)}`;
    case "all_of":
      return p.predicates.map((v) => describe(v, names)).join("; ");
  }
}
void initializeTelegram().then((context) =>
  createRoot(document.getElementById("root")!).render(
    <App context={context} />,
  ),
);
