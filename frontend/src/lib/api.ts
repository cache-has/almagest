// SPDX-License-Identifier: MIT OR Apache-2.0
//
// The ONE client boundary. Every server call goes through this module — JSON
// over HTTP, plus the Arrow-IPC panel-execute path. Swapping the backend (e.g.
// the DuckDB-WASM interactive-HTML export) means reimplementing this surface and
// nothing else.

import { decodeArrow, type ArrowResult } from "./arrow";
import { getSnapshot, base64ToBytes } from "./snapshotData";
import type {
  AlmagestMeta,
  AssetEntry,
  AuthMe,
  AuthSession,
  Dashboard,
  DashboardSummary,
  DatabaseSchema,
  DatasetInfo,
  HistoryEntry,
  IngestOptions,
  IngestResult,
  Role,
  User,
} from "./types";

const BASE = "/api/almagest";

/** Read the double-submit CSRF token from the JS-readable `alm_csrf` cookie. */
function csrfToken(): string | undefined {
  const m = document.cookie.match(/(?:^|;\s*)alm_csrf=([^;]+)/);
  return m ? decodeURIComponent(m[1]) : undefined;
}

/** Headers for a state-changing request: JSON content type + CSRF token. The
 *  token is required by the server only when auth is enabled; sending it
 *  unconditionally is harmless otherwise. */
function writeHeaders(json: boolean, contentType?: string): Record<string, string> {
  const h: Record<string, string> = {};
  if (json) h["Content-Type"] = "application/json";
  else if (contentType) h["Content-Type"] = contentType;
  const t = csrfToken();
  if (t) h["X-CSRF-Token"] = t;
  return h;
}

/** An empty Arrow result for panels with no baked snapshot data. */
const EMPTY_RESULT: ArrowResult = { fields: [], rows: [], rowCount: 0 };

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
    headers: writeHeaders(true),
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (!res.ok) await failure(res);
  if (res.status === 204) return undefined as T;
  const text = await res.text();
  return (text ? JSON.parse(text) : undefined) as T;
}

