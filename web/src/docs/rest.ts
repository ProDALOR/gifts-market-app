import { config } from "zod/v4/core";
import "@scalar/api-reference/style.css";
import "./docs.css";

// Scalar's schema validation must use the CSP-safe interpreter, including its
// feature probe. Set this before importing Scalar and constructing its schemas.
config({ jitless: true });
try {
  const { createApiReference } = await import("@scalar/api-reference");
  const reference = createApiReference("#api-reference", {
    url: "/openapi.json",
    servers: [{ url: location.origin, description: "This server" }],
    withDefaultFonts: false,
    telemetry: false,
    agent: { disabled: true },
    proxyUrl: "",
    persistAuth: false,
    showDeveloperTools: "never",
    hideClientButton: true,
    darkMode: true,
  });
  window.addEventListener("pagehide", (event) => {
    if (!event.persisted) reference.destroy();
  });
} catch {
  document.getElementById("api-reference")!.textContent =
    "The REST reference could not load. Reload this page or open /openapi.json.";
}
