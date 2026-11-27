// Real header/session React integration. All credentials and API responses are synthetic.
import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
import { chromium } from "playwright";

const root = fileURLToPath(new URL("..", import.meta.url));
const cacheDir = await mkdtemp(join(tmpdir(), "igifts-session-"));
const html = `<!doctype html><html><head><meta name="viewport" content="width=device-width,initial-scale=1"></head><body><div id="root"></div><script type="module">
import React,{useState}from'react';import{createRoot}from'react-dom/client';import{SessionProvider,useSession}from'/src/app/session.tsx';import{NavigationProvider}from'/src/app/navigation.tsx';import{AppHeader}from'/src/app/header.tsx';import{useSharing}from'/src/use-sharing.tsx';import'/src/style.css';
window.events=new Map();window.nativeBack=new Set();window.colors=[];window.reopened=[];
window.emit=name=>[...(window.events.get(name)||[])].forEach(f=>f());
window.tapBack=()=>[...window.nativeBack].forEach(f=>f());
const version=new URLSearchParams(location.search).get('client')||'9.0',throwHeader=new URLSearchParams(location.search).has('throw');
const atLeast=(value)=>{const a=version.split('.').map(Number),b=value.split('.').map(Number);return a[0]>b[0]||a[0]===b[0]&&(a[1]||0)>=(b[1]||0)};
window.app={initData:'browser-fixture',themeParams:{},colorScheme:'dark',isVersionAtLeast:atLeast,
setHeaderColor(v){if(throwHeader)throw Error('unsupported');window.colors.push(['header',v])},
setBackgroundColor(v){window.colors.push(['background',v])},setBottomBarColor(v){window.colors.push(['bottom',v])},openTelegramLink(v){window.reopened.push(v)},
onEvent(name,f){if(!events.has(name))events.set(name,new Set());events.get(name).add(f)},offEvent(name,f){events.get(name)?.delete(f)},
BackButton:{show(){},hide(){},onClick(f){nativeBack.add(f)},offClick(f){nativeBack.delete(f)}}};
const context={initData:'browser-fixture',app:window.app};
const h=React.createElement;
function View(){const sharing=useSharing(context,"fixture");window.shareFromApp=()=>sharing.share({type:"gift",slug:"Gift-123",currency_id:"GRAM"});const session=useSession();window.session=session;const[open,setOpen]=useState(false),[currency,changeCurrency]=useState('GRAM');window.changeCurrency=changeCurrency;return h('div',{className:'workspace'},h('main',{className:'orderbook market-shell'},h('aside',{className:'market-sidebar'},h(AppHeader,{context,currencies:['GRAM','XTR'],currency,changeCurrency,accountOpen:open,setAccountOpen:setOpen}),h('button',{style:{position:'fixed',bottom:12,left:12},id:'outside'},'Outside'),h('output',null,session.status)),h('section',{className:'market-results'})))}
const root=createRoot(document.getElementById('root'));window.unmount=()=>root.unmount();root.render(h(SessionProvider,{context},h(NavigationProvider,{context},()=>h(View))));
</script></body></html>`;
const server = await createServer({
  root,
  cacheDir,
  define: { "import.meta.env.VITE_MARKET_MINIAPP": JSON.stringify("true") },
  server: { host: "127.0.0.1", port: 0 },
  plugins: [
    {
      name: "telegram-session-fixture",
      configureServer(server) {
        server.middlewares.use(async (req, res, next) => {
          if (req.url.startsWith("/__session")) {
            res.setHeader("content-type", "text/html");
            res.end(await server.transformIndexHtml(req.url, html));
          } else next();
        });
      },
    },
  ],
});
let browser;
try {
  await server.listen();
  const origin = server.resolvedUrls.local[0].replace(/\/$/, "");
  browser = await chromium.launch({ headless: true });
  const config = {
    bot_id: "42",
    bot_username: "fixture_bot",
    public_url: "https://example.test",
    auth_enabled: true,
  };
  const name = "🔥 Avery" + "LongName".repeat(20);
  const user = {
    id: "7",
    first_name: name,
    last_name: null,
    username: "fixture_user",
  };
  async function open(viewport, query = "", options = {}) {
    const page = await browser.newPage({ viewport, reducedMotion: "reduce" });
    await page.clock.install();
    let configs = 0,
      signins = 0;
    await page.route(
      (url) => url.pathname === "/telegram",
      (r) => {
        configs++;
        return r.fulfill({
          json:
            options.notReady && configs === 1
              ? { auth_enabled: false }
              : config,
        });
      },
    );
    await page.route(
      (url) => url.pathname === "/auth/telegram",
      (r) => {
        signins++;
        return r.fulfill({
          json: {
            user,
            token: "browser-fixture-token",
            expires_at: new Date(
              Date.now() + (options.expiring ? 3000 : 86400000),
            ).toISOString(),
          },
        });
      },
    );
    await page.route(
      (url) => url.pathname.startsWith("/media/"),
      (r) =>
        r.fulfill({
          contentType: "image/svg+xml",
          body: '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><circle cx="12" cy="12" r="9" fill="#54c9ff"/></svg>',
        }),
    );
    await page.goto(`${origin}/__session${query}`);
    return { page, calls: () => ({ configs, signins }) };
  }

  for (const viewport of [
    { width: 417, height: 740 },
    { width: 1280, height: 900 },
  ]) {
    const { page, calls } = await open(viewport);
    const button = page.getByRole("button", {
      name: `Account: ${name}`,
      exact: true,
    });
    await button.waitFor();
    assert.equal(await page.locator(".account-initial").innerText(), "A");
    assert.equal(await page.locator(".account-bar").count(), 0);
    const bounds = await page.evaluate(() => {
      const rect = (s) => {
        const r = document.querySelector(s).getBoundingClientRect();
        return { y: r.y, height: r.height };
      };
      return {
        profile: rect(".account-button"),
        currency: rect(".currency-picker"),
        heading: rect(".sidebar-heading"),
      };
    });
    assert.ok(
      Math.abs(
        bounds.profile.y +
          bounds.profile.height / 2 -
          bounds.currency.y -
          bounds.currency.height / 2,
      ) < 1,
    );
    assert.equal(bounds.heading.height, 44);
    assert.deepEqual(calls(), { configs: 1, signins: 1 });
    await button.click();
    let menu = page.getByRole("dialog", { name: "Telegram account" });
    await menu.waitFor();
    const dimensions = await menu.boundingBox();
    assert.ok(
      dimensions.x >= 0 && dimensions.x + dimensions.width <= viewport.width,
    );
    assert.equal(
      await menu.evaluate((e) => e.scrollWidth <= e.clientWidth),
      true,
      "long names stay contained",
    );
    await page.keyboard.press("Escape");
    await menu.waitFor({ state: "detached" });
    await button.focus();
    assert.equal(
      await button.evaluate((e) => e === document.activeElement),
      true,
    );
    await button.click();
    await page.getByRole("button", { name: "Outside", exact: true }).click();
    await menu.waitFor({ state: "detached" });
    assert.equal(
      await page
        .locator("#outside")
        .evaluate((e) => e === document.activeElement),
      true,
    );
    await button.click();
    await page.evaluate(() => tapBack());
    await menu.waitFor({ state: "detached" });
    await button.click();
    await menu.focus();
    const controls = await menu
      .locator("a[href],button:not([disabled])")
      .count();
    for (let i = 0; i <= controls; i++) await page.keyboard.press("Tab");
    await menu.waitFor({ state: "detached" });
    await button.click();
    const botLink = page.getByRole("link", {
      name: "Open in bot",
      exact: true,
    });
    assert.equal(new URL(await botLink.getAttribute("href")).hostname, "t.me");
    await botLink.click();
    await menu.waitFor({ state: "detached" });
    assert.equal(await page.evaluate(() => reopened.length), 1);
    await page.evaluate(() => shareFromApp());
    const shareLink = await page.evaluate(() => reopened.at(-1));
    assert.equal(
      new URL(new URL(shareLink).searchParams.get("url")).origin,
      "https://example.test",
      "fallback uses configured public app, never the local preview host",
    );
    await page.evaluate(() => changeCurrency("XTR"));
    await button.click();
    const changed = new URL(
      await page
        .getByRole("link", { name: "Open in bot", exact: true })
        .getAttribute("href"),
    );
    const draft = changed.searchParams.get("text");
    const launch = changed.searchParams.get("start");
    const params = launch
      ? new URLSearchParams(
          Buffer.from(launch.slice(2), "base64url").toString(),
        )
      : new URLSearchParams();
    assert(
      draft?.includes("currency=XTR") || params.get("currency") === "XTR",
      "header links follow current state before URL effects run",
    );
    await page.keyboard.press("Escape");
    await page.evaluate(() => changeCurrency("GRAM"));
    assert.deepEqual(await page.evaluate(() => colors), [
      ["header", "#151419"],
      ["background", "#151419"],
      ["bottom", "#151419"],
    ]);
    await page.evaluate(() => {
      app.safeAreaInset = { top: 3 };
      app.contentSafeAreaInset = { top: 5 };
      emit("viewportChanged");
      emit("safeAreaChanged");
    });
    assert.equal(await page.evaluate(() => colors.length), 3);
    assert.equal(
      await page.evaluate(() =>
        document.documentElement.style.getPropertyValue("--tg-safe-top"),
      ),
      "8px",
    );
    await page.evaluate(() => {
      app.colorScheme = "light";
      app.themeParams = { bg_color: "#fafafa" };
      emit("themeChanged");
    });
    assert.deepEqual(await page.evaluate(() => colors.slice(-3)), [
      ["header", "#fafafa"],
      ["background", "#fafafa"],
      ["bottom", "#fafafa"],
    ]);
    await button.click();
    await page.getByRole("button", { name: "Sign out", exact: true }).click();
    await page.waitForFunction(() => session.status === "signed_out");
    await page.clock.fastForward(60000);
    assert.deepEqual(calls(), { configs: 1, signins: 1 });
    await page.evaluate(() => unmount());
    assert.equal(
      await page.evaluate(
        () =>
          [...events.values()].reduce((sum, values) => sum + values.size, 0) +
          nativeBack.size,
      ),
      0,
    );
    await page.close();
  }

  const recovery = await open({ width: 375, height: 740 }, "", {
    notReady: true,
  });
  await recovery.page.waitForFunction(() => session.status === "retrying");
  assert.equal(await recovery.page.locator(".account-button").count(), 1);
  await recovery.page.clock.fastForward(1000);
  await recovery.page.waitForFunction(() => session.status === "connected");
  assert.deepEqual(recovery.calls(), { configs: 2, signins: 1 });
  await recovery.page.close();

  const expiry = await open({ width: 417, height: 740 }, "", {
    expiring: true,
  });
  await expiry.page.waitForFunction(() => session.status === "connected");
  await expiry.page.clock.fastForward(5000);
  await expiry.page.waitForFunction(() => session.status === "reopen");
  await expiry.page
    .getByRole("button", { name: "Telegram sign-in", exact: true })
    .click();
  assert.equal(
    await expiry.page
      .getByRole("button", { name: "Sign in", exact: true })
      .count(),
    0,
  );
  await expiry.page
    .getByRole("link", { name: "Reopen in Telegram", exact: true })
    .click();
  assert.match(
    await expiry.page.evaluate(() => reopened[0]),
    /^https:\/\/t.me\/fixture_bot\?startapp=t_/,
  );
  assert.deepEqual(expiry.calls(), { configs: 1, signins: 1 });
  await expiry.page.close();

  const legacy = await open({ width: 375, height: 740 }, "?client=6.4");
  await legacy.page.waitForFunction(() => session.status === "connected");
  assert.deepEqual(await legacy.page.evaluate(() => colors), [
    ["header", "bg_color"],
    ["background", "#151419"],
  ]);
  await legacy.page.close();
  const unsupported = await open({ width: 375, height: 740 }, "?throw=1");
  await unsupported.page.waitForFunction(() => session.status === "connected");
  assert.deepEqual(await unsupported.page.evaluate(() => colors), [
    ["background", "#151419"],
    ["bottom", "#151419"],
  ]);
  await unsupported.page.close();
  console.log(
    "Telegram browser passed: header layout, long names, dismissal/focus/native Back, retry recovery, expiry relaunch, sign-out cleanup and native theme compatibility.",
  );
} finally {
  await browser?.close();
  await server.close();
  await rm(cacheDir, { recursive: true, force: true });
}
