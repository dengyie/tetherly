import { defineConfig } from "@playwright/test";

// Browser-tier e2e over the MockTransport seam: the test server serves the
// built SPA at /next/ with a mock /api/v1 back end, so the same script that
// unit tests drive flows through, a real Chromium renders.
export default defineConfig({
  testDir: "e2e",
  timeout: 30_000,
  use: {
    // System Chrome (no CDN download); CI machines that lack it should set
    // PW_CHANNEL and install a browser via `npx playwright install`.
    channel: process.env.PW_CHANNEL || "chrome",
    baseURL: "http://127.0.0.1:4173",
  },
  webServer: {
    command: "node e2e/server.mjs",
    port: 4173,
    reuseExistingServer: false,
  },
});