export const api = {
  meta: (): Promise<AlmagestMeta> => {
    const snap = getSnapshot();
    if (snap) {
      return Promise.resolve({
        id: snap.dashboardId,
        title: snap.dashboard.name,
        description: snap.dashboard.description ?? "",
        format_version: 0,
        server_version: "snapshot",
        dashboard_count: 1,
        read_only: true,
        heartbeat_enabled: false,
        auth_enabled: false,
      });
    }
    return getJson<AlmagestMeta>("");
  },

  listDashboards: (): Promise<DashboardSummary[]> => {
    const snap = getSnapshot();
    if (snap) {
      return Promise.resolve([
        {
          id: snap.dashboardId,
          name: snap.dashboard.name,
          description: snap.dashboard.description ?? null,
          folder: null,
          created_at: snap.generatedAt,
          updated_at: snap.generatedAt,
        },
      ]);
    }
    return getJson<DashboardSummary[]>("/dashboards");
  },

  getDashboard: (id: string): Promise<Dashboard> => {
    const snap = getSnapshot();
    if (snap) return Promise.resolve(snap.dashboard);
    return getJson<Dashboard>(`/dashboards/${encodeURIComponent(id)}`);
  },

  createDashboard: (dashboard: Dashboard, folder?: string) =>
    sendJson<{ id: string }>("POST", "/dashboards", { ...dashboard, folder }),

  updateDashboard: (id: string, dashboard: Dashboard, folder?: string) =>
    sendJson<void>("PUT", `/dashboards/${encodeURIComponent(id)}`, { ...dashboard, folder }),

  deleteDashboard: (id: string) =>
    sendJson<void>("DELETE", `/dashboards/${encodeURIComponent(id)}`),

  schema: () => getJson<DatabaseSchema>("/schema"),

  resolveOptions: (dashboardId: string, parameter: string): Promise<string[]> => {
    // In a snapshot, parameters are frozen — declared static options only.
    const snap = getSnapshot();
    if (snap) {
      const decl = snap.dashboard.parameters?.find((p) => p.id === parameter);
      return Promise.resolve(decl?.options ?? []);
    }
    return sendJson<{ options: string[] }>("POST", "/options", {
      dashboard_id: dashboardId,
      parameter,
    }).then((r) => r.options);
  },

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
      headers: writeHeaders(false, "application/octet-stream"),
      body: file,
    });
    if (!res.ok) await failure(res);
    return (await res.json()) as IngestResult;
  },

  // --- asset write ---

  uploadAsset: async (path: string, file: File | Blob): Promise<void> => {
    const res = await fetch(assetUrl(path), {
      method: "PUT",
      headers: writeHeaders(false, file.type || "application/octet-stream"),
      body: file,
    });
    if (!res.ok) await failure(res);
  },

  deleteAsset: (path: string) =>
    fetch(assetUrl(path), { method: "DELETE", headers: writeHeaders(false) }).then(
      async (res) => {
        if (!res.ok) await failure(res);
      },
    ),

  // --- auth & multi-user (doc 13) ---

  me: () => getJson<AuthMe>("/auth/me"),

  setup: (username: string, password: string, email?: string) =>
    sendJson<AuthSession>("POST", "/auth/setup", { username, password, email }),

  login: (username: string, password: string) =>
    sendJson<AuthSession>("POST", "/auth/login", { username, password }),

  logout: () => sendJson<void>("POST", "/auth/logout"),

  changePassword: (currentPassword: string, newPassword: string) =>
    sendJson<void>("POST", "/auth/change-password", {
      current_password: currentPassword,
      new_password: newPassword,
    }),

  // admin-only account management
  listUsers: () => getJson<User[]>("/admin/users"),

  createUser: (username: string, password: string, role: Role, email?: string) =>
    sendJson<User>("POST", "/admin/users", { username, password, role, email }),

  updateUserRole: (id: string, role: Role) =>
    sendJson<void>("PUT", `/admin/users/${encodeURIComponent(id)}`, { role }),

  deleteUser: (id: string) =>
    sendJson<void>("DELETE", `/admin/users/${encodeURIComponent(id)}`),

  resetPassword: (id: string) =>
    sendJson<{ temporary_password: string }>(
      "POST",
      `/admin/users/${encodeURIComponent(id)}/reset-password`,
    ),

  unlockUser: (id: string) =>
    sendJson<void>("POST", `/admin/users/${encodeURIComponent(id)}/unlock`),

  audit: (opts?: { userId?: string; eventKind?: string; limit?: number }) => {
    const p = new URLSearchParams();
    if (opts?.userId) p.set("user_id", opts.userId);
    if (opts?.eventKind) p.set("event_kind", opts.eventKind);
    if (opts?.limit) p.set("limit", String(opts.limit));
    const qs = p.toString();
    return getJson<HistoryEntry[]>(`/admin/audit${qs ? `?${qs}` : ""}`);
  },

  disableAuth: () => sendJson<void>("POST", "/admin/auth/disable"),

  /** Execute a panel's query and decode the Arrow IPC result. */
  executePanel: async (
    dashboardId: string,
    panelId: string,
    parameters: Record<string, unknown>,
  ): Promise<ArrowResult> => {
    // Snapshot mode: results are frozen and inlined; return the baked Arrow.
    const snap = getSnapshot();
    if (snap) {
      const b64 = snap.panels[panelId];
      if (!b64) return EMPTY_RESULT;
      // base64ToBytes returns a fresh, exact-size Uint8Array — its buffer is the
      // whole stream.
      return decodeArrow(base64ToBytes(b64).buffer as ArrayBuffer);
    }
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

/** Absolute URL for an embedded asset (used by image panels). In snapshot mode
 *  this resolves to the inlined data URL so images render with no server. */
export function assetUrl(path: string): string {
  const snap = getSnapshot();
  if (snap) return snap.assets[path] ?? "";
  return `${BASE}/assets/${path.split("/").map(encodeURIComponent).join("/")}`;
}
