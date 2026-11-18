import {
  request,
  ApiFailure,
  type HistoryQuery,
  type OrderPage,
} from "../market.ts";
import { orderKey } from "../orderbook/order-activity.ts";
import {
  emptyFeed,
  type Bookmark,
  type FeedController,
  type FeedState,
} from "./types.ts";
/** Forward pagination belongs only to immutable execution-history snapshots. */
export class HistoryController implements FeedController {
  private stop = new AbortController();
  private state = emptyFeed();
  private pages: OrderPage[] = [];
  private working = false;
  private wanted: number;
  ready = false;
  constructor(
    private query: HistoryQuery,
    private changed: (state: FeedState) => void,
    bookmark?: Bookmark,
  ) {
    this.wanted = bookmark?.kind === "history" ? bookmark.depth : 1;
  }
  get bookmark(): Bookmark {
    return { kind: "history", depth: this.wanted };
  }
  start() {
    void this.load();
  }
  close() {
    this.stop.abort();
  }
  setVisibleRange() {}
  loadMore() {
    if (!this.working && !this.state.error && this.state.hasMore) {
      this.wanted++;
      void this.load();
    }
  }
  private async load() {
    if (this.working) return;
    this.working = true;
    this.emit({
      loadingMore: true,
      status: this.ready ? "Snapshot" : "Loading",
    });
    let restarted = false;
    try {
      while (this.pages.length < this.wanted) {
        const token = this.pages.at(-1)?.next_page_token || "";
        if (this.pages.length && !token) break;
        try {
          const page = await request<OrderPage>(
            "/history/query",
            { ...this.query, page_token: token },
            this.stop.signal,
          );
          if (this.stop.signal.aborted) return;
          if (
            page.next_page_token &&
            (page.next_page_token === token ||
              this.pages.some(
                (p) => p.next_page_token === page.next_page_token,
              ))
          )
            throw new Error("Market returned a repeated continuation");
          this.pages.push(page);
        } catch (error) {
          if (
            error instanceof ApiFailure &&
            error.code === "restart_read" &&
            !restarted
          ) {
            restarted = true;
            this.pages = [];
            continue;
          }
          throw error;
        }
      }
      const seen = new Set<string>();
      const orders = this.pages.flatMap((page) =>
        page.data.filter((row) => {
          const key = orderKey(row);
          if (seen.has(key)) return false;
          seen.add(key);
          return true;
        }),
      );
      this.ready = true;
      this.emit({
        orders,
        displayOrders: orders,
        sync: this.pages[0]?.sync ?? this.state.sync,
        cursor: this.pages.at(-1)?.cursor ?? "",
        hasMore: !!this.pages.at(-1)?.next_page_token,
        status: "Snapshot",
        error: "",
      });
    } catch (error) {
      this.emit({
        error: error instanceof Error ? error.message : String(error),
        status: "Unavailable",
      });
    } finally {
      this.working = false;
      this.emit({ loadingMore: false });
    }
  }
  private emit(patch: Partial<FeedState>) {
    if (!this.stop.signal.aborted) {
      this.state = { ...this.state, ...patch };
      this.changed(this.state);
    }
  }
}
