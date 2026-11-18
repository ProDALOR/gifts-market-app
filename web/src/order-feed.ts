import type { FeedQuery } from "./market.ts";
import { WindowController } from "./feed/window-controller.ts";
import { HistoryController } from "./feed/history-controller.ts";
import type { FeedController, FeedState, Bookmark } from "./feed/types.ts";
export { emptyFeed } from "./feed/types.ts";
export type { FeedController, FeedState, Bookmark } from "./feed/types.ts";
export function createOrderFeed(
  query: FeedQuery,
  paused: boolean,
  changed: (state: FeedState) => void,
  bookmark?: Bookmark,
): FeedController {
  return query.kind === "history"
    ? new HistoryController(query.query, changed, bookmark)
    : new WindowController(query.query, paused, changed, bookmark);
}
