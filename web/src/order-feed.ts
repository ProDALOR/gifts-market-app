import { ApiFailure, request } from "./market.ts";
import type { Order, Page, FeedQuery } from "./market.ts";
export type FeedState = {
  page: Page<Order>;
  status: string;
  error: string;
  loadingMore: boolean;
  hasMore: boolean;
};
export const emptyFeed = (): FeedState => ({
  page: {
    data: [],
    next_page_token: "",
    cursor: "",
    sync: { ready: false, state: "starting", updated_at: null },
  },
  status: "Connecting",
  error: "",
  loadingMore: false,
  hasMore: false,
});
export function mergePages(
  pages: Page<Order>[],
  ended?: (count: number) => void,
): Page<Order> {
  const seen = new Set<string>();
  const data: Order[] = [];
  for (const page of pages) {
    for (const order of page.data) {
      const key = JSON.stringify([order.book.venue_id, order.id]);
      if (!seen.has(key)) {
        seen.add(key);
        data.push(order);
      }
    }
    ended?.(data.length);
  }
  return {
    data,
    next_page_token: pages.at(-1)?.next_page_token || "",
    cursor: pages.at(-1)?.cursor || "",
    sync: pages[0]?.sync || emptyFeed().page.sync,
  };
}
/** One live head plus lazily refreshed keyset pages. Freshness does not invalidate
 * data. Offscreen pages retain scroll history and refresh when they enter view. */
