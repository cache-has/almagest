// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Stops the fixture server and removes the temp file.

import fs from "node:fs";
import { STATE_FILE, type E2EState } from "./shared";

export default async function globalTeardown(): Promise<void> {
  if (!fs.existsSync(STATE_FILE)) return;
  const state = JSON.parse(fs.readFileSync(STATE_FILE, "utf8")) as E2EState;

  // Ask the server to shut down cleanly, then hard-kill as a backstop.
  try {
    await fetch(`http://127.0.0.1:${state.port}/api/almagest/shutdown`, { method: "POST" });
  } catch {
    // ignore — falling through to kill
  }
  if (state.pid > 0) {
    try {
      process.kill(state.pid);
    } catch {
      // already gone
    }
  }
  try {
    fs.rmSync(STATE_FILE);
  } catch {
    // ignore
  }
}
