import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
export default defineConfig({
  plugins: [react()],
  server: {
    proxy: Object.fromEntries(
      [
        "/telegram",
        "/auth",
        "/sync",
        "/books",
        "/catalog",
        "/orders",
        "/executions",
        "/openapi.json",
        "/healthz",
        "/readyz",
        "/ws",
      ].map((path) => [
        path,
        { target: "http://127.0.0.1:8080", ws: path === "/ws" },
      ]),
    ),
  },
});
