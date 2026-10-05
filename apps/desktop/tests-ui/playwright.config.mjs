// Browser tests: the built frontend, the production CSP, and a stand-in for
// the backend that replays responses recorded from the real commands
// (see src-tauri/tests/ui_fixtures.rs). Run with `npm run test:ui`.

import { existsSync } from "node:fs";
import { defineConfig } from "@playwright/test";

// A system Chromium when there is one; otherwise `npx playwright install chromium`.
const system = ["/usr/bin/chromium", "/usr/bin/chromium-browser", "/usr/bin/google-chrome"].find(existsSync);

export default defineConfig({
  testDir: ".",
  testMatch: /.*\.spec\.mjs$/,
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  reporter: process.env.CI ? "github" : "list",
  timeout: 30_000,
  use: {
    baseURL: "http://127.0.0.1:4173",
    viewport: { width: 1280, height: 860 },
    // The frontend serves vault media from http://asset.localhost when it
    // believes it is on Windows; that origin, unlike asset://, can be routed.
    userAgent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0 Safari/537.36",
    launchOptions: system ? { executablePath: system } : {},
    trace: "retain-on-failure",
  },
  webServer: {
    command: "npx vite preview --port 4173 --strictPort --host 127.0.0.1",
    cwd: "..",
    url: "http://127.0.0.1:4173",
    reuseExistingServer: !process.env.CI,
  },
});
