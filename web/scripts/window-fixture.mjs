// Fake server windows use the same bounded, identity-anchored replacement contract.
export const position = (row) => JSON.stringify([row.book.venue_id, row.id]);
export function orderWindow(query, rows) {
  const size = query.before + query.after + 1;
  const pivot = Math.max(
    0,
    rows.findIndex((row) => position(row) === query.anchor),
  );
  const start = Math.max(0, Math.min(pivot - query.before, rows.length - size));
  return {
    data: rows
      .slice(start, start + size)
      .map((row) => ({ ...row, position: position(row) })),
    anchor: rows[pivot] ? position(rows[pivot]) : null,
    has_before: start > 0,
    has_after: start + size < rows.length,
    cursor: "fixture",
  };
}
