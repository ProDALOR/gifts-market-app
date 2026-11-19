import { useSyncExternalStore } from "react";
import { liveSocket } from "./live-socket.ts";
const subscribe = (notify: () => void) => liveSocket.watchStatus(notify);
export const useMarketStatus = () =>
  useSyncExternalStore(
    subscribe,
    liveSocket.statusSnapshot,
    liveSocket.statusSnapshot,
  );
