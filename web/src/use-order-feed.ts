import { useCallback, useLayoutEffect, useRef, useState } from "react";
import { emptyFeed, createOrderFeed } from "./order-feed";
import type { FeedController, Bookmark } from "./order-feed";
import type { FeedQuery } from "./market";
export function useOrderFeed(
  query: FeedQuery | null,
  paused: boolean,
  refresh: number,
) {
  const signature = JSON.stringify(query);
  const [view, setView] = useState(() => ({ signature, state: emptyFeed() }));
  const feed = useRef<FeedController | undefined>(undefined);
  const saved = useRef<{ signature: string; bookmark?: Bookmark }>({
    signature: "",
  });
  const frozen = query?.kind === "orders" && paused;
  useLayoutEffect(() => {
    const same = saved.current.signature === signature;
    if (!same) saved.current = { signature };
    if (!query) return;
    const current = createOrderFeed(
      query,
      frozen,
      (next) => {
        const ready = current.ready;
        setView((previous) => {
          // A queued update from a retired collection must not replace the current view.
          if (feed.current !== current) return previous;
          const prior =
            previous.signature === signature ? previous.state : emptyFeed();
          return {
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
        });
      },
      same ? saved.current.bookmark : undefined,
    );
    feed.current = current;
    current.start();
    return () => {
      saved.current = { signature, bookmark: current.bookmark };
      current.close();
      if (feed.current === current) feed.current = undefined;
    };
  }, [signature, frozen, refresh]);
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
