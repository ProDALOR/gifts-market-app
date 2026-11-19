import { useCallback, useLayoutEffect, useRef, useState } from "react";
import type { Order } from "../market";
import type { ComponentProps } from "react";
import { GiftCard } from "./gift-card";
import { orderKey } from "./order-activity";
type Range = { start: number; end: number; top: number; bottom: number };
function rowsRange(
  length: number,
  columns: number,
  stride: number,
  gap: number,
  start: number,
  end: number,
): Range {
  const rows = Math.ceil(length / columns);
  start = Math.max(0, Math.min(start, rows));
  end = Math.max(start, Math.min(end, rows));
  return {
    start: Math.min(length, start * columns),
    end: Math.min(length, end * columns),
    top: start * stride - (start === end && start > 0 && length ? gap : 0),
    bottom: Math.max(0, rows - end) * stride,
  };
}
function locateAnchor(orders: Order[], previous: Order[], key: string) {
  const index = orders.findIndex((order) => orderKey(order) === key);
  if (index >= 0 || !orders.length) return index;
  const keys = new Map(orders.map((order, index) => [orderKey(order), index]));
  const old = previous.findIndex((order) => orderKey(order) === key);
  const nearby = [
    ...previous.slice(Math.max(0, old)),
    ...previous.slice(0, Math.max(0, old)).reverse(),
  ];
  return (
    nearby
      .map((order) => keys.get(orderKey(order)))
      .find((index) => index !== undefined) ?? 0
  );
}
/** Window rows, not individual cards, so columns and scroll geometry stay stable. */
export function GiftGrid({
  orders,
  card,
  busy,
  visible,
}: {
  orders: Order[];
  card: (order: Order) => Omit<ComponentProps<typeof GiftCard>, "order">;
  busy: boolean;
  visible: (start: number, end: number, anchor?: number) => void;
}) {
  const host = useRef<HTMLUListElement>(null);
  const anchor = useRef<{ key: string; offset: number } | undefined>(undefined);
  const geometry = useRef({ columns: 3, stride: 118, gap: 6 });
  const [range, setRange] = useState({ start: 0, end: 30, top: 0, bottom: 0 });
  const previous = useRef(orders);
  const saved = anchor.current;
  const nextAnchor =
    previous.current !== orders && saved
      ? locateAnchor(orders, previous.current, saved.key)
      : -1;
  let rendered = range;
  if (nextAnchor >= 0 && saved) {
    const { columns, stride, gap } = geometry.current;
    const old = previous.current.findIndex(
      (order) => orderKey(order) === saved.key,
    );
    const shift =
      Math.floor(nextAnchor / columns) - Math.floor(Math.max(0, old) / columns);
    // Rebase before React commits the new data, so visible keys never unmount
    // under obsolete virtual row indices and restart their artwork.
    rendered = rowsRange(
      orders.length,
      columns,
      stride,
      gap,
      Math.floor(range.start / columns) + shift,
      Math.ceil(range.end / columns) + shift,
    );
  }
  const measure = useCallback(() => {
    const element = host.current;
    if (!element) return;
    const style = getComputedStyle(element);
    const columns = style.gridTemplateColumns.split(" ").length;
    const stride = parseFloat(style.gridAutoRows) + parseFloat(style.rowGap);
    if (!stride || !Number.isFinite(stride)) return;
    geometry.current = { columns, stride, gap: parseFloat(style.rowGap) };
    const top = element.getBoundingClientRect().top;
    const first = Math.max(0, Math.floor(-top / stride));
    const startRow = Math.min(
      Math.ceil(orders.length / columns),
      Math.max(0, first - 3),
    );
    const endRow = Math.max(
      startRow,
      Math.min(
        Math.ceil(orders.length / columns),
        Math.ceil((innerHeight - top) / stride) + 3,
      ),
    );
    const next = rowsRange(
      orders.length,
      columns,
      stride,
      parseFloat(style.rowGap),
      startRow,
      endRow,
    );
    setRange((old) =>
      Object.keys(next).every(
        (k) => old[k as keyof typeof old] === next[k as keyof typeof next],
      )
        ? old
        : next,
    );
    visible(next.start, next.end, Math.min(orders.length - 1, first * columns));
    const index = first * columns;
    // A scrolled viewer can still see the grid below the header. Preserve that
    // first row when a preceding window arrives; keep the unscrolled head free.
    anchor.current =
      (top < 0 || window.scrollY > 0) && orders[index]
        ? { key: orderKey(orders[index]), offset: top + first * stride }
        : undefined;
  }, [orders, visible]);
  useLayoutEffect(() => {
    const saved = anchor.current;
    if (saved && host.current) {
      const index = locateAnchor(orders, previous.current, saved.key);
      if (index >= 0) {
        const { columns, stride } = geometry.current;
        const offset =
          host.current.getBoundingClientRect().top +
          Math.floor(index / columns) * stride;
        window.scrollBy({ top: offset - saved.offset, behavior: "instant" });
      }
    }
    previous.current = orders;
    measure();
    let frame = 0;
    const schedule = () => {
      if (!frame)
        frame = requestAnimationFrame(() => {
          frame = 0;
          measure();
        });
    };
    const observer = new ResizeObserver(schedule);
    if (host.current) observer.observe(host.current);
    window.addEventListener("scroll", schedule, { passive: true });
    window.addEventListener("resize", schedule);
    return () => {
      observer.disconnect();
      cancelAnimationFrame(frame);
      window.removeEventListener("scroll", schedule);
      window.removeEventListener("resize", schedule);
    };
  }, [orders, measure]);
  return (
    <ul
      ref={host}
      className="gift-grid"
      data-order-count={orders.length}
      aria-label="Gift listings"
      aria-busy={busy}
      style={{ paddingTop: rendered.top, paddingBottom: rendered.bottom }}
    >
      {orders.slice(rendered.start, rendered.end).map((order, index) => (
        <GiftCard
          key={orderKey(order)}
          order={order}
          {...card(order)}
          position={rendered.start + index + 1}
          total={orders.length}
        />
      ))}
    </ul>
  );
}
