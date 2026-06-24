// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Client-side parameter helpers: seed initial values from declared defaults, and
// resolve click-to-filter action tokens ($row.<col> / $column / $selection.<col>)
// against a clicked row — mirroring `almagest-query::interpolate_action_value`.

import type { Parameter } from "./types";

/** Seed parameter values from declared defaults (URL persistence is deferred). */
export function initialParamValues(params: Parameter[]): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const p of params) {
    out[p.id] = p.default !== undefined && p.default !== null ? p.default : emptyFor(p);
  }
  return out;
}

function emptyFor(p: Parameter): unknown {
  switch (p.kind) {
    case "number":
      return p.min ?? 0;
    case "boolean":
      return false;
    case "multiselect":
      return [];
    case "select":
      return p.options?.[0] ?? "";
    case "daterange":
      return { start: "", end: "" };
    default:
      return "";
  }
}

const WHOLE_TOKEN = /^\$(row|selection)\.([A-Za-z0-9_]+)$|^\$column$/;
const EMBEDDED_TOKEN = /\$(?:row|selection)\.([A-Za-z0-9_]+)|\$column/g;

/**
 * Resolve a `set_parameter` action value against a clicked row. A value that is
 * exactly one token returns the raw (typed) cell value; a token embedded in a
 * larger string does textual substitution.
 */
export function interpolateActionValue(
  value: unknown,
  row: Record<string, unknown>,
  column?: string,
): unknown {
  if (typeof value !== "string") return value;

  const whole = value.match(WHOLE_TOKEN);
  if (whole) {
    if (value === "$column") return column ?? "";
    const col = whole[2];
    return row[col] ?? "";
  }

  return value.replace(EMBEDDED_TOKEN, (match, col) => {
    if (match === "$column") return String(column ?? "");
    return String(row[col] ?? "");
  });
}
