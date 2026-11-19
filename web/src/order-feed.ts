import type { FeedQuery } from "./market.ts";
import { RangeController } from "./feed/range-controller.ts";
import { SnapshotController } from "./feed/snapshot-controller.ts";
import type { FeedController, FeedState, Bookmark } from "./feed/types.ts";
export { emptyFeed } from "./feed/types.ts";
export type { FeedController, FeedState, Bookmark } from "./feed/types.ts";
export function createOrderFeed(
  query: FeedQuery,
  paused: boolean,
  changed: (state: FeedState) => void,
  bookmark?: Bookmark,
): FeedController {
  if (query.kind === "history")
    return new SnapshotController(
      { ...query.query, source: "history" },
      changed,
      bookmark,
    );
  if (paused) {
    const { side, ...params } = query.query;
    return new SnapshotController(
      { ...params, source: side === "bid" ? "bids" : "asks", limit: 100 },
      changed,
      bookmark,
    );
  }
  return new RangeController(query.query, changed, bookmark);
}
