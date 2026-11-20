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

// Telegram's 420 × 280 gift SVG; coordinates are top-left positions of a
// 100 × 100 symbol before its per-instance scale. Preserve individual alpha.
// Source: https://t.me/nft/ResistanceDog-14029 (2026-11-19).
const GIFT_PATTERN = [
  [140.5761, 13.79, 0.3, 0.212890625],
  [249.465, 13.79, 0.3, 0.212890625],
  [291.8539, 102.7918, 0.3, 0.223865327],
  [98.1872, 102.7918, 0.3, 0.223865327],
  [276.2551, 176.2043, 0.277, 0.221633185],
  [196.144, 188.6412, 0.277, 0.123046875],
  [116.0329, 176.2043, 0.277, 0.221633185],
  [355.0988, 79.3286, 0.2247, 0.189569382],
  [292.0988, 52.1228, 0.2247, 0.260904948],
  [334.0988, 17.5326, 0.2247, 0.146437872],
  [198.7654, -5.7866, 0.2247, 0.153087798],
  [63.4321, 17.5326, 0.2247, 0.145321801],
  [105.4321, 52.1228, 0.2247, 0.260904948],
  [42.4321, 79.3286, 0.2247, 0.165899368],
  [72.7654, 155.8935, 0.2247, 0.165899368],
  [49.4321, 205.6413, 0.2247, 0.10500372],
  [344.2099, 205.6413, 0.2247, 0.10500372],
  [337.2099, 155.8935, 0.2247, 0.152669271],
] as const;

/** Keep native symbol spacing; cards center the composition's model anchor at y=110. */
export function giftPatternTiles(
  width: number,
  height: number,
  centered = false,
) {
  if (width <= 0 || height <= 0) return [];
  const scale = Math.max(width / 420, height / 280);
  const left = (width - 420 * scale) / 2;
  const top = (height - 280 * scale) / 2 + (centered ? 30 * scale : 0);
  return GIFT_PATTERN.map(([x, y, size, opacity]) => ({
    x: left + x * scale,
    y: top + y * scale,
    size: 100 * size * scale,
    opacity,
  }));
}
