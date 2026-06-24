// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Exercises `almagest export`: bake a static HTML snapshot from the seeded
// fixture file, then open it via file:// in a real browser and confirm it
// renders fully offline (no server) with frozen, read-only parameters.

import { test, expect } from "@playwright/test";
import { spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { ALMAGEST_BIN, STATE_FILE, US_TOTAL, type E2EState } from "./shared";

function run(cmd: string, args: string[]): Promise<void> {
  return new Promise((resolve, reject) => {
    const p = spawn(cmd, args, { stdio: "inherit" });
    p.on("error", reject);
    p.on("exit", (code) => (code === 0 ? resolve() : reject(new Error(`${cmd} exited ${code}`))));
  });
}

test("exported snapshot opens offline and renders frozen data", async ({ page, context }) => {
  const state = JSON.parse(fs.readFileSync(STATE_FILE, "utf8")) as E2EState;
  const outDir = fs.mkdtempSync(path.join(os.tmpdir(), "almagest-export-"));
  const outFile = path.join(outDir, "snap.html");

  // Bake the snapshot at region=US.
  await run(ALMAGEST_BIN, [
    "export",
    state.almPath,
    "--output",
    outFile,
    "--parameters",
    JSON.stringify({ region: "US" }),
  ]);
  expect(fs.existsSync(outFile)).toBe(true);
  // Self-contained: the payload + the bundle are inlined.
  const html = fs.readFileSync(outFile, "utf8");
  expect(html).toContain("window.__ALMAGEST_SNAPSHOT__");

  // Open it as a file:// page — no server involved.
  await page.goto(pathToFileURL(outFile).href);

  // Frozen banner + frozen US metric, rendered entirely from the inlined data.
  await expect(page.locator(".snapshot-banner")).toBeVisible();
  await expect(page.locator(".metric .value")).toContainText(US_TOTAL, { timeout: 10_000 });
  await expect(page.locator(".chart canvas").first()).toBeVisible();

  // Read-only: the parameter select is disabled and the live toolbar is gone.
  await expect(page.getByRole("combobox")).toBeDisabled();
  await expect(page.getByRole("button", { name: "Share" })).toHaveCount(0);

  await context.close();
  fs.rmSync(outDir, { recursive: true, force: true });
});
