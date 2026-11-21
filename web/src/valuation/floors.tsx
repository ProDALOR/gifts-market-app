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
import { roundedAmount } from "../amount";
import { Quote } from "../ui/market-controls";
import "./floors.css";
const empty: FloorsState = {
  currency: "",
  selected: "",
  entries: new Map(),
  error: "",
  detail: { pending: false, error: "" },
};
const Context = createContext<{
  state: FloorsState;
  retry: () => void;
}>({ state: empty, retry: () => {} });
export function FloorsProvider({
  currency,
  collection,
  collections,
  order,
  children,
}: {
  currency: string;
  collection: string;
  collections?: readonly string[];
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
  const ids = useMemo(
    () => collections ?? (collection ? [collection] : []),
    [collections, collection],
  );
  useLayoutEffect(() => {
    controller.configure(
      currency,
      ids,
      collection,
      model && backdrop ? { model_id: model, backdrop_id: backdrop } : null,
    );
  }, [controller, currency, ids, collection, model, backdrop]);
  useEffect(() => () => controller.close(), [controller]);
  const view =
    state.currency === currency && state.selected === collection
      ? state
      : { ...empty, currency, selected: collection };
  const value = useMemo(
    () => ({
      state: view,
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
export function useCollectionFloorDetails(id: string, currency?: string) {
  const { state } = useFloorReport();
  return state.selected === id && (!currency || state.currency === currency)
    ? state.detail
    : empty.detail;
}
export function FloorBadge({
  floor,
  currency,
  stale = false,
  label = "Current floor",
  compact = false,
  rounded = false,
  abbreviated = false,
}: {
  floor?: SharedFloor;
  currency: string;
  stale?: boolean;
  label?: string;
  compact?: boolean;
  rounded?: boolean;
  abbreviated?: boolean;
}) {
  if (!floor) return null;
  const price = "value" in floor.price ? floor.price.value : undefined;
  const message =
    "error" in floor.price ? floor.price.error.message : undefined;
  const value = price ? `${price} ${currency}` : "unavailable";
  const display = price && rounded ? roundedAmount(price) : price;
  return (
    <span
      className={`floor-badge${stale ? " stale" : ""}`}
      title={`${label}: ${value}${stale ? " · updates pending" : ""}${message ? `: ${message}` : ""}\n${evidenceTooltip(floor)}`}
      aria-label={`${label}: ${display ? `${display} ${currency}` : "unavailable"}${stale ? "; updates pending" : ""}`}
    >
      {!compact && <span>{label}</span>}
      {display ? (
        <Quote
          price={abbreviated ? price! : display}
          currency={currency}
          format={abbreviated ? "compact" : "exact"}
        />
      ) : (
        <span>—</span>
      )}
    </span>
  );
}
const counts = new Intl.NumberFormat();
function giftCount(floor: SharedFloor) {
  const { gifts } = floor.evidence;
  return `${counts.format(gifts)} ${gifts === 1 ? "gift" : "gifts"}`;
}
function evidenceTooltip(floor: SharedFloor) {
  const { listings } = floor.evidence;
  return `${giftCount(floor)} · ${counts.format(listings)} ${listings === 1 ? "listing" : "listings"}\nGifts are unique identifiable collectibles with current asks in this scope. Listings are sale offers; the same gift can appear on several markets. Offers without an identifiable gift contribute only to listings.`;
}
/** Each scope shows its own live valuation evidence, independently of gift estimates. */
export function FloorSummary(props: ComponentProps<typeof FloorBadge>) {
  if (!props.floor) return null;
  return (
    <span className={`floor-summary${props.stale ? " stale" : ""}`}>
      <FloorBadge {...props} />
      <small className="floor-evidence" title={evidenceTooltip(props.floor)}>
        {giftCount(props.floor)}
      </small>
      {!props.compact && "error" in props.floor.price && (
        <small className="floor-evidence">
          {props.floor.price.error.message}
        </small>
      )}
    </span>
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
      abbreviated
    />
  );
}
export function TraitFloor({
  collection,
  id,
  kind,
  currency,
  rounded = false,
}: {
  collection?: string;
  currency?: string;
  id: string;
  kind: "model" | "backdrop" | "pattern";
  rounded?: boolean;
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
      rounded={rounded}
    />
  );
}
