import assert from "node:assert/strict";
import { test } from "node:test";
import { OrderCoverage } from "../src/feed/coverage.ts";
const key = (n: number) => String(n).padStart(4, "0");
test("coverage joins adjacent and overlapping reads while keeping gaps explicit", () => {
  const c = new OrderCoverage();
  assert.deepEqual(c.gap(), { start: null, end: null });
  c.add({ start: null, end: key(10) });
  c.add({ start: key(20), end: null });
  assert.deepEqual(c.gap(), { start: key(10), end: key(20) });
  assert.equal(c.gap({ start: key(2), end: key(8) }), undefined);
  assert.deepEqual(c.gap({ start: key(15), end: key(25) }), {
    start: key(15),
    end: key(20),
  });
  c.add({ start: key(10), end: key(15) });
  assert.deepEqual(c.gap(), { start: key(15), end: key(20) });
  c.add({ start: key(14), end: key(21) });
  assert.equal(c.gap(), undefined);
  c.remove({ start: key(4), end: key(24) });
  assert.deepEqual(c.gap(), { start: key(4), end: key(24) });
  assert.equal(c.containing(key(10)), undefined);
  assert.ok(c.containing(key(24)));
  assert.equal(c.hasBefore, false);
  c.remove({ start: null, end: key(5) });
  assert.equal(c.hasBefore, true);
});
test("coverage mutations agree with independent point membership across open and bounded intervals", () => {
  const c = new OrderCoverage(),
    points = Array.from({ length: 22 }, (_, n) => n),
    covered = new Set<number>();
  let seed = 17;
  const random = () => {
    seed = (seed * 48271) % 2147483647;
    return seed;
  };
  for (let i = 0; i < 200; i++) {
    const a = random() % 22,
      b = a + (random() % (23 - a)),
      start = a === 0 ? null : key(a),
      end = b === 22 ? null : key(b);
    const add = random() % 2 === 0;
    if (add) c.add({ start, end });
    else c.remove({ start, end });
    for (const n of points) {
      if (
        (start === null || key(n) >= start) &&
        (end === null || key(n) < end)
      ) {
        if (add) covered.add(n);
        else covered.delete(n);
      }
    }
    for (const n of points)
      assert.equal(
        !!c.containing(key(n)),
        covered.has(n),
        `mutation ${i}, point ${n}`,
      );
  }
});
