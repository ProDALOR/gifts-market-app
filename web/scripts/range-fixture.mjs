// Synthetic ordered slices and current-state range batches for browser regressions.
export const position = (row) => JSON.stringify([row.book.venue_id, row.id]);
const ordered = (rows) =>
  rows.map((row, index) => ({
    ...row,
    position: position(row),
    sort_key: `${String(index).padStart(16, "0")}:${position(row)}`,
  }));
const boundary = (row) => ({ position: row.position, sort_key: row.sort_key });
export function orderResult(query, rows) {
  const all = ordered(rows);
  const size = query.limit;
  const pivot = Math.max(
    0,
    all.findIndex((row) => row.position === query.position),
  );
  const before =
    query.direction === "around"
      ? Math.floor(size / 3)
      : query.direction === "backward"
        ? size - 1
        : 0;
  const start =
    query.direction === "around"
      ? Math.max(0, Math.min(pivot - before, all.length - size))
      : Math.max(0, pivot - before);
  const end = query.direction === "backward" ? pivot + 1 : start + size;
  const data = all.slice(start, end);
  return {
    data,
    range: {
      start: start > 0 ? boundary(data[0]) : null,
      end: end < all.length ? boundary(data.at(-1)) : null,
    },
  };
}
export class RangeWatch {
  constructor(query, rows) {
    this.initial = orderResult(query, rows);
    this.sequence = -1;
  }
  frame(id, rows, sync, revision) {
    const next =
      this.sequence < 0
        ? this.initial
        : {
            range: this.initial.range,
            data: ordered(rows).filter(
              (row) =>
                (!this.initial.range.start ||
                  row.sort_key >= this.initial.range.start.sort_key) &&
                (!this.initial.range.end ||
                  row.sort_key <= this.initial.range.end.sort_key),
            ),
          };
    const sequence = ++this.sequence;
    const old = this.previous;
    this.previous = next;
    if (!old)
      return {
        type: "orders.snapshot",
        id,
        sequence,
        reset: false,
        result: next,
        sync,
        revision,
      };
    const previous = new Map(old.data.map((row) => [position(row), row]));
    const current = new Set(next.data.map(position));
    return {
      type: "orders.changed",
      id,
      sequence,
      base_sequence: sequence - 1,
      upserts: next.data.filter(
        (row) =>
          JSON.stringify(previous.get(position(row))) !== JSON.stringify(row),
      ),
      removed: old.data
        .filter((row) => !current.has(position(row)))
        .map((row) => ({ venue_id: row.book.venue_id, id: row.id })),
      sync,
      revision,
    };
  }
}
