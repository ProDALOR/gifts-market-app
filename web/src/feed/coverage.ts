import type { components } from "../gen/api.ts";
export type OrderRange = components["schemas"]["OrderRange"];
const empty = (range: OrderRange) =>
  range.start !== null && range.start === range.end;
const earlierEnd = (a: string | null, b: string | null) =>
  a === null ? b : b === null ? a : a < b ? a : b;
/** Visited half-open intervals. Null is the open head for starts and open tail for ends. */
export class OrderCoverage {
  private ranges: OrderRange[] = [];
  get hasBefore() {
    return this.ranges[0]?.start != null;
  }
  containing(key: string) {
    return this.ranges.find(
      (range) =>
        (range.start === null || key >= range.start) &&
        (range.end === null || key < range.end),
    );
  }
  add(range: OrderRange) {
    if (empty(range)) return;
    const sorted = [...this.ranges, { ...range }].sort((a, b) =>
      a.start === b.start
        ? 0
        : a.start === null
          ? -1
          : b.start === null
            ? 1
            : a.start < b.start
              ? -1
              : 1,
    );
    this.ranges = [];
    for (const next of sorted) {
      const last = this.ranges.at(-1);
      if (
        last &&
        (last.end === null || next.start === null || next.start <= last.end)
      ) {
        last.end =
          last.end === null || next.end === null
            ? null
            : last.end > next.end
              ? last.end
              : next.end;
      } else this.ranges.push({ ...next });
    }
  }
  remove(range: OrderRange) {
    if (empty(range)) return;
    this.ranges = this.ranges.flatMap((part) => {
      if (
        (part.end !== null &&
          range.start !== null &&
          part.end <= range.start) ||
        (range.end !== null && part.start !== null && range.end <= part.start)
      )
        return [part];
      const pieces: OrderRange[] = [];
      if (
        range.start !== null &&
        (part.start === null || part.start < range.start)
      )
        pieces.push({ start: part.start, end: range.start });
      if (range.end !== null && (part.end === null || part.end > range.end))
        pieces.push({ start: range.end, end: part.end });
      return pieces;
    });
  }
  gap(range: OrderRange = { start: null, end: null }): OrderRange | undefined {
    if (empty(range)) return;
    let cursor = range.start;
    for (const part of this.ranges) {
      if (part.end !== null && cursor !== null && part.end <= cursor) continue;
      if (range.end !== null && part.start !== null && part.start >= range.end)
        break;
      if (part.start !== null && (cursor === null || cursor < part.start))
        return { start: cursor, end: earlierEnd(part.start, range.end) };
      if (part.end === null || (range.end !== null && part.end >= range.end))
        return;
      cursor = part.end;
    }
    return { start: cursor, end: range.end };
  }
}
