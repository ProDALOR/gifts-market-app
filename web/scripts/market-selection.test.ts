import {
  marketLaunch,
  decodedMarket,
  launchParams,
} from "./navigation-helpers.ts";
import assert from "node:assert/strict";
import { test } from "node:test";
import { readFileSync } from "node:fs";
import {
  filterLine,
  selectionFromParams,
  selectionParams,
  selectionChoice,
  choiceSelection,
  selectionAt,
  selectionHref,
} from "../src/market-selection.ts";
import { telegramLink } from "../src/telegram.ts";
import { initialLocation } from "../src/app/routes.ts";
const fixtures = JSON.parse(
  readFileSync(
    new URL("../../crates/market-links/tests/codec.json", import.meta.url),
    "utf8",
  ),
) as { name: string; query: string; line: string }[];
test("Rust and TypeScript share the readable grammar, escaping and legacy fixtures", () => {
  for (const f of fixtures) {
    const s = selectionFromParams(new URLSearchParams(f.query));
    assert.equal(filterLine(s), f.line, f.name);
    assert.deepEqual(selectionFromParams(selectionParams(s)), s, f.name);
    const choice = selectionChoice(s);
    assert.deepEqual(
      choiceSelection(choice, s.gift ?? undefined).filters,
      s.filters,
    );
  }
});
test("bad expressions fail atomically instead of broadening a selection", () => {
  for (const f of [
    "m=@name:Model",
    "c=a|c=b",
    "x=a",
    "b=",
    "b=@name:A,",
    "b=@name:A|",
    "b=%GG",
    "b=%FF",
    "n=0",
    "n=..",
    "n=+1",
    "n=5..1",
    "c=a,b",
    "b=@slug:Has%20Space",
  ]) {
    assert.throws(
      () => selectionFromParams(new URLSearchParams({ f })),
      undefined,
      f,
    );
  }
  assert.throws(() => selectionFromParams(new URLSearchParams("f=&f=b%3Dx")));
  assert.throws(() =>
    selectionFromParams(
      selectionAt(new URL("https://market.example/#f=b%3Da&f=b%3Db")),
    ),
  );
});
test("large links preserve every filter without storage or API requests", () => {
  const p = new URLSearchParams({
    f: `c=@slug:Gift|m=${Array.from({ length: 300 }, (_, i) => `@name:Model%20${i}`).join(",")}|b=@name:Black%7CWhite`,
    currency: "XTR",
    mode: "bid",
    bid_match: "any",
    min: "1.25",
    max: "999",
  });
  const original = selectionFromParams(p);
  assert.equal(marketLaunch(p), null);
  const config = {
    bot_username: "market_bot",
    public_url: "https://market.example/app",
    auth_enabled: false,
  };
  const link = telegramLink(config, p)!;
  const url = new URL(link);
  assert.equal(url.origin, "https://market.example");
  assert.equal(url.search, "");
  assert(url.hash.startsWith("#f="));
  assert.deepEqual(selectionFromParams(selectionAt(url)), original);
  const initial = initialLocation({ initData: "" }, url, null);
  assert.equal(initial.route.page, "market");
  assert(initial.href.includes("#f="));
  assert.deepEqual(
    selectionFromParams(selectionAt(new URL(initial.href, url.origin))),
    original,
  );
  const href = selectionHref("/", selectionParams(original));
  assert(href.startsWith("/#f="));
});
test("explicit selections replace launched attributes as one scope", () => {
  const start = marketLaunch(
    new URLSearchParams({ f: "c=@slug:Gift|m=@name:Model", currency: "GRAM" }),
  )!;
  const merged = launchParams(
    { initData: new URLSearchParams({ start_param: start }).toString() },
    "?f=b%3D%40name%3ABlack&currency=XTR",
  );
  const s = selectionFromParams(merged);
  assert.equal(s.filters.collection_id, undefined);
  assert.deepEqual(s.filters.models, []);
  assert.deepEqual(s.filters.backdrops, ["@name:Black"]);
  assert.equal(s.filters.currency_id, "XTR");
  assert(decodedMarket(start)?.has("f"));
  const all = selectionParams(selectionFromParams(new URLSearchParams("f=")));
  assert(all.has("f"));
  const reopened = launchParams(
    { initData: new URLSearchParams({ start_param: start }).toString() },
    all.toString(),
  );
  assert.deepEqual(selectionFromParams(reopened).filters.models, []);
  assert.equal(selectionFromParams(reopened).filters.collection_id, undefined);
});
