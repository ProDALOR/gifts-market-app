import type { LoadedOrders } from "./loaded-orders.ts";
import type { Order, SyncStatus } from "../market.ts";
import type { OrderActivity } from "../orderbook/order-activity.ts";
export type FeedState = {
  orders: Order[];
  displayOrders: Order[];
  activity: ReadonlyMap<string, OrderActivity>;
  sync: SyncStatus;
  status: string;
  error: string;
  loadingMore: boolean;
  hasMore: boolean;
};
export type Bookmark =
  | { kind: "range"; anchor?: string; cache?: LoadedOrders }
  | { kind: "history"; depth: number };
export interface FeedController {
  start(): void;
  close(): void;
  loadMore(): void;
  setVisibleRange(start: number, end: number, anchor?: number): void;
  readonly ready: boolean;
  readonly bookmark: Bookmark;
}
export const emptyFeed = (): FeedState => ({
  orders: [],
  displayOrders: [],
  activity: new Map(),
  sync: { ready: false, state: "starting", updated_at: null },
  status: "Connecting",
  error: "",
  loadingMore: false,
  hasMore: false,
});
