// Validate the production Rust renderer against the pinned 43-gift Lottie corpus.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { readFile, writeFile, mkdir } from "node:fs/promises";
import { createServer } from "node:http";
import { gzipSync } from "node:zlib";
import { performance } from "node:perf_hooks";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";
import { installPreviewMetrics } from "./preview-metrics.mjs";

const root = fileURLToPath(new URL("../..", import.meta.url));
const args = process.argv.slice(2);
const option = (key, fallback) =>
  args.includes(key) ? args[args.indexOf(key) + 1] : fallback;
const output = path.resolve(
  root,
  option("--output", ".market/run/svg-validation"),
);
const media = path.resolve(root, option("--media-dir", "data/live-hub/media"));
const referenceDir = path.resolve(
  root,
  option("--reference", ".market/run/preview-benchmark"),
);
const corpusPath = "web/scripts/fixtures/preview-corpus.json";
const corpus = JSON.parse(await readFile(path.join(root, corpusPath), "utf8"));
assert.equal(
  corpus.gifts.length,
  43,
  "The full 43-gift regression corpus is required",
);
const reference = JSON.parse(
  await readFile(path.join(referenceDir, "results.json"), "utf8"),
);
assert.equal(reference.frame, 0);
assert.deepEqual(
  reference.results.map(({ id, sourceSha256 }) => ({ id, sourceSha256 })),
  corpus.gifts.map(({ id, sourceSha256 }) => ({ id, sourceSha256 })),
);
const renderer = path.resolve(
  root,
  option("--renderer", "target/debug/market-media-render"),
);
const sha = (bytes) => createHash("sha256").update(bytes).digest("hex");
const rendererSha256 = sha(await readFile(renderer));
await mkdir(output, { recursive: true });
const assets = new Map();
const rows = [];
for (const gift of reference.results) {
  const input = path.join(media, `${gift.id}.tgs`);
  const source = await readFile(input);
  assert.equal(sha(source), gift.sourceSha256, `Source changed: ${gift.id}`);
  const original = await readFile(
    path.join(referenceDir, gift.variants.svg.file),
  );
  assets.set(`/${gift.id}.reference.svg`, {
    bytes: original,
    type: "image/svg+xml",
  });
  await writeFile(path.join(output, `${gift.id}.reference.svg`), original);
  const file = `${gift.id}.rust.svg`;
  const row = {
    id: gift.id,
    composition: gift.composition,
    sourceSha256: gift.sourceSha256,
    referenceSha256: sha(original),
    file,
    diagnostics: [],
  };
  const start = performance.now();
  const result = spawnSync(renderer, [input, path.join(output, file)], {
    encoding: "utf8",
    timeout: 10000,
    maxBuffer: 1024 * 1024,
  });
  row.processMs = performance.now() - start;
  if (result.status !== 0) {
    row.error =
      result.error?.message || result.stderr.trim() || `Exit ${result.status}`;
    row.passed = false;
    rows.push(row);
    continue;
  }
  row.diagnostics = result.stderr.trim() ? [result.stderr.trim()] : [];
  const bytes = await readFile(path.join(output, file));
  row.svgBytes = bytes.length;
  row.gzipBytes = gzipSync(bytes).length;
  row.svgSha256 = sha(bytes);
  assets.set(`/${file}`, { bytes, type: "image/svg+xml" });
  rows.push(row);
}
const server = createServer((request, response) => {
  if (request.url === "/") {
    response.setHeader("Content-Type", "text/html");
    response.end("<!doctype html><meta charset=utf-8>");
    return;
  }
  const asset = assets.get(request.url);
  if (!asset) {
    response.writeHead(404).end();
    return;
  }
  response.setHeader("Content-Type", asset.type);
  response.end(asset.bytes);
});
await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
const origin = `http://127.0.0.1:${server.address().port}`;
let browser;
try {
  browser = await chromium.launch({ headless: true });
  const page = await browser.newPage({
    viewport: { width: 1450, height: 900 },
  });
  await page.route("**/*", (route) =>
    route.request().url().startsWith(`${origin}/`)
      ? route.continue()
      : route.abort(),
  );
  await page.goto(origin);
  await page.evaluate(installPreviewMetrics);
  for (const row of rows) {
    if (row.error) {
      console.log(`FAIL ${row.id}: ${row.error.slice(0, 160)}`);
      continue;
    }
    try {
      const quality = await page.evaluate(
        async ({ id, file }) => {
          const [reference, actual] = await Promise.all([
            window.raster(`/${id}.reference.svg`, 512),
            window.raster(`/${file}`, 512),
          ]);
          const text = await (await fetch(`/${file}`)).text();
          const xml = new DOMParser().parseFromString(text, "image/svg+xml");
          const external = [...xml.querySelectorAll("*")].some((node) =>
            [...node.attributes].some(
              (attr) =>
                (/^(href|src)$/.test(attr.localName) &&
                  !attr.value.startsWith("#")) ||
                /url\(\s*["']?(?!#)/.test(attr.value),
            ),
          );
          return {
            quality: window.quality(reference.pixels, actual.pixels),
            svgElements: xml.querySelectorAll("*").length,
            vectorOnly:
              !xml.querySelector(
                "image,foreignObject,script,animate,animateTransform,parsererror",
              ) && !external,
            png: actual.canvas.toDataURL().split(",")[1],
          };
        },
        { id: row.id, file: row.file },
      );
      const { png, ...metrics } = quality;
      Object.assign(row, metrics);
      row.visualPassed =
        row.quality.psnr >= 30 && row.quality.changedPercent <= 10;
      row.passed =
        row.visualPassed && row.vectorOnly && row.diagnostics.length === 0;
      const bytes = Buffer.from(png, "base64");
      await writeFile(path.join(output, `${row.id}.rust.png`), bytes);
      console.log(
        `${row.passed ? "PASS" : "FAIL"} ${row.id}: ${row.quality.psnr.toFixed(1)} dB / ${row.quality.changedPercent.toFixed(1)}%, ${row.diagnostics.length} diagnostic(s)`,
      );
    } catch (error) {
      row.error = error.message;
      row.passed = false;
    }
  }
  const median = (values) => {
    if (!values.length) return null;
    values.sort((a, b) => a - b);
    const middle = Math.floor(values.length / 2);
    return values.length % 2
      ? values[middle]
      : (values[middle - 1] + values[middle]) / 2;
  };
  const exports = rows.filter((row) => !row.error);
  const summary = {
    total: rows.length,
    exported: exports.length,
    visualPassed: exports.filter((row) => row.visualPassed).length,
    passed: rows.filter((row) => row.passed).length,
    diagnostics: exports.filter((row) => row.diagnostics.length > 0).length,
    medianProcessMs: median(exports.map((row) => row.processMs)),
    medianGzipBytes: median(exports.map((row) => row.gzipBytes)),
    medianPsnr: median(
      exports
        .map((row) => row.quality?.psnr)
        .filter((value) => value !== undefined),
    ),
  };
  const report = {
    created: new Date().toISOString(),
    browser: browser.version(),
    referenceBrowser: reference.browser,
    corpus: corpusPath,
    frame: 0,
    width: 512,
    height: 512,
    gate: {
      minimumPsnr: 30,
      maxChangedPercent: 10,
      maxChannelDifference: 10,
      backgrounds: [24, 245],
      vectorOnly: true,
      noDiagnostics: true,
    },
    renderer: { executable: renderer, sha256: rendererSha256 },
    summary,
    results: rows,
  };
  await writeFile(
    path.join(output, "results.json"),
    JSON.stringify(report, null, 2) + "\n",
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
  const gallery = `<!doctype html><meta charset=utf-8><style>body{background:#211d25;color:#eee;font:14px system-ui}section{display:flex;gap:12px;align-items:center;margin:16px 0}aside{width:220px}article{width:256px;text-align:center}img,.error{width:256px;height:256px;background:radial-gradient(#b489ad,#674260)}small{display:block}.error{overflow:auto;text-align:left;white-space:pre-wrap}</style><h1>Rust SVG comparison</h1><pre>${escape(JSON.stringify(summary, null, 2))}</pre>${rows
    .map(
      (row) =>
        `<section data-id="${row.id}"><aside>${escape(row.composition ?? "Untitled")}<small>${row.id}</small></aside><article><img src="${row.id}.reference.svg"><small>Lottie baseline</small></article><article>${row.error ? `<div class="error">${escape(row.error)}</div>` : `<img src="${row.file}">`}<small>Rust: ${row.error ? "ERROR" : `${row.quality?.psnr.toFixed(1)} dB / ${row.quality?.changedPercent.toFixed(1)}%`}</small><small>${row.diagnostics.length} diagnostic(s)</small></article></section>`,
    )
    .join("")}`;
  await writeFile(path.join(output, "comparison.html"), gallery);
  await page.setContent(gallery.replaceAll('src="', `src="${origin}/`));
  await page
    .locator("img")
    .evaluateAll((images) =>
      Promise.all(images.map((image) => image.decode())),
    );
  await page.screenshot({
    path: path.join(output, "comparison.png"),
    fullPage: true,
  });
  await page.evaluate(() => {
    document.querySelector("pre").remove();
    for (const section of document.querySelectorAll("section"))
      section.style.display = [
        "5222196055977778464",
        "5352858362805512179",
        "5375536850985318669",
        "5195366267158033241",
      ].includes(section.dataset.id)
        ? ""
        : "none";
  });
  await page
    .locator("body")
    .screenshot({ path: path.join(output, "regression-cases.png") });
  console.log(JSON.stringify(summary, null, 2));
  console.log(`Report: ${output}`);
  if (summary.passed !== summary.total) process.exitCode = 1;
} finally {
  await browser?.close();
  await new Promise((resolve) => server.close(resolve));
}
