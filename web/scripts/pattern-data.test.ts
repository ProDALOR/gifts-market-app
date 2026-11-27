import assert from "node:assert/strict";
import { test } from "node:test";
import { loadPattern, peekPattern } from "../src/previews/pattern-data.ts";
test("rows and details share in-flight pattern decoding and tinting", async (t) => {
  let downloads = 0,
    decodes = 0,
    closes = 0,
    canvases = 0,
    tints = 0;
  const originalDocument = Object.getOwnPropertyDescriptor(
    globalThis,
    "document",
  );
  const originalBitmap = Object.getOwnPropertyDescriptor(
    globalThis,
    "createImageBitmap",
  );
  t.after(() => {
    if (originalDocument)
      Object.defineProperty(globalThis, "document", originalDocument);
    else Reflect.deleteProperty(globalThis, "document");
    if (originalBitmap)
      Object.defineProperty(globalThis, "createImageBitmap", originalBitmap);
    else Reflect.deleteProperty(globalThis, "createImageBitmap");
  });
  Object.defineProperty(globalThis, "document", {
    configurable: true,
    value: {
      createElement() {
        canvases++;
        return {
          width: 0,
          height: 0,
          getContext: () => ({
            drawImage() {},
            fillRect() {
              tints++;
            },
          }),
        };
      },
    },
  });
  Object.defineProperty(globalThis, "createImageBitmap", {
    configurable: true,
    value: async () => {
      decodes++;
      return {
        close() {
          closes++;
        },
      };
    },
  });
  t.mock.method(globalThis, "fetch", async () => {
    downloads++;
    return new Response(new Uint8Array([1]), {
      headers: { "Content-Type": "image/webp" },
    });
  });
  const signal = new AbortController().signal;
  const [first, second] = await Promise.all([
    loadPattern("/pattern-shared", "#112233", signal),
    loadPattern("/pattern-shared", "#112233", signal),
  ]);
  assert.equal(first, second);
  assert.equal(peekPattern("/pattern-shared", "#112233"), first);
  await loadPattern("/pattern-shared", "#334455", signal);
  assert.equal(downloads, 1);
  assert.equal(decodes, 1);
  assert.equal(closes, 1);
  assert.equal(canvases, 3);
  assert.equal(tints, 2);
});
