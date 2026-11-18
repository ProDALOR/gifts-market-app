import { useCallback, useEffect, useRef, useState } from "react";
import { emptyFeed, OrderFeed } from "./order-feed";
import type { FeedQuery } from "./market";
export function useOrderFeed(
  query: FeedQuery | null,
  paused: boolean,
  refresh: number,
) {
  const signature = JSON.stringify(query);
  const [state, setState] = useState(emptyFeed);
  const feed = useRef<OrderFeed | undefined>(undefined);
  const window = useRef({ signature: "", depth: 1 });
  useEffect(() => {
    const same = window.current.signature === signature;
    if (!same) setState(emptyFeed());
    const current: OrderFeed = new OrderFeed(
      query,
      paused,
      (next) => {
        setState((previous) =>
          same && !current.ready
            ? {
                ...next,
                page: previous.page,
                displayOrders: previous.displayOrders,
                activity: previous.activity,
                hasMore: previous.hasMore,
                loadingMore: true,
              }
            : next,
        );
      },
      same ? window.current.depth : 1,
    );
    feed.current = current;
    current.start();
    return () => {
      window.current = { signature, depth: current.windowPages };
      current.close();
      if (feed.current === current) feed.current = undefined;
    };
  }, [signature, paused, refresh]);
  const loadMore = useCallback(() => feed.current?.loadMore(), []);
  const setVisibleRange = useCallback(
    (start: number, end: number) => feed.current?.setVisibleRange(start, end),
    [],
  );
  return { ...state, loadMore, setVisibleRange };
}
