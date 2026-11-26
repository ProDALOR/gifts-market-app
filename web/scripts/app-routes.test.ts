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

test("portfolio message links restore owner currency filters and sorting while explicit routes still win", async () => {
  const {
    encodePortfolioLaunch,
    decodePortfolioLaunch,
    portfolioFiltersFromParams,
  } = await import("../src/portfolio/links.ts");
  const params = new URLSearchParams({
    user: "123",
    currency: "XTR",
    f: "c=gift|m=model|b=black|s=sun|n=42",
    sort: "position_desc",
  });
  const payload = encodePortfolioLaunch(params)!;
  assert.ok(payload.startsWith("p_"));
  assert.ok(payload.length <= 512);
  const first = initialLocation(
    context(payload),
    new URL("https://market.test/"),
    null,
  );
  assert.deepEqual(first.route, { page: "portfolio", user: 123, index: 0 });
  assert.equal(first.currency, "XTR");
  const decoded = new URL(first.href, "https://market.test").searchParams;
  const filters = portfolioFiltersFromParams(decoded);
  assert.equal(filters.collection, "gift");
  assert.deepEqual(filters.traits.models, ["model"]);
  assert.deepEqual(filters.traits.backdrops, ["black"]);
  assert.deepEqual(filters.traits.patterns, ["sun"]);
  assert.equal(filters.number, "42");
  assert.equal(filters.sort, "position_desc");
  const website = initialLocation(
    context(""),
    new URL("https://market.test/portfolio/123#" + params),
    null,
  );
  assert.equal(
    portfolioFiltersFromParams(
      new URL(website.href, "https://market.test").searchParams,
    ).sort,
    "position_desc",
  );
  assert.equal(
    initialLocation(
      context(payload),
      new URL("https://market.test/?gift=Gift-7"),
      null,
    ).route.page,
    "market",
  );
  assert.equal(
    initialLocation(
      context(payload),
      new URL("https://market.test/portfolio/456?currency=GRAM"),
      null,
    ).route.user,
    456,
  );
  assert.equal(decodePortfolioLaunch("p_" + "a".repeat(511)), null);
  assert.equal(decodePortfolioLaunch("p_!!!!"), null);
});
