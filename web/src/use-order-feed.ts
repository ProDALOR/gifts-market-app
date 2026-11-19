import { useCallback, useEffect, useRef, useState } from "react";
import { emptyFeed, createOrderFeed } from "./order-feed";
import type { FeedController, Bookmark } from "./order-feed";
import type { FeedQuery } from "./market";
export function useOrderFeed(
  query: FeedQuery | null,
  paused: boolean,
  refresh: number,
) {
  const signature = JSON.stringify(query);
  const [state, setState] = useState(emptyFeed);
  const feed = useRef<FeedController | undefined>(undefined);
  const saved = useRef<{ signature: string; bookmark?: Bookmark }>({
    signature: "",
  });
  const frozen = query?.kind === "orders" && paused;
  useEffect(() => {
    const same = saved.current.signature === signature;
    if (!same) {
      saved.current = { signature };
      setState(emptyFeed());
    }
    if (!query) {
      setState({ ...emptyFeed(), status: "Choose a collection" });
      return;
    }
    const current = createOrderFeed(
      query,
      frozen,
      (next) => {
        setState((previous) =>
          same && !current.ready
            ? {
                ...next,
                orders: previous.orders,
                sync: previous.sync,
                displayOrders: previous.displayOrders,
                activity: previous.activity,
                hasMore: previous.hasMore,
              }
            : next,
        );
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
  const loadMore = useCallback(() => feed.current?.loadMore(), []);
  const setVisibleRange = useCallback(
    (start: number, end: number, anchor?: number) =>
      feed.current?.setVisibleRange(start, end, anchor),
    [],
  );
  return { ...state, loadMore, setVisibleRange };
}
