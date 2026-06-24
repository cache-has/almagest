// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Snapshot mode. `almagest export` bakes a self-contained static HTML that sets
// `window.__ALMAGEST_SNAPSHOT__` to a frozen payload — the dashboard definition,
// each panel's pre-executed result (Arrow IPC, base64), referenced assets as
// data URLs, and the parameter values in effect. When present, the API client
// (lib/api.ts) serves everything from this payload instead of the HTTP server,
// so the same frontend renders with no backend at all.

import type { Dashboard } from "./types";

export interface SnapshotPayload {
  dashboard: Dashboard;
  dashboardId: string;
  /** panelId → base64-encoded Arrow IPC stream of that panel's frozen result. */
  panels: Record<string, string>;
  /** asset path → inlined data URL. */
  assets: Record<string, string>;
  /** Parameter values frozen at export time (for display in the frozen bar). */
  params: Record<string, unknown>;
  /** ISO timestamp the snapshot was generated. */
  generatedAt: string;
}

declare global {
  interface Window {
    __ALMAGEST_SNAPSHOT__?: SnapshotPayload;
  }
}

let cached: SnapshotPayload | null | undefined;

/** The active snapshot payload, or null when running against a live server. */
export function getSnapshot(): SnapshotPayload | null {
  if (cached === undefined) {
    cached = (typeof window !== "undefined" && window.__ALMAGEST_SNAPSHOT__) || null;
  }
  return cached;
}

/** Decode a base64 string to bytes (for the inlined Arrow IPC results). */
export function base64ToBytes(b64: string): Uint8Array {
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return bytes;
}
