import {
  useEffect,
  useLayoutEffect,
  useMemo,
  useSyncExternalStore,
} from "react";
import { CatalogController } from "./controller";
import type { CatalogOption } from "../market";
const empty: ReadonlyMap<string, CatalogOption> = new Map();
/** React renders controller state; requests and live-update recovery live in the controller. */
const noPreviews: readonly string[] = [];
export function useCatalog(
  refresh: number,
  selectedId: string,
  previews: readonly string[] = noPreviews,
) {
  const controller = useMemo(() => new CatalogController(), [refresh]);
  const state = useSyncExternalStore(
    controller.subscribe,
    controller.getSnapshot,
  );
  useEffect(() => {
    controller.start();
    return () => controller.close();
  }, [controller]);
  useLayoutEffect(
    () => controller.select(selectedId),
    [controller, selectedId],
  );
  useLayoutEffect(() => controller.previews(previews), [controller, previews]);
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
