import assert from "node:assert/strict";
import { test } from "node:test";
import {
  balancedRows,
  previewLayout,
  bannerPatternTiles,
  giftPatternTiles,
} from "../src/previews/preview-layout.ts";
test("gift covers preserve Telegram's native SVG positions, scales and individual opacity", () => {
  const reference = giftPatternTiles(420, 280);
  assert.equal(reference.length, 18);
  assert.deepEqual(reference[0], {
    x: 140.5761,
    y: 13.79,
    size: 30,
    opacity: 0.212890625,
  });
  assert.equal(reference[5].opacity, 0.123046875);
  assert.equal(new Set(reference.map((tile) => tile.size)).size, 3);
  assert.ok(new Set(reference.map((tile) => tile.opacity)).size > 3);
  assert.deepEqual(giftPatternTiles(420, 280), reference);
  assert.deepEqual(
    giftPatternTiles(840, 560),
    reference.map((tile) => ({
      x: tile.x * 2,
      y: tile.y * 2,
      size: tile.size * 2,
      opacity: tile.opacity,
    })),
  );
  // Portrait panes crop the sides of the same composition, rather than
  // moving symbols independently or enlarging them at different rates.
  const portrait = giftPatternTiles(420, 560);
  assert.equal(portrait[0].x, reference[0].x * 2 - 210);
  assert.equal(portrait[0].y, reference[0].y * 2);
  assert.equal(portrait[0].size, reference[0].size * 2);
  const landscape = giftPatternTiles(840, 280);
  assert.equal(landscape[0].x, reference[0].x * 2);
  assert.equal(landscape[0].y, reference[0].y * 2 - 140);
  assert.deepEqual(giftPatternTiles(0, 400), []);
  assert.deepEqual(giftPatternTiles(400, 0), []);
});
test("card symbols keep width-based sizing and consistent radial clearance", () => {
  for (const width of [92, 115, 161, 240]) {
    const short = giftPatternTiles(width, 100, true);
    const tall = giftPatternTiles(width, 240, true);
    assert.deepEqual(
      tall.map((tile) => tile.size),
      short.map((tile) => tile.size),
    );
    assert(
      Math.abs(
        Math.max(...tall.map((tile) => tile.size)) / width - (30 / 420) * 1.35,
      ) < 1e-10,
    );
    const native = giftPatternTiles(width, 240);
    const offset = 30 * Math.max(width / 420, 240 / 280);
    tall.forEach((tile, index) => {
      const original = native[index];
      const centerX = tile.x + tile.size / 2;
      const centerY = tile.y + tile.size / 2;
      const expectedX =
        width / 2 + (original.x + original.size / 2 - width / 2) * 1.12;
      const expectedY =
        120 + (original.y + original.size / 2 + offset - 120) * 1.12;
      assert(Math.abs(centerX - expectedX) < 1e-10);
      assert(Math.abs(centerY - expectedY) < 1e-10);
      assert.equal(tile.opacity, original.opacity);
    });
  }
});
test("backdrops split into balanced complete rows, including twelve as six plus six", () => {
  assert.deepEqual(
    balancedRows(
      Array.from({ length: 12 }, (_, i) => i),
      11,
    ).map((row) => row.length),
    [6, 6],
  );
  for (let count = 1; count <= 100; count++)
    for (const capacity of [1, 4, 8, 11]) {
      const items = Array.from({ length: count }, (_, i) => i),
        rows = balancedRows(items, capacity);
      assert.deepEqual(rows.flat(), items);
      assert.ok(Math.max(...rows.map((row) => row.length)) <= capacity);
      assert.ok(
        Math.max(...rows.map((row) => row.length)) -
          Math.min(...rows.map((row) => row.length)) <=
          1,
      );
    }
});
test("models use available width and leave room for every row and reset control", () => {
  const narrow = previewLayout(358, 20, 12, 12),
    wide = previewLayout(448, 20, 12, 12);
  assert.equal(narrow.modelColumns, 5);
  assert.equal(wide.modelColumns, 5);
  assert.equal(narrow.modelSize, 40);
  assert.equal(wide.modelSize, 52);
  assert.deepEqual(
    balancedRows(Array.from({ length: 20 }), narrow.modelColumns).map(
      (row) => row.length,
    ),
    [5, 5, 5, 5],
  );
  assert.deepEqual(
    balancedRows(Array.from({ length: 20 }), wide.modelColumns).map(
      (row) => row.length,
    ),
    [5, 5, 5, 5],
  );
  assert.equal(narrow.height, 4 * 60 + 24);
  assert.equal(wide.height, 4 * 72 + 24);
});
test("model sizing compacts only beyond four rows and keeps a readable minimum", () => {
  assert.equal(previewLayout(358, 16, 0, 0).modelSize, 52);
  assert.equal(previewLayout(358, 17, 0, 0).modelSize, 40);
  assert.equal(previewLayout(358, 21, 0, 0).modelSize, 32);
  for (const width of [288, 358, 448])
    for (let count = 1; count <= 100; count++) {
      const layout = previewLayout(width, count, 0, 0);
      assert.ok(layout.modelSize >= 32);
      assert.ok(count <= layout.modelColumns * 4 || layout.modelSize === 32);
      assert.equal(
        layout.height,
        Math.max(
          84,
          Math.ceil(count / layout.modelColumns) * layout.modelRowHeight + 24,
        ),
      );
    }
});
test("taller pattern wallpaper repeats the one-row geometry without scaling or shifting", () => {
  const short = bannerPatternTiles(358, 84, 12),
    tall = bannerPatternTiles(358, 384, 12);
  assert.deepEqual(tall.slice(0, short.length), short);
  assert.ok(tall.length > short.length);
  assert.ok(tall.every((tile) => tile.size === 14));
  assert.equal(new Set(tall.map((tile) => tile.symbol)).size, 12);
  assert.deepEqual(bannerPatternTiles(0, 84, 12), []);
  assert.deepEqual(bannerPatternTiles(358, 84, 0), []);
});
test("flat balanced grids cover each row without overlaps and preserve model order", async () => {
  const { balancedGrid } = await import("../src/previews/preview-layout.ts");
  for (let count = 1; count <= 100; count++)
    for (const capacity of [1, 3, 4, 6, 8]) {
      const items = Array.from({ length: count }, (_, i) => i),
        grid = balancedGrid(items, capacity);
      assert.deepEqual(
        grid.cells.map((cell) => cell.item),
        items,
      );
      for (let row = 1; row <= grid.rows; row++) {
        const cells = grid.cells.filter((cell) => cell.row === row);
        assert.ok(cells.length <= capacity);
        assert.equal(cells[0].column, 1);
        assert.equal(
          cells.at(-1)!.column + cells.at(-1)!.span,
          grid.columns + 1,
        );
        for (let i = 1; i < cells.length; i++)
          assert.equal(
            cells[i].column,
            cells[i - 1].column + cells[i - 1].span,
          );
      }
    }
});
