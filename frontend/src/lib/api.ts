// SPDX-License-Identifier: MIT OR Apache-2.0
//
// The ONE client boundary. Every server call goes through this module — JSON
// over HTTP, plus the Arrow-IPC panel-execute path. Swapping the backend (e.g.
// the DuckDB-WASM interactive-HTML export) means reimplementing this surface and
// nothing else.

import { decodeArrow, type ArrowResult } from "./arrow";
import type {
  AlmagestMeta,
  AssetEntry,
  Dashboard,
  DashboardSummary,
  DatabaseSchema,
  DatasetInfo,
  IngestOptions,
  IngestResult,
} from "./types";

const BASE = "/api/almagest";

/** An error carrying the server's structured `{ error: { code, message } }`. */
export class ApiError extends Error {
  code: string;
  status: number;
  constructor(status: number, code: string, message: string) {
    super(message);
    this.name = "ApiError";
    this.code = code;
    this.status = status;
  }
}

async function failure(res: Response): Promise<never> {
  let code = "http_error";
  let message = `${res.status} ${res.statusText}`;
  try {
    const body = await res.json();
    if (body?.error) {
      code = body.error.code ?? code;
      message = body.error.message ?? message;
    }
  } catch {
    // Non-JSON error body — keep the status line.
  }
  throw new ApiError(res.status, code, message);
}

async function getJson<T>(path: string): Promise<T> {
  const res = await fetch(`${BASE}${path}`);
  if (!res.ok) await failure(res);
  return (await res.json()) as T;
}

async function sendJson<T>(method: string, path: string, body?: unknown): Promise<T> {
  const res = await fetch(`${BASE}${path}`, {
    method,
    headers: { "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (!res.ok) await failure(res);
  if (res.status === 204) return undefined as T;
  const text = await res.text();
  return (text ? JSON.parse(text) : undefined) as T;
}

export const api = {
  meta: () => getJson<AlmagestMeta>(""),

  listDashboards: () => getJson<DashboardSummary[]>("/dashboards"),

  getDashboard: (id: string) => getJson<Dashboard>(`/dashboards/${encodeURIComponent(id)}`),

  createDashboard: (dashboard: Dashboard, folder?: string) =>
    sendJson<{ id: string }>("POST", "/dashboards", { ...dashboard, folder }),

  updateDashboard: (id: string, dashboard: Dashboard, folder?: string) =>
    sendJson<void>("PUT", `/dashboards/${encodeURIComponent(id)}`, { ...dashboard, folder }),

  deleteDashboard: (id: string) =>
    sendJson<void>("DELETE", `/dashboards/${encodeURIComponent(id)}`),

  schema: () => getJson<DatabaseSchema>("/schema"),

  resolveOptions: (dashboardId: string, parameter: string) =>
    sendJson<{ options: string[] }>("POST", "/options", {
      dashboard_id: dashboardId,
      parameter,
    }).then((r) => r.options),

  exportDashboard: async (id: string): Promise<string> => {
    const res = await fetch(`${BASE}/export/dashboard/${encodeURIComponent(id)}`, {
      method: "POST",
    });
    if (!res.ok) await failure(res);
    return res.text();
  },

  importDashboard: (dashboard: Dashboard) =>
    sendJson<{ id: string }>("POST", "/import/dashboard", dashboard),

  listAssets: () => getJson<AssetEntry[]>("/assets"),

  // --- data ingest / datasets ---

  listDatasets: () => getJson<DatasetInfo[]>("/datasets"),

  renameDataset: (name: string, to: string) =>
    sendJson<void>("POST", `/datasets/${encodeURIComponent(name)}/rename`, { to }),

  deleteDataset: (name: string) =>
    sendJson<void>("DELETE", `/datasets/${encodeURIComponent(name)}`),

  /** Ingest a file as a dataset (or preview its schema with `dryRun`). */
  ingestDataset: async (file: File | Blob, opts: IngestOptions): Promise<IngestResult> => {
    const params = new URLSearchParams();
    const filename = opts.filename ?? (file instanceof File ? file.name : undefined);
    if (opts.format) params.set("format", opts.format);
    if (filename) params.set("filename", filename);
    if (opts.name) params.set("name", opts.name);
    if (opts.replace) params.set("replace", "true");
    if (opts.dryRun) params.set("dry_run", "true");
    if (opts.jsonFormat) params.set("json_format", opts.jsonFormat);
    if (opts.noHeader) params.set("no_header", "true");
    if (opts.delimiter) params.set("delimiter", opts.delimiter);
    const res = await fetch(`${BASE}/datasets?${params.toString()}`, {
      method: "POST",
      headers: { "Content-Type": "application/octet-stream" },
      body: file,
    });
    if (!res.ok) await failure(res);
    return (await res.json()) as IngestResult;
  },

  // --- asset write ---

  uploadAsset: async (path: string, file: File | Blob): Promise<void> => {
    const res = await fetch(assetUrl(path), {
      method: "PUT",
      headers: { "Content-Type": file.type || "application/octet-stream" },
      body: file,
    });
    if (!res.ok) await failure(res);
  },

  deleteAsset: (path: string) =>
    fetch(assetUrl(path), { method: "DELETE" }).then(async (res) => {
      if (!res.ok) await failure(res);
    }),

  /** Execute a panel's query and decode the Arrow IPC result. */
  executePanel: async (
    dashboardId: string,
    panelId: string,
    parameters: Record<string, unknown>,
  ): Promise<ArrowResult> => {
    const res = await fetch(`${BASE}/panels/execute`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        dashboard_id: dashboardId,
        panel_id: panelId,
        parameters,
      }),
    });
    if (!res.ok) await failure(res);
    return decodeArrow(await res.arrayBuffer());
  },
};

/** Absolute URL for an embedded asset (used by image panels). */
export function assetUrl(path: string): string {
  return `${BASE}/assets/${path.split("/").map(encodeURIComponent).join("/")}`;
}
