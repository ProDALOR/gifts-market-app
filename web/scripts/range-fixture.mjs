// Synthetic opaque ordering keys for browser regressions; production keys are server-owned.
export const position = (row) => JSON.stringify([row.book.venue_id, row.id]);
const alphabet =
  "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ_abcdefghijklmnopqrstuvwxyz~";
const standard =
  "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const encode = (value) =>
  Buffer.from(value)
    .toString("base64")
    .replace(/=/g, "")
    .replace(/./g, (c) => alphabet[standard.indexOf(c)]);
const ordered = (rows) =>
  rows.map((row, index) => ({
    ...row,
    key: encode(`${String(index).padStart(16, "0")}:${position(row)}`),
  }));
export function orderResult(query, rows) {
  const all = ordered(rows),
    size = query.limit;
  const nav = query.navigation ?? { type: "head" };
  let pivot = Math.max(
    0,
    all.findIndex((row) => row.key === nav.key),
  );
  const start =
    nav.type === "around"
      ? Math.max(0, Math.min(pivot - Math.floor(size / 3), all.length - size))
      : nav.type === "before"
        ? Math.max(0, pivot - size)
        : nav.type === "after"
          ? pivot + 1
          : 0;
  const end = nav.type === "before" ? pivot : start + size;
  const data = all.slice(start, end);
  return {
    data,
    next: end < all.length ? (data.at(-1)?.key ?? null) : null,
    range: {
      start: start > 0 ? (data[0]?.key ?? null) : null,
      end: end < all.length ? (data.at(-1)?.key ?? null) : null,
    },
  };
}
export class RangeWatch {
  constructor(query, rows) {
    this.initial = orderResult(query, rows);
    this.sequence = -1;
  }
  frame(id, rows) {
    const next =
      this.sequence < 0
        ? this.initial
        : {
            ...this.initial,
            data: ordered(rows).filter(
              (row) =>
                (!this.initial.range.start ||
                  row.key >= this.initial.range.start) &&
                (!this.initial.range.end || row.key <= this.initial.range.end),
            ),
          };
    const sequence = ++this.sequence,
      old = this.previous;
    this.previous = next;
    if (!old)
      return {
        type: "orders.snapshot",
        id,
        sequence,
        data: next.data,
        range: next.range,
      };
    const previous = new Map(old.data.map((row) => [position(row), row])),
      current = new Set(next.data.map(position));
    return {
      type: "orders.changed",
      id,
      sequence,
      upserts: next.data.filter(
        (row) =>
          JSON.stringify(previous.get(position(row))) !== JSON.stringify(row),
      ),
      removed: old.data
        .filter((row) => !current.has(position(row)))
        .map((row) => ({ venue_id: row.book.venue_id, id: row.id })),
    };
  }
}

export function orderSnapshot(query, rows) {
  const { data, range } = orderResult(query, rows);
  return { data, range };
}
