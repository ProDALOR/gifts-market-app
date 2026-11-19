import { useSyncExternalStore } from "react";
import { liveSocket } from "./live-socket.ts";
const subscribe = (notify: () => void) => liveSocket.watchStatus(notify);
const inactive = () => () => {};
export const useMarketStatus = (enabled = true) =>
  useSyncExternalStore(
    enabled ? subscribe : inactive,
    liveSocket.statusSnapshot,
    liveSocket.statusSnapshot,
  );
