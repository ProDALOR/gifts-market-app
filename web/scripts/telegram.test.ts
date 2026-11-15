import assert from "node:assert/strict";
import { test } from "node:test";
import {
  encodeView,
  decodeView,
  launchSelection,
  cleanSelection,
  telegramLink,
} from "../src/telegram.ts";
test("Telegram links restore market selection without credentials", () => {
  const params = new URLSearchParams({
    item: "tg.gift.5167939598143193218",
    currency: "GRAM",
    mode: "bid",
    sort: "price_desc",
    filters: JSON.stringify({
      "tg.number": { op: "eq", value: { type: "integer", value: "42" } },
    }),
    token: "private",
    tgWebAppData: "private",
  });
  const payload = encodeView(params)!;
  assert.ok(payload.length <= 512);
  assert.deepEqual([...decodeView(payload)!], [...cleanSelection(params)]);
  assert.ok(!decodeView(payload)!.has("token"));
  const selected = launchSelection(
    { initData: new URLSearchParams({ start_param: payload }).toString() },
    "?currency=XTR",
  );
  assert.equal(selected.get("item"), params.get("item"));
  assert.equal(selected.get("currency"), "XTR");
  const link = telegramLink(
    {
      bot_id: "42",
      bot_username: "test_market_bot",
      auth_enabled: true,
      public_url: "https://markets.example/",
    },
    params,
  )!;
  assert.ok(link.startsWith("https://t.me/test_market_bot?startapp=v_"));
  assert.ok(!link.includes("private"));
});
test("Malformed or excessive launch parameters cannot create arbitrary routes", () => {
  for (const value of ["bad", "v_%GG", "v_!", "v_" + "a".repeat(512)])
    assert.equal(decodeView(value), null);
  assert.equal(
    encodeView(new URLSearchParams({ filters: "x".repeat(1000) })),
    null,
  );
  assert.equal(
    cleanSelection(new URLSearchParams("item=one&item=two&secret=hidden")).has(
      "item",
    ),
    false,
  );
});
