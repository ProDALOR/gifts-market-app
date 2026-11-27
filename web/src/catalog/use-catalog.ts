import { resolveSelector } from "./aliases.ts";
import { useLayoutEffect, useMemo, useSyncExternalStore } from "react";
import { CatalogController } from "./controller";
import { useSharedCatalog } from "./provider";
import type { CatalogOption } from "../market";
const empty: ReadonlyMap<string, CatalogOption> = new Map();
/** React renders controller state; requests and live-update recovery live in the controller. */
const noPreviews: readonly string[] = [];
export function useCatalog(
  selectedId: string,
  previews: readonly string[] = noPreviews,
) {
  const shared = useSharedCatalog();
  const controller = useMemo(() => shared ?? new CatalogController(), [shared]);
  const state = useSyncExternalStore(
    controller.subscribe,
    controller.getSnapshot,
  );
  const resolvedId = resolveSelector(state.types, selectedId)?.id ?? selectedId;
  useLayoutEffect(
    () => controller.select(resolvedId),
    [controller, resolvedId],
  );
  useLayoutEffect(() => controller.previews(previews), [controller, previews]);
  useLayoutEffect(() => {
    controller.start();
    return () => (shared ? controller.suspend() : controller.close());
  }, [controller, shared]);
  const cached = controller.peekOptions(resolvedId);
  const view =
    state.selectedId === resolvedId
      ? state
      : {
          ...state,
          item: state.types.find((row) => row.id === resolvedId),
          options: cached ?? empty,
          optionsLoading: !!selectedId && cached === undefined,
          optionsError: "",
        };
  return {
    ...view,
    retryOptions: controller.retryOptions,
    retryCatalog: controller.retryCatalog,
  };
}
