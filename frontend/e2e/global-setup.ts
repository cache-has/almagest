// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Builds a seeded `.alm`, serves it with the real binary, and leaves the server
// running for the suite. Seeding goes through the live HTTP API (the same path a
// user's Studio would) so the fixture exercises real ingest + dashboard CRUD.

import { spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import {
  ALMAGEST_BIN,
  BASE_URL,
  DASHBOARD,
  PORT,
  SALES_CSV,
  STATE_FILE,
  type E2EState,
} from "./shared";

async function waitForServer(timeoutMs: number): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      const res = await fetch(`${BASE_URL}/api/almagest`);
      if (res.ok) return;
    } catch {
      // not up yet
    }
    await new Promise((r) => setTimeout(r, 150));
  }
  throw new Error(`almagest server did not come up on ${BASE_URL} within ${timeoutMs}ms`);
}

export default async function globalSetup(): Promise<void> {
  if (!fs.existsSync(ALMAGEST_BIN)) {
    throw new Error(
      `almagest binary not found at ${ALMAGEST_BIN}. Build it first: ` +
        `(cd frontend && npm run build) && cargo build -p almagest-cli`,
    );
  }

  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "almagest-e2e-"));
  const almPath = path.join(tmpDir, "smoke.alm");

  // 1) create the file
  await run(ALMAGEST_BIN, ["new", almPath]);

  // 2) serve it (long-lived; killed in teardown)
  const child = spawn(ALMAGEST_BIN, ["serve", almPath, "--port", String(PORT)], {
    stdio: "inherit",
    detached: false,
  });
  child.on("error", (e) => {
    throw e;
  });

  await waitForServer(15_000);

  // 3) ingest the dataset via the live API
  const ingestRes = await fetch(
    `${BASE_URL}/api/almagest/datasets?format=csv&filename=sales.csv&name=sales`,
    { method: "POST", headers: { "Content-Type": "application/octet-stream" }, body: SALES_CSV },
  );
  if (!ingestRes.ok) {
    throw new Error(`dataset ingest failed: ${ingestRes.status} ${await ingestRes.text()}`);
  }

  // 4) create the dashboard via the live API
  const dashRes = await fetch(`${BASE_URL}/api/almagest/dashboards`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(DASHBOARD),
  });
  if (!dashRes.ok) {
    throw new Error(`dashboard create failed: ${dashRes.status} ${await dashRes.text()}`);
  }
  const { id: dashboardId } = (await dashRes.json()) as { id: string };

  const state: E2EState = { port: PORT, pid: child.pid ?? -1, almPath, dashboardId };
  fs.mkdirSync(path.dirname(STATE_FILE), { recursive: true });
  fs.writeFileSync(STATE_FILE, JSON.stringify(state, null, 2));
}

function run(cmd: string, args: string[]): Promise<void> {
  return new Promise((resolve, reject) => {
    const p = spawn(cmd, args, { stdio: "inherit" });
    p.on("error", reject);
    p.on("exit", (code) => (code === 0 ? resolve() : reject(new Error(`${cmd} exited ${code}`))));
  });
}
