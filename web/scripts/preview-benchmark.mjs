// Compare static images from real cached TGS files. No hub, credentials or runtime converter.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFile, readdir, mkdir, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import { cpus } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { gzipSync, gunzipSync } from "node:zlib";
import { chromium } from "playwright";
import { installPreviewMetrics } from "./preview-metrics.mjs";

const root = fileURLToPath(new URL("../..", import.meta.url));
const args = process.argv.slice(2);
const option = (name, fallback) => {
  const index = args.indexOf(name);
  return index < 0 ? fallback : args[index + 1];
};
const media = path.resolve(
  root,
  option("--media-dir", path.join(root, "data/live-hub/media")),
);
const output = path.resolve(
  root,
  option("--output", path.join(root, ".market/run/preview-benchmark")),
);
const count = Number(option("--samples", "40"));
const runs = Number(option("--runs", "7"));
const cpuRate = Number(option("--cpu-rate", "1"));
const corpusPath = option("--corpus", undefined);
const corpus = corpusPath
  ? JSON.parse(await readFile(path.resolve(root, corpusPath), "utf8"))
  : undefined;
if (corpus)
  assert.ok(
    Array.isArray(corpus.gifts) && corpus.gifts.length >= 2,
    "A corpus requires at least two gifts",
  );
assert.ok(
  Number.isInteger(count) &&
    count >= 2 &&
    Number.isInteger(runs) &&
    runs >= 3 &&
    Number.isFinite(cpuRate) &&
    cpuRate >= 1,
);
await mkdir(output, { recursive: true });
const candidates = [];
for (const name of (await readdir(media))
  .filter((name) => name.endsWith(".tgs"))
  .sort()) {
  const bytes = await readFile(path.join(media, name));
  const json = gunzipSync(bytes);
  const data = JSON.parse(json);
  candidates.push({
    id: name.slice(0, -4),
    jsonBytes: json.length,
    composition: data.nm ?? null,
  });
}
assert.ok(
  candidates.length >= 2,
  "Supply a media cache containing real TGS files",
);
candidates.sort(
  (a, b) => a.jsonBytes - b.jsonBytes || a.id.localeCompare(b.id),
);
// Uniform complexity quantiles include both extremes; opacity-gradient regressions are explicit.
const selected = new Map();
if (corpus) {
  for (const gift of corpus.gifts) {
    const row = candidates.find((row) => row.id === gift.id);
    assert.ok(row, `Missing pinned gift: ${gift.id}`);
    selected.set(row.id, { ...row, expectedSha256: gift.sourceSha256 });
  }
} else {
  for (let i = 0; i < Math.min(count, candidates.length); i++) {
    const row =
      candidates[
        Math.round(
          (i * (candidates.length - 1)) /
            (Math.min(count, candidates.length) - 1),
        )
      ];
    selected.set(row.id, row);
  }
  for (const row of candidates) {
    if (/brick|hot video card/i.test(row.composition ?? ""))
      selected.set(row.id, row);
  }
}
const assets = new Map();
const server = createServer((request, response) => {
  const pathname = new URL(request.url, "http://localhost").pathname;
  if (pathname === "/") {
    response.setHeader("Content-Type", "text/html");
    response.end(
      '<!doctype html><style>body{margin:0;background:transparent}#host{width:512px;height:512px}img{object-fit:contain}</style><div id="host"></div><div id="grid"></div>',
    );
    return;
  }
  const asset = assets.get(pathname);
  if (!asset) {
    response.writeHead(404).end();
    return;
  }
  response.setHeader("Content-Type", asset.type);
  response.setHeader("Cache-Control", "public,max-age=3600");
  if (asset.type === "image/svg+xml")
    response.setHeader("Content-Encoding", "gzip");
  response.end(asset.bytes);
});
await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
const origin = `http://127.0.0.1:${server.address().port}`;
const median = (values) =>
  [...values].sort((a, b) => a - b)[Math.floor(values.length / 2)];
