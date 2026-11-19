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
  const range = query.range ?? { start: null, end: null };
  const matching = ordered(rows).filter(
    (row) =>
      (range.start == null || row.key >= range.start) &&
      (range.end == null || row.key <= range.end),
  );
  const limit = Math.min(query.limit ?? 100, 100);
  const data = matching.slice(0, limit),
    more = matching.length > limit;
  const last = data.at(-1)?.key;
  return {
    data,
    next: more ? last + (last.length % 4 === 3 ? "0" : "00") : null,
    range: {
      start: range.start ?? null,
      end: more ? last : (range.end ?? null),
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
        next: next.next,
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
  return orderResult(query, rows);
}
