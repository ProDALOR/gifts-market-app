import type { components } from "../gen/api";
export type PortfolioPage = components["schemas"]["PortfolioPage"];
export type PortfolioReadQuery = components["schemas"]["PortfolioReadQuery"];
export type PortfolioRows = {
  items: PortfolioPage["items"];
  selection?: PortfolioPage["selection"];
  version?: string;
  hasMore: boolean;
  loading: boolean;
  error: string;
};
export const emptyRows = (): PortfolioRows => ({
  items: [],
  hasMore: false,
  loading: false,
  error: "",
});
type Query = Omit<PortfolioReadQuery, "offset" | "limit" | "version">;
type Options = {
  query: Query;
  request: (
    query: PortfolioReadQuery,
    signal: AbortSignal,
  ) => Promise<PortfolioPage>;
  changed: (rows: PortfolioRows) => void;
  refreshSummary: () => void;
  initial?: PortfolioRows;
};
const PAGE_SIZE = 100;
/** Desired coverage survives cancellation and version changes. Rows commit atomically. */
export class PortfolioPager {
  private rows: PortfolioRows;
  private version?: string;
  private request?: AbortController;
  private waiting?: string;
  private paused = false;
  private disposed = false;
  private desired: number;
  private ids: Set<string>;
  constructor(private options: Options) {
    this.rows = options.initial ?? emptyRows();
    this.desired = Math.max(PAGE_SIZE, this.rows.items.length);
    this.ids = new Set(this.rows.items.map((item) => item.gift.gift.id));
  }
  current() {
    return this.rows;
  }
  private publish(patch: Partial<PortfolioRows>) {
    this.rows = { ...this.rows, ...patch };
    this.options.changed(this.rows);
  }
  update(version: string) {
    if (this.disposed) return;
    const resuming = this.paused;
    this.paused = false;
    if (this.waiting === version) return;
    this.waiting = undefined;
    const changed = version !== this.version;
    if (changed) {
      this.request?.abort();
      this.request = undefined;
      this.version = version;
    }
    if (changed || resuming || !this.rows.error) void this.run();
  }
  loadMore = () => {
    if (
      this.disposed ||
      this.request ||
      this.waiting ||
      this.paused ||
      !this.version ||
      !this.rows.hasMore ||
      this.rows.error
    )
      return;
    this.desired = Math.max(this.desired, this.rows.items.length + PAGE_SIZE);
    void this.run();
  };
  retry = () => {
    if (this.disposed || this.paused || this.request) return;
    if (this.waiting) this.options.refreshSummary();
    else void this.run();
  };
  pause() {
    this.paused = true;
    this.request?.abort();
    this.request = undefined;
  }
  dispose() {
    this.disposed = true;
    this.request?.abort();
    this.request = undefined;
  }
  private async run() {
    if (
      this.disposed ||
      this.paused ||
      this.waiting ||
      this.request ||
      !this.version
    )
      return;
    const version = this.version;
    const reset = this.rows.version !== version || !this.rows.selection;
    if (
      !reset &&
      this.rows.items.length >=
        Math.min(this.desired, this.rows.selection!.count)
    ) {
      if (this.rows.loading || this.rows.error)
        this.publish({ loading: false, error: "" });
      return;
    }
    const stop = new AbortController();
    this.request = stop;
    const current = () =>
      !this.disposed &&
      !stop.signal.aborted &&
      this.request === stop &&
      this.version === version;
    const read = async (offset: number) => {
      const page = await this.options.request(
        { ...this.options.query, offset, limit: PAGE_SIZE, version },
        stop.signal,
      );
      stop.signal.throwIfAborted();
      if (
        page.version !== version ||
        page.offset !== offset ||
        page.items.length > PAGE_SIZE ||
        page.selection.count < offset + page.items.length ||
        page.has_more !== offset + page.items.length < page.selection.count ||
        (page.has_more && page.items.length !== PAGE_SIZE)
      )
        throw new Error("Portfolio page did not match the requested coverage");
      return page;
    };
    this.publish({ loading: true, error: "" });
    try {
      const first = await read(reset ? 0 : this.rows.items.length);
      const pages = [first];
      if (reset) {
        const target = Math.min(this.desired, first.selection.count);
        for (let start = PAGE_SIZE; start < target; start += PAGE_SIZE * 2) {
          const offsets = [start, start + PAGE_SIZE].filter(
            (offset) => offset < target,
          );
          pages.push(...(await Promise.all(offsets.map(read))));
        }
      } else if (this.rows.selection?.count !== first.selection.count) {
        throw new Error("Portfolio count changed within one version");
      }
      if (!current()) return;
      const additions: PortfolioPage["items"] = [];
      const addedIds = new Set<string>();
      for (const page of pages) {
        if (page.selection.count !== first.selection.count)
          throw new Error("Portfolio count changed within one version");
        for (const item of page.items) {
          const id = item.gift.gift.id;
          if (addedIds.has(id) || (!reset && this.ids.has(id)))
            throw new Error("Portfolio pages contained duplicate gifts");
          addedIds.add(id);
          additions.push(item);
        }
      }
      if (reset) this.ids = addedIds;
      else for (const id of addedIds) this.ids.add(id);
      this.publish({
        items: reset ? additions : this.rows.items.concat(additions),
        selection: reset ? first.selection : this.rows.selection,
        version,
        hasMore: pages[pages.length - 1].has_more,
        loading: false,
        error: "",
      });
    } catch (error) {
      if (!current()) return;
      stop.abort();
      if (
        error &&
        typeof error === "object" &&
        (error as { code?: string }).code === "restart_read"
      ) {
        this.waiting = version;
        this.publish({ loading: true, error: "" });
        this.options.refreshSummary();
      } else
        this.publish({
          loading: false,
          error:
            error instanceof Error
              ? error.message
              : "Could not load portfolio gifts",
        });
    } finally {
      if (this.request === stop) this.request = undefined;
    }
  }
}
