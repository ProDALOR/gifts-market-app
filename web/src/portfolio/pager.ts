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
/** Only explicit scroll demand grows coverage. A changed version replaces it atomically. */
export class PortfolioPager {
  private rows: PortfolioRows;
  private version?: string;
  private request?: AbortController;
  private intent: "reset" | "more" = "reset";
  private waiting?: string;
  private paused = false;
  private disposed = false;
  constructor(private options: Options) {
    this.rows = options.initial ?? emptyRows();
  }
  current() {
    return this.rows;
  }
  private publish(patch: Partial<PortfolioRows>) {
    this.rows = { ...this.rows, ...patch };
    this.options.changed(this.rows);
  }
  update(version: string) {
    if (this.disposed || this.waiting === version) return;
    this.paused = false;
    this.waiting = undefined;
    if (version !== this.version) {
      this.request?.abort();
      this.request = undefined;
      this.version = version;
      if (this.rows.version === version && this.rows.selection) return;
      this.intent = "reset";
      void this.run();
    } else if (
      !this.request &&
      (this.rows.loading || this.rows.error || !this.rows.selection)
    ) {
      void this.run();
    }
  }
  loadMore = () => {
    if (
      this.request ||
      this.waiting ||
      this.paused ||
      !this.rows.hasMore ||
      this.rows.error
    )
      return;
    this.intent = "more";
    void this.run();
  };
  retry = () => {
    if (this.waiting) this.options.refreshSummary();
    else void this.run();
  };
  pause() {
    this.paused = true;
    this.request?.abort();
    this.request = undefined;
    // Keep pending intent. A fresh summary after resume authorizes its retry.
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
    const stop = new AbortController();
    this.request = stop;
    const version = this.version;
    const reset = this.intent === "reset";
    const current = () =>
      !this.disposed &&
      !stop.signal.aborted &&
      this.request === stop &&
      this.version === version;
    const read = async (offset: number) => {
      const page = await this.options.request(
        { ...this.options.query, offset, limit: 100, version },
        stop.signal,
      );
      stop.signal.throwIfAborted();
      if (
        page.version !== version ||
        page.offset !== offset ||
        page.items.length > 100 ||
        page.selection.count < offset + page.items.length ||
        page.has_more !== offset + page.items.length < page.selection.count ||
        (page.has_more && page.items.length !== 100)
      )
        throw new Error("Portfolio page did not match the requested coverage");
      return page;
    };
    this.publish({ loading: true, error: "" });
    try {
      const offset = reset ? 0 : this.rows.items.length;
      const first = await read(offset);
      const pages = [first];
      if (reset) {
        const target = Math.min(
          Math.max(100, this.rows.items.length),
          first.selection.count,
        );
        // Refresh only visited coverage, with at most two page requests in flight.
        for (let start = 100; start < target; start += 200) {
          const offsets = [start, start + 100].filter(
            (offset) => offset < target,
          );
          pages.push(...(await Promise.all(offsets.map(read))));
        }
      } else if (this.rows.selection?.count !== first.selection.count) {
        throw new Error("Portfolio count changed within one version");
      }
      if (!current()) return;
      if (pages.some((page) => page.selection.count !== first.selection.count))
        throw new Error("Portfolio count changed within one version");
      const items = [
        ...(reset ? [] : this.rows.items),
        ...pages.flatMap((page) => page.items),
      ];
      if (new Set(items.map((item) => item.gift.gift.id)).size !== items.length)
        throw new Error("Portfolio pages contained duplicate gifts");
      this.publish({
        items,
        selection: first.selection,
        version,
        hasMore: pages[pages.length - 1].has_more,
        loading: false,
        error: "",
      });
    } catch (error) {
      if (!current()) return;
      stop.abort();
      if ((error as { code?: string }).code === "restart_read") {
        this.waiting = version;
        this.intent = "reset";
        this.publish({ loading: true, error: "" });
        this.options.refreshSummary();
      } else {
        this.publish({
          loading: false,
          error:
            error instanceof Error
              ? error.message
              : "Could not load portfolio gifts",
        });
      }
    } finally {
      if (this.request === stop) this.request = undefined;
    }
  }
}
