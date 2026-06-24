// SPDX-License-Identifier: MIT OR Apache-2.0
//
// End-to-end smoke coverage for the Phase 10 viewer running in a real browser
// against the real binary: render, parameter → re-query + URL sync, share-link,
// and the static snapshot export.

import { test, expect } from "@playwright/test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { STATE_FILE, EU_TOTAL, US_TOTAL, type E2EState } from "./shared";

let state: E2EState;

test.beforeAll(() => {
  state = JSON.parse(fs.readFileSync(STATE_FILE, "utf8")) as E2EState;
});

function viewerPath(): string {
  return `/#/view/${encodeURIComponent(state.dashboardId)}`;
}

test("viewer renders panels with embedded data", async ({ page }) => {
  await page.goto(viewerPath());

  await expect(page.getByRole("heading", { name: "Smoke Dashboard" })).toBeVisible();

  // Metric resolves to the EU default total.
  await expect(page.locator(".metric .value")).toContainText(EU_TOTAL, { timeout: 10_000 });

  // Chart rendered to a canvas (ECharts canvas renderer).
  await expect(page.locator(".chart canvas").first()).toBeVisible();

  // Table has the embedded rows.
  await expect(page.locator("table")).toContainText("2026-01");
});

test("parameter change re-queries and syncs the URL", async ({ page }) => {
  await page.goto(viewerPath());
  await expect(page.locator(".metric .value")).toContainText(EU_TOTAL, { timeout: 10_000 });

  // Flip region EU → US.
  await page.getByRole("combobox").selectOption("US");

  await expect(page.locator(".metric .value")).toContainText(US_TOTAL, { timeout: 10_000 });
  // Shareable URL reflects the new parameter value.
  await expect.poll(() => page.url()).toContain("region=US");
});

test("a shared link restores parameter state", async ({ page }) => {
  await page.goto(`${viewerPath()}?region=US`);
  await expect(page.locator(".metric .value")).toContainText(US_TOTAL, { timeout: 10_000 });
  // The select reflects the URL-seeded value.
  await expect(page.getByRole("combobox")).toHaveValue("US");
});

test("share button copies the link", async ({ page }) => {
  await page.goto(viewerPath());
  await page.getByRole("button", { name: "Share" }).click();
  await expect(page.getByRole("button", { name: /Copied/ })).toBeVisible();
});

test("snapshot export downloads a self-contained HTML file", async ({ page }) => {
  await page.goto(viewerPath());
  await expect(page.locator(".metric .value")).toContainText(EU_TOTAL, { timeout: 10_000 });

  const [download] = await Promise.all([
    page.waitForEvent("download"),
    page.getByRole("button", { name: /Export/ }).click(),
  ]);

  expect(download.suggestedFilename()).toMatch(/snapshot\.html$/);

  const out = path.join(os.tmpdir(), download.suggestedFilename());
  await download.saveAs(out);
  const html = fs.readFileSync(out, "utf8");

  // Self-contained: a full doc with the title, a frozen metric, and a chart image.
  expect(html).toContain("<!doctype html>");
  expect(html).toContain("Smoke Dashboard");
  expect(html).toContain("Snapshot");
  expect(html).toContain(EU_TOTAL);
  expect(html).toMatch(/data:image\/png;base64,/); // chart rasterized inline
  fs.rmSync(out, { force: true });
});
