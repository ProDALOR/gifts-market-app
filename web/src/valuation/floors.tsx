import {
  createContext,
  useContext,
  useEffect,
  useLayoutEffect,
  useMemo,
  useSyncExternalStore,
} from "react";
import type { ComponentProps, ReactNode } from "react";
import { FloorsController, traitFloor } from "./floors-controller";
import type {
  FloorsEntry,
  FloorsState,
  SharedFloor,
} from "./floors-controller";
import type { Order } from "../market";
import { Quote } from "../ui/market-controls";
import "./floors.css";
const empty: FloorsState = {
  currency: "",
  selected: "",
  entries: new Map(),
  error: "",
};
const Context = createContext<{
  state: FloorsState;
  visible: (ids: string[]) => void;
  retry: () => void;
}>({ state: empty, visible: () => {}, retry: () => {} });
export function FloorsProvider({
  currency,
  collection,
  order,
  children,
}: {
  currency: string;
  collection: string;
  order?: Order;
  children: ReactNode;
}) {
  const controller = useMemo(() => new FloorsController(), []);
  const state = useSyncExternalStore(
    controller.subscribe,
    controller.getSnapshot,
  );
  const traits =
    order?.target.kind === "gift" &&
    order.book.collection_id === collection &&
    order.book.currency_id === currency
      ? order.target.traits
      : undefined;
  const model = traits?.model,
    backdrop = traits?.backdrop;
  useLayoutEffect(() => {
    controller.select(
      currency,
      collection,
      model && backdrop ? { model_id: model, backdrop_id: backdrop } : null,
    );
  }, [controller, currency, collection, model, backdrop]);
  useEffect(() => () => controller.close(), [controller]);
  const view =
    state.currency === currency && state.selected === collection
      ? state
      : { ...empty, currency, selected: collection };
  const value = useMemo(
    () => ({
      state: view,
      visible: controller.visibleCollections,
      retry: controller.retry,
    }),
    [view, controller],
  );
  return <Context.Provider value={value}>{children}</Context.Provider>;
}
export const useFloorReport = () => useContext(Context);
export function FloorsStatus() {
  const { state, retry } = useFloorReport();
  return state.error ? (
    <p className="floors-error" role="status" title={state.error}>
      Current floors are unavailable.{" "}
      <button onClick={retry}>Retry floors</button>
    </p>
  ) : null;
}
export function useCollectionFloors(
  id: string,
  currency?: string,
): FloorsEntry | undefined {
  const { state } = useFloorReport();
  return currency && currency !== state.currency
    ? undefined
    : state.entries.get(id);
}
export function FloorBadge({
  floor,
  currency,
  stale = false,
  label = "Current floor",
  compact = false,
}: {
  floor?: SharedFloor;
  currency: string;
  stale?: boolean;
  label?: string;
  compact?: boolean;
}) {
  if (!floor) return null;
  const price = "value" in floor.price ? floor.price.value : undefined;
  const message =
    "error" in floor.price ? floor.price.error.message : undefined;
  const value = price ? `${price} ${currency}` : "unavailable";
  return (
    <span
      className={`floor-badge${stale ? " stale" : ""}`}
      title={`${label}: ${value} · ${evidenceText(floor)}${stale ? " · updates pending" : ""}${message ? `: ${message}` : ""}`}
      aria-label={`${label}: ${value}${stale ? "; updates pending" : ""}`}
    >
      {!compact && <span>{label}</span>}
      {price ? <Quote price={price} currency={currency} /> : <span>—</span>}
    </span>
  );
}
const counts = new Intl.NumberFormat();
function evidenceText(floor: SharedFloor) {
  const { gifts, listings } = floor.evidence;
  return `${counts.format(gifts)} ${gifts === 1 ? "gift" : "gifts"} · ${counts.format(listings)} ${listings === 1 ? "listing" : "listings"}`;
}
/** Each scope shows its own live valuation evidence, independently of gift estimates. */
export function FloorSummary(props: ComponentProps<typeof FloorBadge>) {
  if (!props.floor) return null;
  return (
    <div className={`floor-summary${props.stale ? " stale" : ""}`}>
      <FloorBadge {...props} />
      <small
        className="floor-evidence"
        title="Identifiable gifts and listings with eligible current asks"
      >
        {evidenceText(props.floor)}
      </small>
      {"error" in props.floor.price && (
        <small className="floor-evidence">
          {props.floor.price.error.message}
        </small>
      )}
    </div>
  );
}
export function CollectionFloor({ id }: { id: string }) {
  const { state } = useFloorReport();
  const entry = state.entries.get(id);
  return (
    <FloorBadge
      floor={entry?.floor}
      stale={entry?.stale}
      currency={state.currency}
      compact
    />
  );
}
export function TraitFloor({
  collection,
  id,
  kind,
  currency,
}: {
  collection?: string;
  currency?: string;
  id: string;
  kind: "model" | "backdrop" | "pattern";
}) {
  const { state } = useFloorReport();
  if (currency && currency !== state.currency) return null;
  const entry = state.entries.get(collection || state.selected);
  const floor = traitFloor(entry, kind, id);
  return (
    <FloorBadge
      floor={floor}
      stale={entry?.stale}
      currency={state.currency}
      compact
    />
  );
}
