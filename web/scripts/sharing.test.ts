import assert from "node:assert/strict";
import { test } from "node:test";
import { SharingController } from "../src/sharing.ts";
import type { Target } from "../src/targets.ts";
import { targetFromParams } from "../src/targets.ts";
import type { WebApp } from "../src/telegram.ts";
const gift: Target = { type: "gift", slug: "Gift-123", currency_id: "GRAM" };
test("sharing uses inline directly without signing in or preparing a message", async () => {
  const calls: { query: string; chats: string[] | undefined }[] = [];
  const app = {
    switchInlineQuery(query: string, chats?: string[]) {
      calls.push({ query, chats });
    },
  } as WebApp;
  const controller = new SharingController(app);
  await controller.share(gift);
  assert.deepEqual(calls, [
    { query: "Gift-123 GRAM", chats: ["users", "bots", "groups", "channels"] },
  ]);
  assert.deepEqual(controller.current(), { busy: false, error: "" });
});
test("portfolios share subject, filters and sort without inventory versions", async () => {
  let query = "";
  const controller = new SharingController({
    switchInlineQuery(q: string) {
      query = q;
    },
  } as WebApp);
  await controller.share({
    type: "portfolio",
    target: {
      user_id: 7,
      currency_id: "XTR",
      sort: "number_desc",
      filters: {
        collection_id: "gift",
        models: ["model"],
        backdrops: ["black"],
        patterns: ["sun"],
        number: 42,
      },
    },
  });
  assert.equal(
    query,
    "portfolio user=7 currency=XTR f=c=gift|m=model|b=black|s=sun|n=42 sort=number_desc",
  );
  assert(!/version|offset|limit/.test(query));
});
test("large selections share complete fragment URLs instead of truncating or using cached snapshots", async () => {
  const target = targetFromParams(
    new URLSearchParams({
      t: "market",
      f:
        "c=gift|m=" +
        Array.from({ length: 300 }, (_, i) => `model-${i}`).join(","),
    }),
  );
  const opened: string[] = [];
  const controller = new SharingController(
    {
      switchInlineQuery() {
        assert.fail("oversized inline");
      },
      openTelegramLink(url: string) {
        opened.push(url);
      },
    } as unknown as WebApp,
    "https://app.example/",
  );
  await controller.share(target);
  const chooser = new URL(opened[0]),
    url = new URL(chooser.searchParams.get("url")!);
  assert.equal(chooser.pathname, "/share/url");
  assert.equal(url.search, "");
  assert.equal(
    targetFromParams(new URLSearchParams(url.hash.slice(1))).type,
    "market",
  );
  assert(url.hash.includes("model-299"));
  assert.equal(controller.current().error, "");
});
test("unsupported clients and invalid targets report errors without unhandled click rejections", async () => {
  for (const app of [
    undefined,
    {
      switchInlineQuery() {
        throw Error("unsupported");
      },
    } as unknown as WebApp,
  ]) {
    const controller = new SharingController(app);
    await controller.share(gift);
    assert.ok(controller.current().error);
    assert.equal(controller.current().busy, false);
    controller.cancel();
    assert.equal(controller.current().error, "");
  }
  const controller = new SharingController({
    switchInlineQuery() {
      assert.fail("invalid target");
    },
  } as unknown as WebApp);
  await controller.share({ ...gift, slug: "bad" });
  assert.ok(controller.current().error);
});

test("an older inline bridge can fall back to a complete target URL", async () => {
  const links: string[] = [];
  const controller = new SharingController(
    {
      switchInlineQuery() {
        throw Error("unsupported");
      },
      openTelegramLink(url: string) {
        links.push(url);
      },
    } as unknown as WebApp,
    "https://app.example/",
  );
  await controller.share(gift);
  const url = new URL(new URL(links[0]).searchParams.get("url")!);
  assert.equal(new URLSearchParams(url.hash.slice(1)).get("gift"), "Gift-123");
  assert.equal(controller.current().error, "");
});

test("selections too large for a Telegram draft are rejected rather than silently truncated", async () => {
  const target = targetFromParams(
    new URLSearchParams({
      t: "market",
      f:
        "c=gift|m=" +
        Array.from(
          { length: 500 },
          (_, i) => `model-${i}-xxxxxxxxxxxxxxxxxxxx`,
        ).join(","),
    }),
  );
  const controller = new SharingController(
    {
      openTelegramLink() {
        assert.fail("would be truncated by Telegram");
      },
    } as unknown as WebApp,
    "https://app.example/",
  );
  await controller.share(target);
  assert.match(controller.current().error, /fewer attributes/);
});
