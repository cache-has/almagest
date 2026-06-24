// SPDX-License-Identifier: MIT OR Apache-2.0

import { mount } from "svelte";
import App from "./App.svelte";
import "./app.css";
import { getSnapshot } from "./lib/snapshotData";
import { api } from "./lib/api";
import { startHeartbeat } from "./lib/heartbeat";

const snap = getSnapshot();
if (snap && !location.hash.startsWith("#/view/")) {
  // A baked snapshot opens straight into its (read-only) dashboard view.
  location.hash = `#/view/${encodeURIComponent(snap.dashboardId)}`;
}

const target = document.getElementById("app");
if (!target) throw new Error("missing #app mount point");

const app = mount(App, { target });

// Desktop mode: keep the server alive while a tab is open; when the last tab
// closes the pings stop and the server exits. Only runs when the server asks
// for it (never in snapshot mode or headless serve).
if (!snap) {
  api
    .meta()
    .then((m) => {
      if (m.heartbeat_enabled) startHeartbeat();
    })
    .catch(() => {});
}

export default app;
