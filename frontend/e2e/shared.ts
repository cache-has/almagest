// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Shared constants + fixture data for the Playwright smoke suite. The suite
// builds a real seeded `.alm`, serves it with the actual `almagest` binary, and
// drives the embedded frontend in a real browser — the end-to-end path Phase 09
// and Phase 10 could only assert via build + types.

import { fileURLToPath } from "node:url";
import path from "node:path";

const here = path.dirname(fileURLToPath(import.meta.url));

/** Fixed loopback port for the test server (shared by config + setup). */
export const PORT = 8791;
export const BASE_URL = `http://127.0.0.1:${PORT}`;

/** Repo paths. */
export const REPO_ROOT = path.resolve(here, "..", "..");
export const ALMAGEST_BIN = path.join(REPO_ROOT, "target", "debug", "almagest");

/** Where global-setup stashes runtime state for the specs + teardown. */
export const STATE_FILE = path.join(here, ".artifacts", "state.json");

export interface E2EState {
  port: number;
  pid: number;
  almPath: string;
  dashboardId: string;
}

/** A tiny embedded dataset with a clean EU/US split so region filtering is observable. */
export const SALES_CSV = [
  "region,month,revenue,orders",
  "EU,2026-01,1000,10",
  "EU,2026-02,1200,12",
  "US,2026-01,2000,20",
  "US,2026-02,2500,25",
].join("\n");

// EU total revenue = 2200, US total = 4500 — the smoke test asserts the metric
// flips between these when the region parameter changes.
export const EU_TOTAL = "2,200";
export const US_TOTAL = "4,500";

/** The dashboard definition (DSL JSON) seeded into the fixture file. */
export const DASHBOARD = {
  version: 1,
  name: "Smoke Dashboard",
  description: "End-to-end smoke fixture.",
  parameters: [
    {
      id: "region",
      kind: "select",
      label: "Region",
      options: ["EU", "US"],
      default: "EU",
    },
  ],
  layout: {
    grid: 12,
    rows: [
      {
        panels: [
          {
            id: "total_revenue",
            kind: "metric",
            title: "Total Revenue",
            span: 4,
            query: { sql: "SELECT SUM(revenue) AS value FROM sales WHERE region = {{region}}" },
            format: { kind: "currency", prefix: "$" },
          },
          {
            id: "revenue_by_month",
            kind: "chart",
            chart_type: "bar",
            title: "Revenue by Month",
            span: 8,
            query: {
              sql: "SELECT month, SUM(revenue) AS revenue FROM sales WHERE region = {{region}} GROUP BY month ORDER BY month",
            },
            x: "month",
            y: "revenue",
            y_format: { kind: "currency", prefix: "$" },
          },
        ],
      },
      {
        panels: [
          {
            id: "rows",
            kind: "table",
            title: "Detail",
            span: 12,
            query: {
              sql: "SELECT region, month, revenue, orders FROM sales WHERE region = {{region}} ORDER BY month",
            },
            sortable: true,
          },
        ],
      },
    ],
  },
};