export class OrderFeed {
  private stop = new AbortController();
  private socket?: WebSocket;
  private reconnect?: ReturnType<typeof setTimeout>;
  private head?: Page<Order>;
  private revision?: string;
  private epoch = 0;
  private versions: number[] = [];
  private ends: number[] = [];
  private visible = { start: 0, end: Number.POSITIVE_INFINITY };
  private pending?: AbortController;
  private renderedHead?: Page<Order>;
  private pages: Page<Order>[] = [];
  private wanted = 1;
  private working = false;
  private dirty = false;
  private state = emptyFeed();
  constructor(
    private selection: FeedQuery | null,
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
    if (!this.selection) {
      this.emit({ status: "Choose a collection" });
      return;
    }
    if (this.selection.kind === "history" || this.paused) {
      this.emit({ status: "Loading" });
      void this.read("")
        .then((page) => this.accept(page))
        .catch((error) => this.fail(error));
    } else this.connect();
  }
  close() {
    this.stop.abort();
    this.pending?.abort();
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
  setVisibleRange(start: number, end: number) {
    if (this.visible.start === start && this.visible.end === end) return;
    this.visible = { start, end };
    if (this.stalePages().size) this.reconcile();
  }
  private stalePages() {
    const result = new Set<number>();
    for (let index = 1; index < this.pages.length; index++) {
      if (
        (this.versions[index] ?? -1) < this.epoch &&
        (this.ends[index - 1] || 0) < this.visible.end &&
        this.ends[index] > this.visible.start
      )
        result.add(index);
    }
    return result;
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
        JSON.stringify({
          type: "subscribe",
          id: "offers",
          query: this.selection?.query,
        }),
      );
    };
    socket.onmessage = (event) => {
      if (this.stop.signal.aborted || this.socket !== socket) return;
      try {
        const frame = JSON.parse(event.data);
        if (frame.type === "page")
          this.accept(
            { ...frame.page, sync: frame.sync || frame.page.sync },
            frame.revision,
          );
        else if (frame.type === "sync" && this.head) {
          this.accept({ ...this.head, sync: frame.sync }, frame.revision);
        } else if (frame.type === "error") {
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
  private read(token: string, signal = this.stop.signal): Promise<Page<Order>> {
    const selection = this.selection;
    if (!selection) throw new Error("Choose a collection");
    return selection.kind === "history"
      ? request(
          "/history/query",
          { ...selection.query, page_token: token },
          signal,
        )
      : request(
          "/orders/query",
          {
            ...selection.query,
            page_token: token,
            consistency: this.paused ? "snapshot" : "live",
          },
          signal,
        );
  }
  private accept(head: Page<Order>, revision?: string) {
    if (this.stop.signal.aborted) return;
    const same =
      this.head &&
      (revision === undefined || revision === this.revision) &&
      head.next_page_token === this.head.next_page_token &&
      (head.data === this.head.data ||
        JSON.stringify(head.data) === JSON.stringify(this.head.data));
    const boundaryChanged = this.head?.next_page_token !== head.next_page_token;
    this.head = head;
    this.revision = revision;
    if (same) {
      if (this.pages.length)
        this.pages[0] = { ...this.pages[0], sync: head.sync };
      this.emit({
        page: { ...this.state.page, sync: head.sync },
        status: this.feedStatus(),
        error: "",
      });
      return;
    }
    this.epoch++;
    // Tail-only updates coalesce behind the in-flight refresh. Repeated updates
    // must not starve a slow viewport read. Cancel when its head anchor moved.
    if (
      boundaryChanged &&
      this.visible.start <
        (this.pages[0]?.data.length || 0) + (this.pages[1]?.data.length || 0)
    )
      this.pending?.abort();
    if (this.pages.length) {
      this.pages[0] = head;
      this.versions[0] = this.epoch;
      this.publish();
    }
    this.reconcile();
  }
  private feedStatus() {
    return this.paused
      ? "Paused"
      : this.selection?.kind === "history"
        ? "Snapshot"
        : this.head?.sync.state === "live"
          ? "Live"
          : "Stale";
  }
  private publish() {
    if (!this.head || !this.pages.length) return;
    this.ends = [];
    const merged = mergePages(this.pages, (count) => this.ends.push(count));
    // Retain identities so memoized cards do not re-render for unchanged orders.
    const previous = new Map(
      this.state.page.data.map((order) => [
        JSON.stringify([order.book.venue_id, order.id]),
        order,
      ]),
    );
    merged.data = merged.data.map((order) => {
      const old = previous.get(JSON.stringify([order.book.venue_id, order.id]));
      return old &&
        (old === order || JSON.stringify(old) === JSON.stringify(order))
        ? old
        : order;
    });
    merged.sync = this.head.sync;
    this.emit({
      page: merged,
      hasMore: !!merged.next_page_token,
      error: "",
      status: this.feedStatus(),
    });
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
      const epoch = this.epoch;
      const pass = new AbortController();
      this.pending = pass;
      const signal = AbortSignal.any([this.stop.signal, pass.signal]);
      const pages = this.pages.slice();
      const versions = this.versions.slice();
      pages[0] = head;
      versions[0] = epoch;
      const tokens = new Set<string>();
      const stale = this.stalePages();
      try {
        // Refresh only pages intersecting the viewport. A keyset boundary remains
        // valid when its order disappears, preserving the user's scroll position.
        for (
          let index = 1;
          index < Math.max(this.wanted, pages.length);
          index++
        ) {
          if (!pages[index - 1]?.next_page_token) {
            pages.length = index;
            versions.length = index;
            break;
          }
          const token = pages[index - 1].next_page_token;
          if (tokens.has(token))
            throw new Error("Market returned a repeated continuation");
          tokens.add(token);
          if (pages[index] && !stale.has(index)) continue;
          pages[index] = await this.read(token, signal);
          versions[index] = epoch;
        }
      } catch (error) {
        if (signal.aborted) {
          if (this.stop.signal.aborted) return;
          continue;
        }
        if (
          error instanceof ApiFailure &&
          error.code === "restart_read" &&
          !restarted
        ) {
          restarted = true;
          this.head = await this.read("");
          this.pages = [];
          this.versions = [];
          this.epoch++;
          this.dirty = true;
          continue;
        }
        throw error;
      }
      if (signal.aborted) continue;
      pages[0] = this.head;
      versions[0] = this.epoch;
      this.pages = pages;
      this.versions = versions;
      this.renderedHead = head;
      this.publish();
    }
  }
}
