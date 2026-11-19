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
  return paused
    ? new SnapshotController({ ...query, limit: 100 }, changed, bookmark)
    : new RangeController(query, changed, bookmark);
}
