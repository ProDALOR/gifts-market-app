/** Equalize row occupancy instead of leaving a nearly empty final row. */
export function balancedRows<T>(items: readonly T[], capacity: number): T[][] {
  if (!items.length) return [];
  const rowCount = Math.ceil(items.length / Math.max(1, Math.floor(capacity)));
  const size = Math.floor(items.length / rowCount),
    extra = items.length % rowCount;
  let offset = 0;
  return Array.from({ length: rowCount }, (_, index) => {
    const row = items.slice(offset, offset + size + (index < extra ? 1 : 0));
    offset += row.length;
    return row;
  });
}
/** Flat grid placement preserves keyed media nodes when row membership changes. */
export function balancedGrid<T>(items: readonly T[], capacity: number) {
  const rows = balancedRows(items, capacity);
  const gcd = (a: number, b: number): number => (b ? gcd(b, a % b) : a);
  const columns = rows.reduce(
    (total, row) => (total * row.length) / gcd(total, row.length),
    1,
  );
  return {
    columns,
    rows: rows.length,
    cells: rows.flatMap((row, index) =>
      row.map((item, column) => ({
        item,
        row: index + 1,
        column: column * (columns / row.length) + 1,
        span: columns / row.length,
      })),
    ),
  };
}
export function previewLayout(
  width: number,
  models: number,
  backdrops: number,
  patterns: number,
) {
  // Keep normal artwork through four rows, then compact in readable steps.
  const sizes = [
    { modelSize: 52, modelRowHeight: 72, columnWidth: 80 },
    { modelSize: 40, modelRowHeight: 60, columnWidth: 64 },
    { modelSize: 32, modelRowHeight: 50, columnWidth: 56 },
  ];
  const layouts = sizes.map((size) => ({
    ...size,
    modelColumns: Math.max(1, Math.floor((width - 24 + 8) / size.columnWidth)),
  }));
  const { modelColumns, modelSize, modelRowHeight } =
    layouts.find((layout) => models <= layout.modelColumns * 4) || layouts[2];
  const backdropColumns = Math.max(1, Math.floor(width / 40));
  const patternColumns = Math.max(1, Math.ceil(width / 38));
  return {
    modelColumns,
    modelSize,
    modelRowHeight,
    backdropColumns,
    height: Math.max(
      84,
      models ? Math.ceil(models / modelColumns) * modelRowHeight + 24 : 0,
      Math.ceil(backdrops / backdropColumns) * 42,
      Math.ceil(patterns / patternColumns) * 28,
    ),
  };
}
/** Fixed-size wallpaper. Extra height adds rows without stretching existing ones. */
export function bannerPatternTiles(
  width: number,
  height: number,
  symbols: number,
) {
  if (width <= 0 || height <= 0 || symbols <= 0) return [];
  const columns = Math.max(1, Math.ceil(width / 38));
  const rows = Math.ceil(height / 28);
  return Array.from({ length: columns * rows }, (_, index) => {
    const row = Math.floor(index / columns),
      column = index % columns;
    return {
      x: (column + 0.5 + (row % 2 ? 0.15 : 0)) * 38 - 7,
      y: (row + 0.5) * 28 - 7,
      size: 14,
      symbol: index % symbols,
    };
  });
}
