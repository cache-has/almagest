// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Playwright smoke config. The fixture server is managed by global-setup /
// global-teardown (it must be seeded *after* it starts), so we don't use the
// `webServer` option. Run with: `npm run e2e` (build the bundle + binary first).

import { defineConfig, devices } from "@playwright/test";
import { BASE_URL } from "./e2e/shared";

export default defineConfig({
  testDir: "./e2e",
  testMatch: /.*\.spec\.ts/,
  fullyParallel: false,
  workers: 1,
  timeout: 30_000,
  retries: 0,
  reporter: [["list"]],
  globalSetup: "./e2e/global-setup.ts",
  globalTeardown: "./e2e/global-teardown.ts",
  use: {
    baseURL: BASE_URL,
    trace: "retain-on-failure",
    // Snapshot export uses navigator.clipboard for the Share button; grant it.
    permissions: ["clipboard-read", "clipboard-write"],
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
});
