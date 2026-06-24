// SPDX-License-Identifier: MIT OR Apache-2.0
//
// TypeScript mirror of `almagest-query::urlstate` — encode the current parameter
// values into a shareable query string and decode them back, typed by the
// declarations. Keeping the wire format identical to the Rust side means a link
// shared from the viewer decodes the same whether the next open is served by
// Almagest or by the interactive-HTML export.
//
// Encoding is decl-aware so the query string stays human-readable
// (`region=EU`, not a JSON blob); a `multiselect` repeats its key, a
// `daterange` encodes its `preset` or its `start`/`end`. Parameters declared
// `persist: "none"` are omitted from the URL.

import type { Parameter } from "./types";

/** Build the `key=value&…` query string (no leading `?`) for the given values. */
export function encodeUrlState(
  values: Record<string, unknown>,
  decls: Parameter[],
): string {
  const parts: string[] = [];
  for (const decl of decls) {
    if (decl.persist === "none") continue;
    const value = values[decl.id];
    if (value === undefined || value === null || value === "") continue;

    if (Array.isArray(value)) {
      for (const item of value) {
        const s = scalarToString(item);
        if (s !== null) parts.push(pair(decl.id, s));
      }
    } else if (typeof value === "object") {
      const obj = value as Record<string, unknown>;
      const preset = typeof obj.preset === "string" ? obj.preset : null;
      if (preset) {
        parts.push(pair(`${decl.id}.preset`, preset));
      } else {
        if (typeof obj.start === "string" && obj.start) parts.push(pair(`${decl.id}.start`, obj.start));
        if (typeof obj.end === "string" && obj.end) parts.push(pair(`${decl.id}.end`, obj.end));
      }
    } else {
      const s = scalarToString(value);
      if (s !== null) parts.push(pair(decl.id, s));
    }
  }
  return parts.join("&");
}

/**
 * Decode a query string into raw parameter values, typed by `decls`. Keys not
 * matching any declaration are ignored; a value that doesn't parse for its kind
 * is dropped (so the caller falls back to the declared default).
 */
export function decodeUrlState(query: string, decls: Parameter[]): Record<string, unknown> {
  const raw = parseQuery(query);
  const out: Record<string, unknown> = {};

  for (const decl of decls) {
    const id = decl.id;
    switch (decl.kind) {
      case "multiselect": {
        const vals = raw.get(id);
        if (vals) out[id] = [...vals];
        break;
      }
      case "daterange": {
        const preset = raw.get(`${id}.preset`)?.[0];
        if (preset) {
          out[id] = { preset };
        } else {
          const start = raw.get(`${id}.start`)?.[0];
          const end = raw.get(`${id}.end`)?.[0];
          if (start && end) out[id] = { start, end };
        }
        break;
      }
      case "number": {
        const s = raw.get(id)?.[0];
        if (s !== undefined) {
          const n = Number(s);
          if (Number.isFinite(n)) out[id] = n;
        }
        break;
      }
      case "boolean": {
        const s = raw.get(id)?.[0];
        if (s === "true") out[id] = true;
        else if (s === "false") out[id] = false;
        break;
      }
      default: {
        // text | date | select
        const s = raw.get(id)?.[0];
        if (s !== undefined) out[id] = s;
        break;
      }
    }
  }
  return out;
}

/** Layer parameter sources: URL state overrides seeded defaults. */
export function layeredState(
  url: Record<string, unknown>,
  defaults: Record<string, unknown>,
): Record<string, unknown> {
  return { ...defaults, ...url };
}

function pair(key: string, value: string): string {
  return `${pctEncode(key)}=${pctEncode(value)}`;
}

function scalarToString(v: unknown): string | null {
  if (typeof v === "string") return v;
  if (typeof v === "number") return String(v);
  if (typeof v === "boolean") return String(v);
  return null;
}

/** Parse `key=value&…` into key → ordered values (repeated keys accumulate). */
function parseQuery(query: string): Map<string, string[]> {
  const out = new Map<string, string[]>();
  const q = query.startsWith("?") ? query.slice(1) : query;
  if (!q) return out;
  for (const pairStr of q.split("&")) {
    if (!pairStr) continue;
    const eq = pairStr.indexOf("=");
    const k = pctDecode(eq >= 0 ? pairStr.slice(0, eq) : pairStr);
    const v = eq >= 0 ? pctDecode(pairStr.slice(eq + 1)) : "";
    const arr = out.get(k);
    if (arr) arr.push(v);
    else out.set(k, [v]);
  }
  return out;
}

/** Percent-encode a query component, keeping the RFC 3986 unreserved set literal. */
function pctEncode(s: string): string {
  let out = "";
  for (const byte of new TextEncoder().encode(s)) {
    if (
      (byte >= 0x41 && byte <= 0x5a) || // A-Z
      (byte >= 0x61 && byte <= 0x7a) || // a-z
      (byte >= 0x30 && byte <= 0x39) || // 0-9
      byte === 0x2d || // -
      byte === 0x5f || // _
      byte === 0x2e || // .
      byte === 0x7e // ~
    ) {
      out += String.fromCharCode(byte);
    } else {
      out += "%" + byte.toString(16).toUpperCase().padStart(2, "0");
    }
  }
  return out;
}

/** Decode percent-escapes; `+` is treated as a space (form convention). */
function pctDecode(s: string): string {
  const bytes: number[] = [];
  for (let i = 0; i < s.length; ) {
    const c = s.charCodeAt(i);
    if (c === 0x25 /* % */ && i + 2 < s.length) {
      const h = parseInt(s.slice(i + 1, i + 3), 16);
      if (Number.isFinite(h)) {
        bytes.push(h);
        i += 3;
        continue;
      }
      bytes.push(c);
      i += 1;
    } else if (c === 0x2b /* + */) {
      bytes.push(0x20);
      i += 1;
    } else {
      bytes.push(c);
      i += 1;
    }
  }
  return new TextDecoder().decode(new Uint8Array(bytes));
}
