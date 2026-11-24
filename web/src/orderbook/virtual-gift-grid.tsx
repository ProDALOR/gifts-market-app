import {
  Fragment,
  useCallback,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import type { ReactNode } from "react";
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
function locateAnchor<T>(
  orders: T[],
  previous: T[],
  key: string,
  keyOf: (item: T) => string,
) {
  const index = orders.findIndex((order) => keyOf(order) === key);
  if (index >= 0 || !orders.length) return index;
  const keys = new Map(orders.map((order, index) => [keyOf(order), index]));
  const old = previous.findIndex((order) => keyOf(order) === key);
  const nearby = [
    ...previous.slice(Math.max(0, old)),
    ...previous.slice(0, Math.max(0, old)).reverse(),
  ];
  return (
    nearby
      .map((order) => keys.get(keyOf(order)))
      .find((index) => index !== undefined) ?? 0
  );
}
/** Window rows, not individual cards, so columns and scroll geometry stay stable. */
export function VirtualGiftGrid<T>({
  orders,
  render,
  keyOf,
  className = "",
  label = "Gifts",
  busy,
  visible,
}: {
  orders: T[];
  render: (item: T, index: number) => ReactNode;
  keyOf: (item: T) => string;
  className?: string;
  label?: string;
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
      ? locateAnchor(orders, previous.current, saved.key, keyOf)
      : -1;
  let rendered = range;
  if (nextAnchor >= 0 && saved) {
    const { columns, stride, gap } = geometry.current;
    const old = previous.current.findIndex(
      (order) => keyOf(order) === saved.key,
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
    const saved = anchor.current;
    if (
      saved &&
      (geometry.current.columns !== columns ||
        geometry.current.stride !== stride)
    ) {
      const index = orders.findIndex((order) => keyOf(order) === saved.key);
      if (index >= 0) {
        const offset =
          element.getBoundingClientRect().top +
          Math.floor(index / columns) * stride;
        window.scrollBy({ top: offset - saved.offset, behavior: "instant" });
      }
    }
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
    const rowStart = first * columns;
    const savedOffset = saved
      ? orders
          .slice(rowStart, rowStart + columns)
          .findIndex((order) => keyOf(order) === saved.key)
      : -1;
    // Keep the same gift when a wider row starts with an earlier sibling.
    const index = rowStart + Math.max(0, savedOffset);
    visible(next.start, next.end, Math.min(orders.length - 1, index));
    // A scrolled viewer can still see the grid below the header. Preserve that
    // first row when a preceding window arrives; keep the unscrolled head free.
    anchor.current =
      (top < 0 || window.scrollY > 0) && orders[index]
        ? { key: keyOf(orders[index]), offset: top + first * stride }
        : undefined;
  }, [orders, visible, keyOf]);
  useLayoutEffect(() => {
    const saved = anchor.current;
    if (saved && host.current) {
      const index = locateAnchor(orders, previous.current, saved.key, keyOf);
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
      className={`gift-grid ${className}`}
      data-order-count={orders.length}
      aria-label={label}
      aria-busy={busy}
      style={{ paddingTop: rendered.top, paddingBottom: rendered.bottom }}
    >
      {orders.slice(rendered.start, rendered.end).map((item, index) => (
        <Fragment key={keyOf(item)}>
          {render(item, rendered.start + index)}
        </Fragment>
      ))}
    </ul>
  );
}
