import assert from "node:assert/strict";
import { test } from "node:test";
import {
  initialLocation,
  pathRoute,
  portfolioUser,
} from "../src/app/routes.ts";
import { encodeView } from "../src/telegram.ts";
import type { TelegramContext } from "../src/telegram.ts";
const context = (start: string): TelegramContext => ({
  initData: "",
  app: { initDataUnsafe: { start_param: start } } as TelegramContext["app"],
});
test("portfolio launch targets apply once and explicit routes override persistent Mini App start parameters", () => {
  const ctx = context("portfolio_123");
  assert.deepEqual(
    initialLocation(ctx, new URL("https://market.test/"), null).route,
    { page: "portfolio", user: 123, index: 0 },
  );
  const returned = initialLocation(
    ctx,
    new URL("https://market.test/?currency=XTR"),
    { giftsMarket: { page: "market", index: 2 } },
  );
  assert.equal(returned.route.page, "market");
  assert.equal(returned.currency, "XTR");
  assert.equal(
    initialLocation(ctx, new URL("https://market.test/portfolio/456"), null)
      .route.user,
    456,
  );
  assert.equal(
    initialLocation(ctx, new URL("https://market.test/?page=market"), null)
      .route.page,
    "market",
  );
  assert.equal(
    initialLocation(ctx, new URL("https://market.test/?collection=gift"), null)
      .route.page,
    "market",
  );
});
test("gift launches canonicalize public selection once without keeping launch credentials in URLs", () => {
  const ctx = context(
    encodeView(
      new URLSearchParams({
        gift: "Gift-123",
        collection: "gift",
        currency: "XTR",
      }),
    )!,
  );
  const first = initialLocation(
    ctx,
    new URL(
      "https://market.test/?tgWebAppData=private&tgWebAppStartParam=ignored",
    ),
    null,
  );
  assert.equal(first.route.page, "market");
  assert.match(first.href, /gift=Gift-123/);
  assert.match(first.href, /currency=XTR/);
  assert(!first.href.includes("private"));
  assert(!first.href.includes("tgWebApp"));
  const returned = initialLocation(
    ctx,
    new URL("https://market.test/?collection=gift&currency=GRAM"),
    { giftsMarket: { page: "market", index: 1 } },
  );
  assert(
    !returned.href.includes("gift=Gift-123"),
    "closing a gift must survive navigation and reload",
  );
});
test("portfolio routes and identities reject invalid or lossy identifiers", () => {
  for (const value of [
    "0",
    "-1",
    "foo",
    "9007199254740992",
    "1e3",
    " 123 ",
    true,
    null,
    {},
    [],
  ])
    assert.equal(portfolioUser(value), undefined);
  assert.equal(portfolioUser("123"), 123);
  assert.deepEqual(pathRoute("/portfolio/123"), {
    page: "portfolio",
    user: 123,
  });
  assert.equal(
    initialLocation(
      context("portfolio_0"),
      new URL("https://market.test/"),
      null,
    ).route.page,
    "market",
  );
  assert.equal(
    initialLocation(context(""), new URL("https://market.test/"), {
      giftsMarket: { page: "market", index: -1 },
    }).route.index,
    0,
  );
  const signed = { initData: "start_param=portfolio_123" };
  assert.equal(
    initialLocation(signed, new URL("https://market.test/"), null).route.user,
    123,
  );
});