const summaries = {};
const results = [];
let browser;
try {
  browser = await chromium.launch({ headless: true });
  const page = await browser.newPage({
    viewport: { width: 1200, height: 900 },
    deviceScaleFactor: 1,
  });
  page.setDefaultTimeout(15000);
  await page.route("**/*", (route) =>
    route.request().url().startsWith(`${origin}/`)
      ? route.continue()
      : route.abort(),
  );
  await page.goto(origin);
  await page.addScriptTag({
    path: path.join(
      root,
      "web/node_modules/lottie-web/build/player/lottie_light.min.js",
    ),
  });
  await page.evaluate(installPreviewMetrics);
  for (const [index, candidate] of [...selected.values()].entries()) {
    const bytes = await readFile(path.join(media, `${candidate.id}.tgs`));
    const json = gunzipSync(bytes);
    const row = { ...candidate, bytes, json, data: JSON.parse(json) };
    const sourceSha256 = createHash("sha256").update(bytes).digest("hex");
    if (row.expectedSha256)
      assert.equal(
        sourceSha256,
        row.expectedSha256,
        `Pinned source changed: ${row.id}`,
      );
    const rendered = await page.evaluate(
      async ({ data }) => {
        const host = document.getElementById("host");
        host.replaceChildren();
        const start = performance.now();
        const player = lottie.loadAnimation({
          container: host,
          renderer: "svg",
          loop: false,
          autoplay: false,
          animationData: data,
        });
        if (!player.isLoaded)
          await new Promise((resolve, reject) => {
            player.addEventListener("DOMLoaded", resolve);
            player.addEventListener("data_failed", reject);
          });
        player.goToAndStop(0, true);
        await new Promise(requestAnimationFrame);
        const svg = host.querySelector("svg").cloneNode(true);
        svg.setAttribute("width", String(data.w));
        svg.setAttribute("height", String(data.h));
        svg.removeAttribute("style");
        const text = new XMLSerializer().serializeToString(svg);
        const svgUrl = URL.createObjectURL(
          new Blob([text], { type: "image/svg+xml" }),
        );
        const variants = {};
        for (const size of [256, 512]) {
          const { canvas } = await window.raster(svgUrl, size);
          const encoded = canvas.toDataURL("image/webp", 0.9);
          if (!encoded.startsWith("data:image/webp;"))
            throw new Error("WebP encoding is unavailable");
          variants[`webp${size}`] = encoded.split(",")[1];
        }
        const first = await window.raster(svgUrl, 64);
        const visiblePixels = first.pixels.filter(
          (_, i) => i % 4 === 3 && first.pixels[i] > 8,
        ).length;
        URL.revokeObjectURL(svgUrl);
        return {
          svg: text,
          variants,
          generationMs: performance.now() - start,
          visiblePercent: (visiblePixels / (64 * 64)) * 100,
          nodes: svg.querySelectorAll("*").length,
        };
      },
      { data: row.data },
    );
    const references = {};
    for (const size of [96, 320, 512]) {
      await page.locator("#host").evaluate((host, size) => {
        host.style.width = host.style.height = `${size}px`;
      }, size);
      references[size] = (
        await page
          .locator("#host")
          .screenshot({ omitBackground: true, animations: "disabled" })
      ).toString("base64");
    }
    const record = {
      id: row.id,
      composition: row.data.nm ?? null,
      sourceSha256,
      tgsBytes: row.bytes.length,
      jsonBytes: row.json.length,
      nodes: rendered.nodes,
      visiblePercent: rendered.visiblePercent,
      generationMs: rendered.generationMs,
      variants: {},
    };
    const files = {
      svg: Buffer.from(rendered.svg),
      ...Object.fromEntries(
        Object.entries(rendered.variants).map(([key, value]) => [
          key,
          Buffer.from(value, "base64"),
        ]),
      ),
    };
    try {
      const thumbnail = await readFile(path.join(media, `${row.id}.thumb`));
      if (
        thumbnail.subarray(0, 4).toString() === "RIFF" &&
        thumbnail.subarray(8, 12).toString() === "WEBP"
      )
        files.telegram = thumbnail;
    } catch (error) {
      if (error.code !== "ENOENT") throw error;
    }
    for (const [format, bytes] of Object.entries(files)) {
      const filename = `${row.id}.${format === "svg" ? "svg" : `${format}.webp`}`;
      await writeFile(path.join(output, filename), bytes);
      assets.set(`/${filename}`, {
        bytes: format === "svg" ? gzipSync(bytes) : bytes,
        type: format === "svg" ? "image/svg+xml" : "image/webp",
      });
      const metrics = await page.evaluate(
        async ({ url, references }) => {
          const image = new Image();
          image.src = url;
          await image.decode();
          const quality = {};
          for (const size of [96, 320, 512]) {
            const expected = await window.raster(
              `data:image/png;base64,${references[size]}`,
              size,
            );
            const actual = await window.raster(url, size);
            quality[size] = window.quality(expected.pixels, actual.pixels);
          }
          return {
            width: image.naturalWidth,
            height: image.naturalHeight,
            quality,
          };
        },
        { url: `${origin}/${filename}`, references },
      );
      record.variants[format] = {
        file: filename,
        bytes: bytes.length,
        transferBytes: format === "svg" ? gzipSync(bytes).length : bytes.length,
        ...metrics,
      };
    }
    results.push(record);
    await page.evaluate(() => {
      lottie.destroy();
      document.getElementById("host").replaceChildren();
    });
    console.log(
      `${index + 1}/${selected.size} ${row.id}: SVG.gz ${record.variants.svg.transferBytes}, WebP256 ${record.variants.webp256.bytes}, WebP512 ${record.variants.webp512.bytes}`,
    );
  }
  // Repeated, alternated runs reduce format/order bias. Unique URLs force fresh decoder work.
  const client = await page.context().newCDPSession(page);
  await client.send("Performance.enable");
  await client.send("Emulation.setCPUThrottlingRate", { rate: cpuRate });
  await page.evaluate(() => {
    document.getElementById("host").remove();
    document.body.style.minHeight = "1800px";
  });
  const formats = [
    "svg",
    "webp256",
    "webp512",
    ...(results.some((row) => row.variants.telegram) ? ["telegram"] : []),
  ];
  const measurements = Object.fromEntries(
    formats.map((format) => [format, []]),
  );
  for (let run = 0; run < runs; run++)
    for (const format of [
      ...formats.slice(run % formats.length),
      ...formats.slice(0, run % formats.length),
    ]) {
      const files = results
        .map((row) => row.variants[format]?.file)
        .filter(Boolean);
      await page.evaluate(() =>
        document.getElementById("grid").replaceChildren(),
      );
      await client.send("HeapProfiler.collectGarbage");
      const before = Object.fromEntries(
        (await client.send("Performance.getMetrics")).metrics.map((m) => [
          m.name,
          m.value,
        ]),
      );
      const timing = await page.evaluate(
        async ({ files, format, run }) => {
          const start = performance.now();
          const blobs = await Promise.all(
            files.map(async (file) =>
              (await fetch(`/${file}?decode=${format}-${run}`)).blob(),
            ),
          );
          const fetchMs = performance.now() - start;
          const urls = blobs.map((blob) => URL.createObjectURL(blob));
          const images = urls.map((url) => {
            const img = new Image();
            img.src = url;
            return img;
          });
          await Promise.all(images.map((img) => img.decode()));
          const decodeMs = performance.now() - start - fetchMs;
          const grid = document.getElementById("grid");
          grid.style.cssText =
            "display:grid;grid-template-columns:repeat(10,96px);gap:8px;";
          for (let i = 0; i < 100; i++) {
            const img = images[i % images.length].cloneNode();
            img.width = img.height = 96;
            grid.append(img);
          }
          await new Promise((resolve) =>
            requestAnimationFrame(() => requestAnimationFrame(resolve)),
          );
          const mountMs = performance.now() - start - fetchMs - decodeMs;
          const frames = {};
          for (const mode of ["scroll", "resize"]) {
            const samples = [];
            for (let i = 0; i < 45; i++) {
              const previous = performance.now();
              window.scrollTo(0, (i % 20) * 8);
              if (mode === "resize")
                for (const img of grid.children) {
                  img.style.width = `${80 + (i % 17)}px`;
                  img.style.height = img.style.width;
                }
              await new Promise(requestAnimationFrame);
              samples.push(performance.now() - previous);
            }
            frames[`${mode}FrameMedianMs`] = samples.sort((a, b) => a - b)[22];
            frames[`${mode}LongFrames`] = samples.filter(
              (ms) => ms > 25,
            ).length;
          }
          grid.replaceChildren();
          for (const url of urls) URL.revokeObjectURL(url);
          return { fetchMs, decodeMs, mountMs, ...frames };
        },
        { files, format, run },
      );
      const after = Object.fromEntries(
        (await client.send("Performance.getMetrics")).metrics.map((m) => [
          m.name,
          m.value,
        ]),
      );
      measurements[format].push({
        ...timing,
        taskMs: (after.TaskDuration - before.TaskDuration) * 1000,
        layoutMs: (after.LayoutDuration - before.LayoutDuration) * 1000,
        heapDeltaBytes: after.JSHeapUsedSize - before.JSHeapUsedSize,
      });
      console.log(
        `Run ${run + 1}/${runs} ${format}: decode ${timing.decodeMs.toFixed(1)} ms, scroll ${timing.scrollFrameMedianMs.toFixed(1)} ms, resize ${timing.resizeFrameMedianMs.toFixed(1)} ms`,
      );
    }
  for (const format of formats) {
    const variants = results.map((row) => row.variants[format]).filter(Boolean);
    summaries[format] = {
      medianBytes: median(variants.map((row) => row.bytes)),
      medianTransferBytes: median(variants.map((row) => row.transferBytes)),
      totalTransferBytes: variants.reduce((n, row) => n + row.transferBytes, 0),
      medianPSNR: Object.fromEntries(
        [96, 320, 512].map((size) => [
          size,
          median(variants.map((row) => row.quality[size].psnr)),
        ]),
      ),
      medianChangedPercent: Object.fromEntries(
        [96, 320, 512].map((size) => [
          size,
          median(variants.map((row) => row.quality[size].changedPercent)),
        ]),
      ),
      timing: Object.fromEntries(
        Object.keys(measurements[format][0]).map((key) => [
          key,
          median(measurements[format].map((row) => row[key])),
        ]),
      ),
    };
  }
  const report = {
    created: new Date().toISOString(),
    browser: browser.version(),
    platform: `${process.platform}/${process.arch}`,
    cpu: cpus()[0]?.model,
    available: candidates.length,
    samples: results.length,
    runs,
    cpuRate,
    quality: 0.9,
    frame: 0,
    selection:
      corpusPath ??
      "decompressed-size quantiles plus cached brick/opacity regressions",
    summaries,
    measurements,
    results,
  };
  await writeFile(
    path.join(output, "results.json"),
    JSON.stringify(report, null, 2),
  );
  const escape = (value) =>
    String(value).replace(
      /[&<>"']/g,
      (char) =>
        ({
          "&": "&amp;",
          "<": "&lt;",
          ">": "&gt;",
          '"': "&quot;",
          "'": "&#39;",
        })[char],
    );
  const contact = `<!doctype html><meta charset="utf-8"><style>body{background:#211d25;color:#eee;font:14px system-ui}section{display:flex;align-items:center;gap:12px;margin:8px}article{width:160px;text-align:center}img{width:160px;height:160px;background:radial-gradient(#b489ad,#674260)}aside{width:210px}small{display:block}</style><h1>Frame 0: SVG / WebP 256 / WebP 512 / Telegram</h1>${results.map((row) => `<section><aside>${escape(row.composition ?? "Untitled")}<small>${row.id}</small></aside>${formats.map((format) => (row.variants[format] ? `<article><img src="${row.variants[format].file}"><small>${format}: ${(row.variants[format].transferBytes / 1024).toFixed(1)} KiB</small></article>` : "")).join("")}</section>`).join("")}`;
  await writeFile(path.join(output, "comparison.html"), contact);
  await page.setContent(contact.replaceAll('src="', `src="${origin}/`));
  await page
    .locator("img")
    .evaluateAll((images) =>
      Promise.all(images.map((image) => image.decode())),
    );
  await page.screenshot({
    path: path.join(output, "comparison.png"),
    fullPage: true,
  });
  console.log(JSON.stringify(summaries, null, 2));
  console.log(`Report: ${output}`);
} finally {
  await browser?.close();
  await new Promise((resolve) => server.close(resolve));
}
