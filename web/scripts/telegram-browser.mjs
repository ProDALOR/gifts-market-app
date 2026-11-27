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
import React from'react';import{createRoot}from'react-dom/client';import{SessionProvider,useSession}from'/src/app/session.tsx';import{NavigationProvider,useSharedCurrency}from'/src/app/navigation.tsx';import{ProfilePage}from'/src/profile/page.tsx';import{AppHeader}from'/src/app/header.tsx';import{useSharing}from'/src/use-sharing.tsx';import'/src/style.css';
window.events=new Map();window.nativeBack=new Set();window.colors=[];window.reopened=[];window.webOpened=[];
window.emit=name=>[...(window.events.get(name)||[])].forEach(f=>f());
window.tapBack=()=>[...window.nativeBack].forEach(f=>f());
const version=new URLSearchParams(location.search).get('client')||'9.0',throwHeader=new URLSearchParams(location.search).has('throw');
const atLeast=(value)=>{const a=version.split('.').map(Number),b=value.split('.').map(Number);return a[0]>b[0]||a[0]===b[0]&&(a[1]||0)>=(b[1]||0)};
window.app={initData:'browser-fixture',themeParams:{},colorScheme:'dark',isVersionAtLeast:atLeast,
setHeaderColor(v){if(throwHeader)throw Error('unsupported');window.colors.push(['header',v])},
setBackgroundColor(v){window.colors.push(['background',v])},setBottomBarColor(v){window.colors.push(['bottom',v])},openTelegramLink(v){window.reopened.push(v)},openLink(v){window.webOpened.push(v)},
onEvent(name,f){if(!events.has(name))events.set(name,new Set());events.get(name).add(f)},offEvent(name,f){events.get(name)?.delete(f)},
BackButton:{show(){},hide(){},onClick(f){nativeBack.add(f)},offClick(f){nativeBack.delete(f)}}};
const context=new URLSearchParams(location.search).has('web')?{initData:''}:{initData:'browser-fixture',app:window.app};
const h=React.createElement;
function Probe(){const sharing=useSharing(context,"fixture");window.shareFromApp=()=>sharing.share({type:"gift",slug:"Gift-123",currency_id:"GRAM"});window.session=useSession();const[currency,changeCurrency]=useSharedCurrency();window.changeCurrency=changeCurrency;return null;}
function View(){const[currency,changeCurrency]=useSharedCurrency();return h('div',{className:'workspace'},h('main',{className:'orderbook market-shell'},h('aside',{className:'market-sidebar'},h(AppHeader,{context,currencies:['GRAM','XTR'],currency,changeCurrency})),h('section',{className:'market-results'})))}
const root=createRoot(document.getElementById('root'));window.unmount=()=>root.unmount();root.render(h(SessionProvider,{context},h(NavigationProvider,{context},route=>h(React.Fragment,null,h(Probe),route.page==='profile'?h(ProfilePage,{context}):h(View)))));
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
          if (
            req.url.startsWith("/__session") ||
            req.url.startsWith("/profile")
          ) {
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
      signins = 0,
      identityReads = 0;
    await page.route(
      (url) => url.pathname === "/auth/me",
      (route) => {
        identityReads++;
        return route.fulfill({
          json: {
            user,
            expires_at: new Date(Date.now() + 86400000).toISOString(),
          },
        });
      },
    );
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
    return {
      page,
      calls: () => ({ configs, signins }),
      identityReads: () => identityReads,
    };
  }

  for (const viewport of [
    { width: 320, height: 740 },
    { width: 417, height: 740 },
    { width: 1280, height: 900 },
  ]) {
    const website = await open(viewport, "?web");
    const launchButton = website.page.getByRole("link", {
      name: "Open in Telegram",
      exact: true,
    });
    await launchButton.waitFor();
    const header = await website.page.evaluate(() => {
      const link = document
        .querySelector(".telegram-link")
        .getBoundingClientRect();
      const currency = document
        .querySelector(".currency-picker")
        .getBoundingClientRect();
      const icon = document
        .querySelector(".telegram-link svg")
        .getBoundingClientRect();
      return {
        width: link.width,
        height: link.height,
        iconWidth: icon.width,
        iconHeight: icon.height,
        aligned:
          Math.abs(
            link.y + link.height / 2 - currency.y - currency.height / 2,
          ) < 1,
        contained: document.documentElement.scrollWidth <= innerWidth,
      };
    });
    assert.deepEqual(header, {
      width: 36,
      height: 36,
      iconWidth: 20,
      iconHeight: 20,
      aligned: true,
      contained: true,
    });
    assert.match(
      await launchButton.getAttribute("href"),
      /^https:\/\/t\.me\/fixture_bot\?/,
    );
    assert.equal(await launchButton.getAttribute("rel"), "noreferrer");
    await website.page.close();
    const { page, calls } = await open(viewport);
    const button = page.getByRole("link", {
      name: `Profile: ${name}`,
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
    await page.getByRole("heading", { name: "Profile", exact: true }).waitFor();
    assert.equal(new URL(page.url()).pathname, "/profile");
    const websiteAction = page.getByRole("link", {
      name: "Open website",
      exact: true,
    });
    const publicProfileUrl = new URL(await websiteAction.getAttribute("href"));
    assert.equal(publicProfileUrl.origin, "https://example.test");
    assert.equal(publicProfileUrl.pathname, "/profile");
    assert.equal(publicProfileUrl.searchParams.get("currency"), "GRAM");
    await websiteAction.click();
    assert.deepEqual(await page.evaluate(() => webOpened), [
      publicProfileUrl.href,
    ]);
    assert.equal(
      await page
        .getByRole("button", { name: "Share my portfolio", exact: true })
        .count(),
      1,
    );
    assert.equal(
      await page.locator(".account-button").getAttribute("aria-current"),
      "page",
      "Profile highlights the avatar instead of either main tab",
    );
    assert.equal(
      await page.locator('.app-navigation [aria-current="page"]').count(),
      0,
    );
    assert.equal(
      await page.getByRole("heading", { name, exact: true }).count(),
      1,
    );
    assert.equal(await page.locator(".profile-card dd").innerText(), "7");
    assert.equal(
      await page.getByRole("dialog").count(),
      0,
      "profile replaces the account menu",
    );
    assert.equal(
      await page.locator(".app-navigation a").count(),
      2,
      "Market and Portfolio remain the main tabs",
    );
    assert.equal(
      await page
        .locator(".profile-content")
        .evaluate((el) => el.scrollWidth <= el.clientWidth),
      true,
      "long names stay contained",
    );
    assert.match(
      await page
        .getByRole("link", { name: "My portfolio", exact: true })
        .getAttribute("href"),
      /\/portfolio\/7\?/,
    );
    await page.screenshot({
      path: `/tmp/igifts-profile-${viewport.width}.png`,
    });
    await page.evaluate(() => tapBack());
    await page.waitForFunction(() => location.pathname === "/");
    await button.click();
    await page.getByRole("heading", { name: "Profile", exact: true }).waitFor();
    const botLink = page.getByRole("link", {
      name: "Open in bot",
      exact: true,
    });
    assert.equal(new URL(await botLink.getAttribute("href")).hostname, "t.me");
    await botLink.click();
    assert.equal(await page.evaluate(() => reopened.length), 1);
    await page.evaluate(() => shareFromApp());
    const shareLink = await page.evaluate(() => reopened.at(-1));
    assert.equal(
      new URL(new URL(shareLink).searchParams.get("url")).origin,
      "https://example.test",
      "fallback uses configured public app, never the local preview host",
    );
    await page.evaluate(() => changeCurrency("XTR"));
    await page.waitForFunction(() =>
      document.querySelector('[aria-label="Currency"]').title.includes("Stars"),
    );
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
      "profile bot links follow the current currency",
    );
    assert.equal(new URL(page.url()).searchParams.get("currency"), "XTR");
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
      ["header", "#faf7fb"],
      ["background", "#faf7fb"],
      ["bottom", "#faf7fb"],
    ]);
    await page.getByRole("button", { name: "Sign out", exact: true }).click();
    assert.equal(await page.locator(".profile-identity").count(), 0);
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

  const restored = await open({ width: 390, height: 844 });
  await restored.page.waitForFunction(() => session.status === "connected");
  await restored.page.locator(".account-button").click();
  await restored.page
    .getByRole("heading", { name: "Profile", exact: true })
    .waitFor();
  await restored.page.evaluate(() => changeCurrency("XTR"));
  await restored.page.waitForFunction(
    () => new URL(location.href).searchParams.get("currency") === "XTR",
  );
  await restored.page.reload();
  await restored.page.waitForFunction(() => session.status === "connected");
  assert.equal(new URL(restored.page.url()).pathname, "/profile");
  assert.equal(
    await restored.page.locator(".profile-card dd").innerText(),
    "7",
  );
  assert.match(
    await restored.page
      .getByRole("combobox", { name: "Currency", exact: true })
      .getAttribute("title"),
    /Stars/,
  );
  assert.deepEqual(restored.calls(), { configs: 2, signins: 1 });
  assert.equal(
    restored.identityReads(),
    1,
    "profile reload validates the existing session without signing in again",
  );
  await restored.page.close();

  const publicProfile = await open({ width: 390, height: 844 }, "?web");
  await publicProfile.page.goto(`${origin}/profile?web`);
  await publicProfile.page
    .getByText("Open the app in Telegram to view your profile.", {
      exact: true,
    })
    .waitFor();
  assert.equal(
    await publicProfile.page.locator(".profile-identity").count(),
    0,
  );
  assert.equal(
    await publicProfile.page
      .getByRole("link", { name: "My portfolio", exact: true })
      .count(),
    0,
  );
  assert.equal(publicProfile.calls().signins, 0);
  const launch = publicProfile.page
    .locator(".profile-sign-in")
    .getByRole("link", { name: "Open in Telegram", exact: true });
  await launch.waitFor();
  const launchUrl = new URL(await launch.getAttribute("href"));
  assert.equal(launchUrl.hostname, "t.me");
  assert.equal(launchUrl.pathname, "/fixture_bot");
  assert(launchUrl.searchParams.has("startapp"));
  assert.equal(
    await publicProfile.page
      .locator(".account-button")
      .getAttribute("aria-current"),
    "page",
  );
  await publicProfile.page.close();

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
  await expiry.page.getByRole("link", { name: "Profile", exact: true }).click();
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
    "Telegram browser passed: header/profile layout, long names, native Back, retry recovery, expiry relaunch, sign-out cleanup and native theme compatibility.",
  );
} finally {
  await browser?.close();
  await server.close();
  await rm(cacheDir, { recursive: true, force: true });
}
