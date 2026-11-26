import assert from "node:assert/strict";
import { test } from "node:test";
import { StillImageCache } from "../src/previews/still-image-cache.ts";
function image(width = 10, height = 10, complete = true) {
  const clones: { loading: string; removed: string[] }[] = [];
  const source = {
    naturalWidth: width,
    naturalHeight: height,
    complete,
    cloneNode() {
      const clone = {
        loading: "lazy",
        removed: [] as string[],
        removeAttribute(name: string) {
          this.removed.push(name);
        },
      };
      clones.push(clone);
      return clone;
    },
  } as unknown as HTMLImageElement;
  return { source, clones };
}
test("decoded image retention obeys both byte and entry budgets and releases LRU resources", () => {
  const cache = new StillImageCache(800, 3);
  const a = image(),
    b = image(),
    c = image();
  cache.remember("a", a.source);
  cache.remember("b", b.source);
  assert.ok(cache.has("a"));
  cache.remember("c", c.source);
  assert.ok(cache.has("a"));
  assert.ok(cache.has("c"));
  assert.equal(cache.has("b"), false);
  assert.ok(b.clones[0].removed.includes("src"));
  const one = new StillImageCache(4000, 1);
  one.remember("a", a.source);
  one.remember("b", b.source);
  assert.equal(one.has("a"), false);
  assert.ok(one.has("b"));
});
test("retained images are detached clones; duplicates, incomplete and oversized sources add no work", () => {
  const cache = new StillImageCache(400);
  const a = image();
  cache.remember("a", a.source);
  cache.remember("a", a.source);
  assert.equal(a.clones.length, 1);
  assert.equal(a.clones[0].loading, "eager");
  assert.deepEqual(a.clones[0].removed, ["class", "style"]);
  const incomplete = image(10, 10, false),
    invalid = image(0, 10),
    huge = image(100, 100);
  for (const [key, value] of [
    ["incomplete", incomplete],
    ["invalid", invalid],
    ["huge", huge],
  ] as const) {
    cache.remember(key, value.source);
    assert.equal(cache.has(key), false);
    assert.equal(value.clones.length, 0);
  }
  cache.forget("a");
  cache.forget("a");
  assert.equal(cache.has("a"), false);
  assert.ok(a.clones[0].removed.includes("src"));
});
