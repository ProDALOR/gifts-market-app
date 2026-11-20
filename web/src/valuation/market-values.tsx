import {
  createContext,
  useContext,
  useEffect,
  useMemo,
  useSyncExternalStore,
} from "react";
import type { ReactNode } from "react";
import { MarketValuesController } from "./market-values-controller";
import type {
  ValuesEntry,
  ValuesState,
  SharedFloor,
} from "./market-values-controller";
import type { Order } from "../market";
import { Quote } from "../ui/market-controls";
import "./market-values.css";
const empty: ValuesState = {
  currency: "",
  selected: "",
  entries: new Map(),
  error: "",
};
const Context = createContext<{
  state: ValuesState;
  visible: (ids: string[]) => void;
  retry: () => void;
}>({ state: empty, visible: () => {}, retry: () => {} });
export function MarketValuesProvider({
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
  const controller = useMemo(() => new MarketValuesController(), []);
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
  useEffect(() => {
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
export const useMarketValues = () => useContext(Context);
export function MarketValuesStatus() {
  const { state, retry } = useMarketValues();
  return state.error ? (
    <p className="market-values-error" role="status" title={state.error}>
      Current floors are unavailable.{" "}
      <button onClick={retry}>Retry floors</button>
    </p>
  ) : null;
}
export function useCollectionValues(
  id: string,
  currency?: string,
): ValuesEntry | undefined {
  const { state } = useMarketValues();
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
  return (
    <span
      className={`floor-badge${stale ? " stale" : ""}`}
      title={`${label}${stale ? " · updates pending" : ""}${message ? `: ${message}` : ""}`}
      aria-label={`${label}: ${price ? `${price} ${currency}` : "unavailable"}${stale ? "; updates pending" : ""}`}
    >
      {!compact && <span>{label}</span>}
      {price ? <Quote price={price} currency={currency} /> : <span>—</span>}
    </span>
  );
}
export function CollectionFloor({ id }: { id: string }) {
  const { state } = useMarketValues();
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
  const { state } = useMarketValues();
  if (currency && currency !== state.currency) return null;
  const entry = state.entries.get(collection || state.selected);
  const floor =
    kind === "model"
      ? entry?.traits?.models[id]
      : kind === "backdrop"
        ? entry?.traits?.backdrops[id]
        : undefined;
  return (
    <FloorBadge
      floor={floor}
      stale={entry?.stale}
      currency={state.currency}
      compact
    />
  );
}
