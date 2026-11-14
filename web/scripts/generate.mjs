import { spawnSync } from "node:child_process";
import { mkdir, writeFile } from "node:fs/promises";
import openapiTS, { astToString } from "openapi-typescript";
const command = process.env.MARKET_SCHEMA_BIN;
const result = command
  ? spawnSync(command, ["schema"], { encoding: "utf8" })
  : spawnSync("cargo", ["run", "--quiet", "-p", "market-app", "--", "schema"], {
      cwd: "..",
      encoding: "utf8",
    });
if (result.status !== 0)
  throw new Error(result.stderr || "Could not generate OpenAPI");
const schema = JSON.parse(result.stdout);
await mkdir("src/gen", { recursive: true });
await writeFile("src/gen/api.ts", astToString(await openapiTS(schema)));
await writeFile("src/gen/openapi.json", JSON.stringify(schema));
