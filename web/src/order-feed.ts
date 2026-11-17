import { ApiFailure, request } from "./market.ts";
import type { Order, Page, Query } from "./market.ts";
export type FeedState = {
  page: Page<Order>;
  status: string;
  error: string;
  loadingMore: boolean;
  hasMore: boolean;
};
export const emptyFeed = (): FeedState => ({
  page: { data: [], next_page_token: "", cursor: "" },
  status: "Connecting",
  error: "",
  loadingMore: false,
  hasMore: false,
});
export function mergePages(pages: Page<Order>[]): Page<Order> {
  const seen = new Set<string>();
  return {
    data: pages
      .flatMap((page) => page.data)
      .filter((order) => {
        const key = JSON.stringify([order.book.venue_id, order.id]);
        if (seen.has(key)) return false;
        seen.add(key);
        return true;
      }),
    next_page_token: pages.at(-1)?.next_page_token || "",
    cursor: pages.at(-1)?.cursor || "",
    sync: pages[0]?.sync,
  };
}
/** One live head subscription, snapshot continuations for the scroll window.
 * Live continuations do not retain server snapshots. Head changes reconcile
 * the entire visible window atomically; stale requests
 * cannot leak into another collection, currency or sort. */
export class OrderFeed {
  private stop = new AbortController();
  private socket?: WebSocket;
  private reconnect?: ReturnType<typeof setTimeout>;
  private head?: Page<Order>;
  private renderedHead?: Page<Order>;
  private pages: Page<Order>[] = [];
  private wanted = 1;
  private working = false;
  private dirty = false;
  private state = emptyFeed();
  constructor(
    private query: Query & {
      period?: { from: string | null; until: string | null };
    },
    private history: boolean,
    private paused: boolean,
    private changed: (state: FeedState) => void,
    initialPages = 1,
  ) {
    this.wanted = initialPages;
  }
  get windowPages() {
    return this.wanted;
  }
  get ready() {
    return this.renderedHead !== undefined;
  }
  start() {
    if (!this.query.scope.item_type_id || !this.query.scope.currency_id) {
      this.emit({ status: "Choose a collection" });
      return;
    }
    if (this.history || this.paused) {
      this.emit({ status: "Loading" });
      void this.read("")
        .then((page) => this.accept(page))
        .catch((error) => this.fail(error));
    } else this.connect();
  }
  close() {
    this.stop.abort();
    clearTimeout(this.reconnect);
    if (this.socket) {
      this.socket.onclose = null;
      this.socket.close();
    }
  }
  loadMore() {
    if (this.working || this.state.error || !this.pages.at(-1)?.next_page_token)
      return;
    this.wanted++;
    this.reconcile();
  }
  private emit(patch: Partial<FeedState>) {
    if (this.stop.signal.aborted) return;
    this.state = { ...this.state, ...patch };
    this.changed(this.state);
  }
  private fail(error: unknown) {
    if (!this.stop.signal.aborted)
      this.emit({
        error: error instanceof Error ? error.message : String(error),
        status: "Unavailable",
        loadingMore: false,
      });
  }
  private connect() {
    if (this.stop.signal.aborted) return;
    this.emit({ status: this.pages.length ? "Reconnecting" : "Connecting" });
    const socket = new WebSocket(
      `${location.protocol === "https:" ? "wss:" : "ws:"}//${location.host}/ws`,
    );
    this.socket = socket;
    socket.onopen = () => {
      if (this.stop.signal.aborted || this.socket !== socket) return;
      socket.send(
        JSON.stringify({ type: "subscribe", id: "offers", query: this.query }),
      );
    };
    socket.onmessage = (event) => {
      if (this.stop.signal.aborted || this.socket !== socket) return;
      try {
        const frame = JSON.parse(event.data);
        if (frame.type === "page") this.accept(frame.page);
        else if (frame.type === "error") {
          this.fail(new Error(frame.error.message));
          if (["unavailable", "capacity_exceeded"].includes(frame.error.code))
            socket.close();
        }
      } catch (error) {
        this.fail(error);
      }
    };
    socket.onerror = () => socket.close();
    socket.onclose = () => {
      if (!this.stop.signal.aborted && this.socket === socket) {
        this.emit({ status: "Reconnecting" });
        this.reconnect = setTimeout(() => this.connect(), 1000);
      }
    };
  }
  private read(token: string): Promise<Page<Order>> {
    const { consistency: _consistency, ...query } = this.query;
    return request(
      this.history ? "/executions/query" : "/orders/query",
      {
        ...query,
        page_token: token,
        ...(!this.history
          ? { consistency: this.paused ? "snapshot" : "live" }
          : {}),
      },
      this.stop.signal,
    );
  }
  private accept(head: Page<Order>) {
    if (this.stop.signal.aborted) return;
    this.head = head;
    this.emit({ error: "" });
    this.reconcile();
  }
  private reconcile() {
    this.dirty = true;
    if (this.working || !this.head || this.stop.signal.aborted) return;
    this.working = true;
    this.emit({ loadingMore: true });
    void this.run()
      .catch((error) => this.fail(error))
      .finally(() => {
        this.working = false;
        this.emit({ loadingMore: false });
      });
  }
  private async run() {
    let restarted = false;
    while (this.dirty && !this.stop.signal.aborted && this.head) {
      this.dirty = false;
      const head = this.head;
      const pages = head === this.renderedHead ? this.pages.slice() : [head];
      const tokens = new Set(
        pages.slice(0, -1).map((page) => page.next_page_token),
      );
      try {
        while (pages.length < this.wanted && pages.at(-1)?.next_page_token) {
          const token = pages.at(-1)!.next_page_token;
          if (tokens.has(token))
            throw new Error("Market returned a repeated continuation");
          tokens.add(token);
          pages.push(await this.read(token));
        }
      } catch (error) {
        if (
          error instanceof ApiFailure &&
          error.code === "restart_read" &&
          !restarted
        ) {
          restarted = true;
          this.head = await this.read("");
          this.renderedHead = undefined;
          this.dirty = true;
          continue;
        }
        throw error;
      }
      if (this.stop.signal.aborted) return;
      this.pages = pages;
      this.renderedHead = head;
      const page = mergePages(pages);
      this.emit({
        page,
        hasMore: !!page.next_page_token,
        error: "",
        status: this.paused
          ? "Paused"
          : this.history
            ? "Snapshot"
            : head.sync?.state === "live"
              ? "Live"
              : "Stale",
      });
    }
  }
}
