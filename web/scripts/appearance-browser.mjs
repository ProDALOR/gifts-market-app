// Real profile and shell, without inventory, account or valuation reads.
import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
import { chromium } from "playwright";
const root = fileURLToPath(new URL("..", import.meta.url)),
  cacheDir = await mkdtemp(join(tmpdir(), "igifts-appearance-"));
const html = `<!doctype html><html><head><meta name="viewport" content="width=device-width,initial-scale=1"><meta name="theme-color" content="#151419"></head><body><div id="root"></div><script type="module">
import React from'react';import{createRoot}from'react-dom/client';import{AppShell}from'/src/app/shell.tsx';import{AppLink,AppNavigation}from'/src/app/navigation.tsx';import'/src/style.css';
const h=React.createElement,events=new Map();window.native=[];window.emit=name=>events.get(name)?.forEach(fn=>fn());window.app={colorScheme:'dark',themeParams:{},isVersionAtLeast(){return true},onEvent(name,fn){if(!events.has(name))events.set(name,new Set());events.get(name).add(fn)},offEvent(name,fn){events.get(name)?.delete(fn)},setHeaderColor(c){native.push(['header',c])},setBackgroundColor(c){native.push(['background',c])},setBottomBarColor(c){native.push(['bottom',c])}};
const context=new URLSearchParams(location.search).has('browser')?{}:{app:window.app};createRoot(document.getElementById('root')).render(h(AppShell,{context,market:h('main',{className:'workspace'},h(AppLink,{page:'profile'},'Profile'),h(AppNavigation,{page:'market'}))}));
</script></body></html>`;
const server = await createServer({
  root,
  cacheDir,
  server: { host: "127.0.0.1", port: 0 },
  plugins: [
    {
      name: "appearance-fixture",
      configureServer(server) {
        server.middlewares.use(async (req, res, next) => {
          if (!["/profile", "/"].includes(req.url.split("?")[0])) return next();
          res.setHeader("Content-Type", "text/html");
          res.end(await server.transformIndexHtml(req.url, html));
        });
      },
    },
  ],
});
const contrast = (a, b) => {
  const luminance = (color) => {
    const parts = color.startsWith("#")
      ? [1, 3, 5].map((i) => parseInt(color.slice(i, i + 2), 16))
      : color
          .match(/[\d.]+/g)
          .slice(0, 3)
          .map(Number);
    const rgb = parts.map((v) => {
      v /= 255;
      return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
    });
    return rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722;
  };
  const x = luminance(a),
    y = luminance(b);
  return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
};
const title = (s) => s[0].toUpperCase() + s.slice(1);
async function prepare(page) {
  await page.route("**/telegram", (r) =>
    r.fulfill({
      json: { auth_enabled: false, bot_id: null, bot_username: null },
    }),
  );
  await page.route("**/media/**", (r) =>
    r.fulfill({
      contentType: "image/svg+xml",
      body: '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><circle cx="12" cy="12" r="9"/></svg>',
    }),
  );
}
let browser;
try {
  await server.listen();
  browser = await chromium.launch({ headless: true });
  const url = server.resolvedUrls.local[0];
  for (const width of [280, 320, 390, 959, 960, 1440]) {
    const context = await browser.newContext({
        viewport: { width, height: 1000 },
        colorScheme: "light",
        reducedMotion: "reduce",
      }),
      page = await context.newPage(),
      errors = [],
      reads = [];
    page.on("pageerror", (e) => errors.push(e.message));
    page.on("request", (r) => {
      if (
        /^\/(catalog|collections|orders|valuations|portfolios|auth)(\/|$)/.test(
          new URL(r.url()).pathname,
        )
      )
        reads.push(r.url());
    });
    await prepare(page);
    await page.goto(url + "profile");
    await page.getByRole("radio", { name: "Rose", exact: true }).waitFor();
    assert.equal(
      await page.getByText("Saved on this device.", { exact: true }).count(),
      1,
    );
    assert.equal(
      await page.locator(".account-button").getAttribute("aria-current"),
      "page",
    );
    const targets = await page
      .locator(".display-mode-options input")
      .evaluateAll((inputs) =>
        inputs.map((input) => input.getBoundingClientRect().height),
      );
    assert(
      targets.every((height) => height >= 44),
      "Mode controls have 44px touch targets",
    );
    const enlarged = await page.addStyleTag({
      content:
        ".profile-content{font-size:20px}.palette-option{font-size:18px}",
    });
    assert.equal(
      await page
        .locator(".display-mode-options label, .palette-option")
        .evaluateAll((labels) =>
          labels.every((label) => label.scrollWidth <= label.clientWidth),
        ),
      true,
      `${width}px enlarged setting labels remain contained`,
    );
    await enlarged.evaluate((style) => style.remove());
    if (width < 960) {
      await page.getByRole("radio", { name: "Rose", exact: true }).focus();
      await page.evaluate(() => {
        Object.defineProperty(visualViewport, "height", {
          configurable: true,
          get: () => innerHeight - 200,
        });
        visualViewport.dispatchEvent(new Event("resize"));
      });
      await page.evaluate(
        () =>
          new Promise((resolve) =>
            requestAnimationFrame(() => requestAnimationFrame(resolve)),
          ),
      );
      assert.equal(
        await page
          .locator(".app-navigation")
          .evaluate((nav) => getComputedStyle(nav).display !== "none"),
        true,
        "Radio focus must not be mistaken for a keyboard",
      );
      await page.evaluate(() => {
        const input = document.createElement("input");
        input.id = "keyboard-probe";
        document.body.append(input);
        input.focus();
      });
      await page.waitForFunction(() =>
        document
          .querySelector(".app-navigation")
          .classList.contains("keyboard-open"),
      );
      await page.evaluate(() => {
        const input = document.getElementById("keyboard-probe");
        input.blur();
        input.readOnly = true;
        input.focus();
      });
      await page.waitForFunction(
        () =>
          !document
            .querySelector(".app-navigation")
            .classList.contains("keyboard-open"),
      );
      await page.evaluate(() => {
        document.getElementById("keyboard-probe").remove();
        Reflect.deleteProperty(visualViewport, "height");
        visualViewport.dispatchEvent(new Event("resize"));
      });
    }
    assert.equal(
      await page.locator("html").getAttribute("data-color-scheme"),
      "dark",
      "Auto follows Telegram before the device",
    );
    for (const mode of ["dark", "light"]) {
      await page.getByRole("radio", { name: title(mode), exact: true }).check();
      for (const palette of ["rose", "ocean", "mint", "amber"]) {
        await page
          .getByRole("radio", { name: title(palette), exact: true })
          .check();
        const colors = await page.evaluate(() => {
          const s = getComputedStyle(document.documentElement);
          const nav = getComputedStyle(
            document.querySelector(".app-navigation"),
          );
          return {
            vars: Object.fromEntries(
              [
                "paper",
                "surface",
                "raised",
                "ink",
                "muted",
                "accent",
                "selection",
                "on-selection",
                "on-accent",
              ].map((v) => [v, s.getPropertyValue("--" + v).trim()]),
            ),
            scheme: document.documentElement.dataset.colorScheme,
            palette: document.documentElement.dataset.palette,
            overflow: document.documentElement.scrollWidth > innerWidth,
            native: window.native.slice(-3),
            meta: document.querySelector('meta[name="theme-color"]').content,
            navigation: {
              background: nav.backgroundColor,
              blur: nav.backdropFilter,
              opacity: nav.opacity,
            },
          };
        });
        assert.equal(colors.scheme, mode);
        assert.equal(colors.palette, palette);
        assert.equal(
          colors.overflow,
          false,
          `${width}px ${palette} ${mode} overflow`,
        );
        for (const bg of ["paper", "surface", "raised"])
          for (const fg of ["ink", "muted"])
            assert(
              contrast(colors.vars[fg], colors.vars[bg]) >= 4.5,
              `${palette} ${mode} ${fg}/${bg}`,
            );
        assert(
          contrast(colors.vars.accent, colors.vars.surface) >= 4.5,
          `${palette} ${mode} accent`,
        );
        assert(
          contrast(colors.vars["on-selection"], colors.vars.selection) >= 4.5,
          `${palette} selected control`,
        );
        assert(
          contrast(colors.vars["on-accent"], colors.vars.accent) >= 4.5,
          `${palette} primary action`,
        );
        assert.deepEqual(colors.native, [
          ["header", colors.vars.paper],
          ["background", colors.vars.paper],
          ["bottom", colors.vars.paper],
        ]);
        assert.equal(colors.meta, colors.vars.paper);
        assert.equal(
          colors.navigation.opacity,
          "1",
          "Blur/tint must not fade the buttons",
        );
        assert.equal(
          colors.navigation.blur,
          width < 960 ? "blur(8px)" : "none",
        );
        if (width < 960) {
          const alpha = Number(
            colors.navigation.background.match(/\/\s*([\d.]+)/)?.[1],
          );
          assert.equal(alpha, mode === "light" ? 0.9 : 0.85);
          const paper = [1, 3, 5].map((i) =>
            parseInt(colors.vars.paper.slice(i, i + 2), 16),
          );
          for (const underneath of [0, 255]) {
            const mixed = `rgb(${paper.map((channel) => channel * alpha + underneath * (1 - alpha)).join(",")})`;
            assert(
              contrast(colors.vars.muted, mixed) >= 4.5,
              `${palette} ${mode} navigation stays readable over bright/dark gifts`,
            );
          }
        }
        await page
          .locator(".app-navigation")
          .getByRole("link", { name: "Market", exact: true })
          .click();
        const active = page.locator('.app-navigation [aria-current="page"]');
        const text = await active.evaluate((el) => getComputedStyle(el).color);
        await active.hover();
        assert.equal(
          await active.evaluate((el) => getComputedStyle(el).color),
          text,
          `${palette} ${mode} active hover retains its contrast`,
        );
        await page.getByRole("link", { name: "Profile", exact: true }).click();
        assert.equal(
          await page.locator(".account-button").getAttribute("aria-current"),
          "page",
        );
      }
    }
    await page.reload();
    assert(
      await page.getByRole("radio", { name: "Amber", exact: true }).isChecked(),
    );
    assert(
      await page.getByRole("radio", { name: "Light", exact: true }).isChecked(),
    );
    const before = await page.evaluate(() => native.length);
    await page.evaluate(() => {
      emit("themeChanged");
      emit("viewportChanged");
    });
    assert.equal(
      await page.evaluate(() => native.length),
      before,
      "Unchanged palette does not resend native colors",
    );
    const other = await context.newPage();
    await prepare(other);
    await other.goto(url + "profile");
    await other.getByRole("radio", { name: "Ocean", exact: true }).check();
    await page.waitForFunction(
      () => document.documentElement.dataset.palette === "ocean",
    );
    await page.getByRole("radio", { name: "Auto", exact: true }).check();
    await page.waitForFunction(
      () => document.documentElement.dataset.colorScheme === "dark",
    );
    assert.deepEqual(errors, []);
    assert.deepEqual(
      reads,
      [],
      "Appearance makes no account or market data requests",
    );
    if (width < 960) {
      const client = await context.newCDPSession(page);
      await client.send("Emulation.setEmulatedMedia", {
        features: [{ name: "prefers-reduced-transparency", value: "reduce" }],
      });
      assert.equal(
        await page.evaluate(
          () => matchMedia("(prefers-reduced-transparency: reduce)").matches,
        ),
        true,
      );
      const opaqueNavigation = () =>
        page.locator(".app-navigation").evaluate((nav) => {
          const styles = getComputedStyle(nav);
          return (
            styles.backdropFilter === "none" &&
            styles.backgroundColor ===
              getComputedStyle(document.documentElement).backgroundColor
          );
        });
      assert.equal(
        await opaqueNavigation(),
        true,
        "Reduced transparency disables the glass effect",
      );
      await client.send("Emulation.setEmulatedMedia", { features: [] });
      assert.equal(
        await page
          .locator(".app-navigation")
          .evaluate((nav) => getComputedStyle(nav).backdropFilter),
        "blur(8px)",
      );
      const removed = await page.evaluate(() => {
        let count = 0;
        const removeSupportedBranch = (sheet) => {
          for (let index = sheet.cssRules.length - 1; index >= 0; index--) {
            const rule = sheet.cssRules[index];
            if (
              rule.type === CSSRule.SUPPORTS_RULE &&
              rule.conditionText.includes("backdrop-filter")
            ) {
              sheet.deleteRule(index);
              count++;
            } else if (rule.cssRules) removeSupportedBranch(rule);
          }
        };
        for (const sheet of document.styleSheets) removeSupportedBranch(sheet);
        return count;
      });
      assert(removed > 0);
      assert.equal(
        await opaqueNavigation(),
        true,
        "Browsers without backdrop blur retain a solid navigation surface",
      );
    }
    await context.close();
  }
  const page = await browser.newPage({
    viewport: { width: 390, height: 1000 },
    colorScheme: "light",
  });
  await prepare(page);
  await page.goto(url + "profile?browser");
  await page.getByRole("radio", { name: "Auto", exact: true }).waitFor();
  assert.equal(
    await page.locator("html").getAttribute("data-color-scheme"),
    "light",
  );
  await page.emulateMedia({ colorScheme: "dark" });
  await page.waitForFunction(
    () => document.documentElement.dataset.colorScheme === "dark",
  );
  await page.getByRole("radio", { name: "Light", exact: true }).check();
  await page.emulateMedia({ colorScheme: "light" });
  assert.equal(
    await page.locator("html").getAttribute("data-color-scheme"),
    "light",
  );
  await page.close();
  const blocked = await browser.newPage({
    viewport: { width: 390, height: 1000 },
  });
  await blocked.addInitScript(() => {
    Storage.prototype.setItem = () => {
      throw new Error("Storage disabled");
    };
  });
  await prepare(blocked);
  await blocked.goto(url + "profile");
  await blocked.getByRole("radio", { name: "Mint", exact: true }).check();
  await blocked.getByRole("radio", { name: "Dark", exact: true }).check();
  await blocked
    .locator(".app-navigation")
    .getByRole("link", { name: "Market", exact: true })
    .click();
  await blocked.getByRole("link", { name: "Profile", exact: true }).click();
  assert(
    await blocked.getByRole("radio", { name: "Mint", exact: true }).isChecked(),
  );
  assert(
    await blocked.getByRole("radio", { name: "Dark", exact: true }).isChecked(),
  );
  await blocked.close();
  console.log(
    "Appearance passed: four palettes, dark/light and translucent navigation contrast, solid blur fallbacks, 280–1440px, large text, radio/text keyboard states, Telegram and device Auto, native colors, persistence, cross-tab updates, blocked storage and no data requests.",
  );
} finally {
  await browser?.close();
  await server.close();
  await rm(cacheDir, { recursive: true, force: true });
}
