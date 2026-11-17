import { useEffect, useMemo, useSyncExternalStore } from "react";
import { CatalogController } from "./controller";
import type { CatalogOption } from "../market";
const empty: ReadonlyMap<string, CatalogOption> = new Map();
/** React renders controller state; requests and live-update recovery live in the controller. */
export function useCatalog(refresh: number, selectedId: string) {
  const controller = useMemo(() => new CatalogController(), [refresh]);
  const state = useSyncExternalStore(
    controller.subscribe,
    controller.getSnapshot,
  );
  useEffect(() => {
    controller.start();
    return () => controller.close();
  }, [controller]);
  useEffect(() => controller.select(selectedId), [controller, selectedId]);
  const view =
    state.selectedId === selectedId
      ? state
      : {
          ...state,
          item: state.types.find((row) => row.id === selectedId),
          options: empty,
          optionsLoading: !!selectedId,
          optionsError: "",
        };
  return {
    ...view,
    retryOptions: controller.retryOptions,
    retryCatalog: controller.retryCatalog,
  };
}
