import { useCallback, useLayoutEffect, useRef, useState } from "react";
import type { Order } from "../market";
import type { ComponentProps } from "react";
import { GiftCard } from "./gift-card";
import { orderKey } from "./order-activity";
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
  const geometry = useRef({ columns: 3, stride: 118 });
  const [range, setRange] = useState({ start: 0, end: 30, top: 0, bottom: 0 });
  const measure = useCallback(() => {
    const element = host.current;
    if (!element) return;
    const style = getComputedStyle(element);
    const columns = style.gridTemplateColumns.split(" ").length;
    const stride = parseFloat(style.gridAutoRows) + parseFloat(style.rowGap);
    if (!stride || !Number.isFinite(stride)) return;
    geometry.current = { columns, stride };
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
    const start = Math.min(orders.length, startRow * columns);
    const end = Math.max(start, Math.min(orders.length, endRow * columns));
    const gap = parseFloat(style.rowGap);
    const empty = start === end && orders.length > 0;
    const next = {
      start,
      end,
      top: startRow * stride - (empty && startRow > 0 ? gap : 0),
      bottom: Math.max(0, Math.ceil(orders.length / columns) - endRow) * stride,
    };
    setRange((old) =>
      Object.keys(next).every(
        (k) => old[k as keyof typeof old] === next[k as keyof typeof next],
      )
        ? old
        : next,
    );
    visible(start, end, Math.min(orders.length - 1, first * columns));
    const index = first * columns;
    // A scrolled viewer can still see the grid below the header. Preserve that
    // first row when a preceding window arrives; keep the unscrolled head free.
    anchor.current =
      (top < 0 || window.scrollY > 0) && orders[index]
        ? { key: orderKey(orders[index]), offset: top + first * stride }
        : undefined;
  }, [orders, visible]);
  const previous = useRef(orders);
  useLayoutEffect(() => {
    const saved = anchor.current;
    if (saved && host.current) {
      let index = orders.findIndex((order) => orderKey(order) === saved.key);
      if (index < 0 && orders.length) {
        const keys = new Map(
          orders.map((order, index) => [orderKey(order), index]),
        );
        const old = previous.current.findIndex(
          (order) => orderKey(order) === saved.key,
        );
        const nearby = [
          ...previous.current.slice(Math.max(0, old)),
          ...previous.current.slice(0, Math.max(0, old)).reverse(),
        ];
        index =
          nearby
            .map((order) => keys.get(orderKey(order)))
            .find((index) => index !== undefined) ?? 0;
      }
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
      style={{ paddingTop: range.top, paddingBottom: range.bottom }}
    >
      {orders.slice(range.start, range.end).map((order, index) => (
        <GiftCard
          key={orderKey(order)}
          order={order}
          {...card(order)}
          position={range.start + index + 1}
          total={orders.length}
        />
      ))}
    </ul>
  );
}
