import { selectionFromParams } from "../src/market-selection.ts";
import assert from "node:assert/strict";
import { test } from "node:test";
import {
  encodeView,
  decodeView,
  launchSelection,
  cleanSelection,
  telegramLink,
  telegramReopenLink,
} from "../src/telegram.ts";
test("Telegram links restore market selection without credentials", () => {
  const params = new URLSearchParams({
    collection: "tg.gift.5167939598143193218",
    currency: "GRAM",
    mode: "bid",
    bid_match: "all",
    sort: "price_desc",
    filters: JSON.stringify({
      models: ["gold"],
    }),
    token: "private",
    tgWebAppData: "private",
  });
  const payload = encodeView(params)!;
  assert.ok(payload.length <= 512);
  assert.deepEqual(
    selectionFromParams(decodeView(payload)!),
    selectionFromParams(cleanSelection(params)),
  );
  assert.ok(!decodeView(payload)!.has("token"));
  const selected = launchSelection(
    { initData: new URLSearchParams({ start_param: payload }).toString() },
    "?currency=XTR",
  );
  assert.equal(
    selectionFromParams(selected).filters.collection_id,
    params.get("collection"),
  );
  assert.equal(selected.get("currency"), "XTR");
  assert.equal(selected.get("bid_match"), "all");
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
  assert.throws(() =>
    cleanSelection(
      new URLSearchParams("collection=one&collection=two&secret=hidden"),
    ),
  );
});

test("gift preview deep links preserve only public gift selection", () => {
  const payload = encodeView(
    new URLSearchParams({
      collection: "c",
      currency: "GRAM",
      gift: "Gift-123",
      token: "private",
    }),
  )!;
  const result = launchSelection(
    { initData: new URLSearchParams({ start_param: payload }).toString() },
    "",
  );
  assert.equal(result.get("gift"), "Gift-123");
  assert.equal(selectionFromParams(result).filters.collection_id, "c");
  assert.equal(result.has("token"), false);
});

test("expired sign-in recovery always opens Telegram, even when selection links exceed its limit", () => {
  const config = {
    auth_enabled: true,
    bot_username: "fixture_bot",
    public_url: "https://markets.example",
  };
  const small = new URLSearchParams({ gift: "Gift-123", currency: "GRAM" });
  assert.equal(telegramReopenLink(config, small), telegramLink(config, small));
  const large = new URLSearchParams({
    collection: "gift",
    filters: JSON.stringify({
      models: Array.from(
        { length: 80 },
        (_, i) => `model-${i}-${"x".repeat(30)}`,
      ),
    }),
  });
  assert.ok(
    telegramLink(config, large)!.startsWith("https://markets.example/"),
  );
  assert.equal(
    telegramReopenLink(config, large),
    "https://t.me/fixture_bot?startapp",
  );
  assert.equal(telegramReopenLink(undefined, small), null);
});
