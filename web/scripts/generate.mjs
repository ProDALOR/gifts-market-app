import { spawnSync } from "node:child_process";
import { mkdir, writeFile } from "node:fs/promises";
import openapiTS, { astToString } from "openapi-typescript";
const command = process.env.MARKET_SCHEMA_BIN;
const features = process.env.MARKET_SCHEMA_FEATURES;
const run = (subcommand) =>
  command
    ? spawnSync(command, [subcommand], { encoding: "utf8" })
    : spawnSync(
        "cargo",
        [
          "run",
          "--quiet",
          "-p",
          "market-app",
          ...(features ? ["--features", features] : []),
          "--",
          subcommand,
        ],
        {
          cwd: "..",
          encoding: "utf8",
        },
      );
const result = run("schema");
if (result.status !== 0)
  throw new Error(result.stderr || "Could not generate OpenAPI");
const schema = JSON.parse(result.stdout);
await mkdir("src/gen", { recursive: true });
await writeFile("src/gen/api.ts", astToString(await openapiTS(schema)));
await writeFile("src/gen/openapi.json", JSON.stringify(schema));

// Validate the server's AsyncAPI envelope against the same Rust build as OpenAPI.
const asyncResult = run("asyncapi");
if (asyncResult.status !== 0)
  throw new Error(asyncResult.stderr || "Could not generate AsyncAPI");
const { Parser } = await import("@asyncapi/parser");
const { document, diagnostics } = await new Parser().parse(asyncResult.stdout);
const errors = diagnostics.filter((item) => item.severity === 0);
if (!document || errors.length)
  throw new Error(JSON.stringify(errors, null, 2));
