import { resolveSelector } from "./aliases.ts";
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
  const resolvedId = resolveSelector(state.types, selectedId)?.id ?? selectedId;
  useLayoutEffect(
    () => controller.select(resolvedId),
    [controller, resolvedId],
  );
  useLayoutEffect(() => controller.previews(previews), [controller, previews]);
  const view =
    state.selectedId === resolvedId
      ? state
      : {
          ...state,
          item: state.types.find((row) => row.id === resolvedId),
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
