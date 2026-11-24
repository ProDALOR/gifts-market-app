import { useCallback, useLayoutEffect, useRef, useState } from "react";
import { emptyFeed, createOrderFeed } from "./order-feed";
import type { FeedController, Bookmark } from "./order-feed";
import type { FeedQuery } from "./market";
export type RetainedFeed = {
  signature: string;
  state?: ReturnType<typeof emptyFeed>;
  bookmark?: Bookmark;
};
export function useOrderFeed(
  query: FeedQuery | null,
  paused: boolean,
  refresh: number,
  retained?: RetainedFeed,
) {
  const signature = JSON.stringify(query);
  const [view, setView] = useState(() => ({
    signature,
    state:
      retained?.signature === signature && retained.state
        ? retained.state
        : emptyFeed(),
  }));
  const feed = useRef<FeedController | undefined>(undefined);
  const saved = useRef<{ signature: string; bookmark?: Bookmark }>({
    signature: retained?.signature ?? "",
    bookmark: retained?.bookmark,
  });
  useLayoutEffect(() => {
    // Catalog discovery can briefly leave a restored page without a query.
    // Keep its bookmark until the actual selection is available.
    if (!query) return;
    const same = saved.current.signature === signature;
    if (!same) saved.current = { signature };
    const current = createOrderFeed(
      query,
      paused,
      (next) => {
        const ready = current.ready;
        setView((previous) => {
          // A queued update from a retired collection must not replace the current view.
          if (feed.current !== current) return previous;
          const prior =
            previous.signature === signature
              ? previous.state
              : retained?.signature === signature && retained.state
                ? retained.state
                : emptyFeed();
          const updated = {
            signature,
            state:
              same && !ready
                ? {
                    ...next,
                    orders: prior.orders,
                    displayOrders: prior.displayOrders,
                    activity: prior.activity,
                    hasMore: prior.hasMore,
                  }
                : next,
          };
          return updated;
        });
      },
      same ? saved.current.bookmark : undefined,
    );
    feed.current = current;
    current.start();
    return () => {
      saved.current = { signature, bookmark: current.bookmark };
      if (retained) {
        retained.signature = signature;
        retained.bookmark = current.bookmark;
      }
      current.close();
      if (feed.current === current) feed.current = undefined;
    };
  }, [signature, paused, refresh]);
  useLayoutEffect(() => {
    // Retain committed rows only; React can discard a speculative state update.
    if (query && retained && view.signature === signature) {
      retained.signature = signature;
      retained.state = view.state;
    }
  }, [view, signature, retained]);
  const loadMore = useCallback(() => {
    if (saved.current.signature === signature) feed.current?.loadMore();
  }, [signature]);
  const setVisibleRange = useCallback(
    (start: number, end: number, anchor?: number) => {
      if (saved.current.signature === signature)
        feed.current?.setVisibleRange(start, end, anchor);
    },
    [signature],
  );
  const state = !query
    ? { ...emptyFeed(), status: "Choose a collection" }
    : view.signature === signature
      ? view.state
      : emptyFeed();
  return { ...state, loadMore, setVisibleRange };
}
