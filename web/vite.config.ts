import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { readFileSync } from "node:fs";
const schema = JSON.parse(
  readFileSync(new URL("./src/gen/openapi.json", import.meta.url), "utf8"),
);
const backend = process.env.MARKET_DEV_BACKEND || "http://127.0.0.1:8080";
export default defineConfig({
  define: {
    "import.meta.env.VITE_MARKET_MINIAPP": JSON.stringify(
      schema.paths["/auth/telegram"] ? "true" : "false",
    ),
  },
  plugins: [react()],
  build: {
    manifest: true,
    rolldownOptions: {
      input: {
        market: "index.html",
        docs: "docs/index.html",
        websocket: "docs/ws/index.html",
      },
    },
  },
  server: {
    proxy: Object.fromEntries(
      [
        "/media",
        "/telegram",
        "/auth",
        "/catalog",
        "/collections",
        "/valuations",
        "/orders",
        "/portfolios",
        "/history",
        "/openapi.json",
        "/asyncapi.json",
        "/healthz",
        "/readyz",
        "/ws",
      ].map((path) => [
        path,
        {
          target: backend,
          changeOrigin: true,
          ws: path === "/ws",
          rewriteWsOrigin: path === "/ws",
        },
      ]),
    ),
  },
});
