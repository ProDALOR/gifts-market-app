import { portfolioLaunch, decodedPortfolio } from "./navigation-helpers.ts";
import assert from "node:assert/strict";
import { test } from "node:test";
import {
  portfolioFiltersFromParams,
  portfolioParams,
} from "../src/portfolio/links.ts";
import { targetParams, targetFromParams } from "../src/targets.ts";
import { telegramLink } from "../src/telegram.ts";
test("portfolio links use the shared readable filter codec and fall back to a stateless website URL", () => {
  const filters = {
    collection: "gift",
    traits: { models: ["model"], backdrops: ["black"], patterns: ["sun"] },
    number: "42",
    sort: "number_desc" as const,
  };
  const params = portfolioParams(filters, "XTR");
  params.set("user", "123");
  params.set("page", "portfolio");
  const config = {
    auth_enabled: true,
    bot_username: "gift_bot",
    public_url: "https://app.example",
  };
  const link = telegramLink(config, params)!;
  assert.equal(new URL(link).hostname, "t.me");
  const decoded = decodedPortfolio(
    new URL(link).searchParams.get("startapp")!,
  )!;
  assert.deepEqual(portfolioFiltersFromParams(decoded), filters);
  const large = portfolioParams(
    {
      ...filters,
      traits: { models: Array.from({ length: 100 }, (_, n) => `model-${n}`) },
    },
    "GRAM",
  );
  large.set("user", "123");
  large.set("page", "portfolio");
  assert.equal(portfolioLaunch(large), null);
  const website = new URL(telegramLink(config, large)!);
  assert.equal(website.hostname, "app.example");
  assert.equal(website.pathname, "/portfolio/123");
  const selection = portfolioFiltersFromParams(
    new URLSearchParams(website.hash.slice(1)),
  );
  assert.equal(selection.traits.models?.length, 100);
  assert.equal(selection.sort, "number_desc");
  assert(!website.toString().includes("version="));
});
test("malformed and ambiguous portfolio links cannot create valid launch payloads", () => {
  for (const user of ["0", "-1", "9007199254740992", "1e3"]) {
    assert.equal(portfolioLaunch(new URLSearchParams({ user })), null);
  }
  for (const query of [
    "user=123&currency=GRAM&f=&sort=unknown",
    "user=123&currency=GRAM&f=&f=&sort=value_desc",
    "user=123&user=456&currency=GRAM&f=&sort=value_desc",
    "user=123&currency=GRAM&f=&sort=value_desc&sort=number_desc",
  ]) {
    const payload = "p_" + Buffer.from(query).toString("base64url");
    assert.equal(decodedPortfolio(payload), null);
  }
});

test("Main Mini App return links do not need a separate public origin", () => {
  const params = new URLSearchParams({
    page: "portfolio",
    user: "123",
    currency: "XTR",
    f: "",
    sort: "value_desc",
  });
  const url = telegramLink(
    { bot_username: "gift_bot", auth_enabled: true },
    params,
  )!;
  assert.equal(new URL(url).hostname, "t.me");
  assert.ok(decodedPortfolio(new URL(url).searchParams.get("startapp")!));
});

test("portfolio launch encoding respects the exact 512-character base64url boundary", () => {
  const found = new Set<number>();
  for (let n = 1; n <= 180; n++) {
    const params = portfolioParams(
      {
        collection: "gift",
        traits: { models: ["a".repeat(200), "b".repeat(n)] },
        number: "",
        sort: "position_asc",
      },
      "GRAM",
    );
    params.set("user", "123");
    const bytes = new TextEncoder().encode(
      targetParams(targetFromParams(params)).toString(),
    ).length;
    if (bytes !== 382 && bytes !== 383) continue;
    found.add(bytes);
    const payload = portfolioLaunch(params);
    if (bytes === 382) {
      assert.equal(payload?.length, 512);
      assert.ok(decodedPortfolio(payload!));
    } else assert.equal(payload, null);
  }
  assert.deepEqual([...found], [382, 383]);
});
