import { readFileSync } from "node:fs";
import assert from "node:assert/strict";
import { test } from "node:test";
import {
  targetFromParams,
  targetParams,
  decodeTarget,
  encodeTarget,
  targetInlineQuery,
  targetWebsiteUrl,
  targetBotUrl,
} from "../src/targets.ts";
const fixtures = JSON.parse(
  readFileSync(
    new URL("../../crates/market-links/tests/targets.json", import.meta.url),
    "utf8",
  ),
);
test("Rust and GUI use identical target queries, launches and inline text", () => {
  for (const f of fixtures) {
    const target = targetFromParams(new URLSearchParams(f.query));
    assert.equal(targetParams(target).toString(), f.query);
    assert.equal(targetInlineQuery(target), f.inline);
    assert.equal(encodeTarget(target), f.launch);
    if (f.launch)
      assert.equal(targetParams(decodeTarget(f.launch)!).toString(), f.query);
    const website = new URL(targetWebsiteUrl(target, "https://app.example/"));
    assert.equal(website.search, "");
    // Portfolio URLs carry their owner in the route, rather than the fragment.
    if (target.type !== "portfolio")
      assert.equal(
        targetParams(
          targetFromParams(new URLSearchParams(website.hash.slice(1))),
        ).toString(),
        f.query,
      );
    assert(!/version=|offset=|token=/.test(website.hash));
  }
});
test("legacy launch decoding is retained without legacy writers", () => {
  for (const [prefix, query] of [
    ["v_", "collection=gift&gift=Gift-123&currency=XTR"],
    ["p_", "user=7&currency=XTR&f=&sort=value_desc"],
  ]) {
    const target = decodeTarget(
      prefix + Buffer.from(query).toString("base64url"),
    );
    assert(target);
    assert(encodeTarget(target)?.startsWith("t_"));
  }
  assert.equal(decodeTarget("portfolio_7")?.type, "portfolio");
});
test("bot deep links dispatch short launches and preserve large drafts", () => {
  const gift = targetFromParams(new URLSearchParams({ gift: "Gift-1" })),
    url = new URL(targetBotUrl(gift, "gift_bot"));
  assert(url.searchParams.get("start"));
  assert.deepEqual(decodeTarget(url.searchParams.get("start")!), gift);
  const market = targetFromParams(
    new URLSearchParams({
      t: "market",
      f:
        "c=gift|m=" +
        Array.from({ length: 100 }, (_, i) => `model-${i}`).join(","),
    }),
  );
  const draft = new URL(targetBotUrl(market, "gift_bot"));
  assert(draft.searchParams.get("text")?.includes("model-99"));
  assert(!draft.searchParams.has("startapp"));
  assert.throws(() => targetBotUrl(gift, "bad/name"));
});

test("launch prefixes cannot hide another target type or silently drop its subject", () => {
  for (const [prefix, query] of [
    ["t_", "currency=GRAM"],
    ["p_", "gift=Gift-123"],
    ["v_", "t=portfolio&user=7"],
  ])
    assert.equal(
      decodeTarget(prefix + Buffer.from(query).toString("base64url")),
      null,
    );
});
