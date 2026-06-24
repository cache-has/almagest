// SPDX-License-Identifier: MIT OR Apache-2.0
//
// TypeScript mirror of `almagest-format` (`apply`). The Rust formatter is the
// canonical spec; this reproduces it on the client so the viewer renders values
// identically. Keep the two in sync.

import type { Format, DurationUnit } from "./types";

/** Display string for a SQL NULL / missing value. */
export const NULL_DISPLAY = "—";

/** Default ECharts series palette when a theme provides none. */
export const DEFAULT_PALETTE = [
  "#5470c6",
  "#91cc75",
  "#fac858",
  "#ee6666",
  "#73c0de",
  "#3ba272",
  "#fc8452",
  "#9a60b4",
  "#ea7ccc",
];

/** Numeric view of a raw cell value, or null if it has none. */
export function toNumber(value: unknown): number | null {
  if (value === null || value === undefined) return null;
  if (typeof value === "number") return Number.isFinite(value) ? value : null;
  if (typeof value === "bigint") return Number(value);
  if (typeof value === "boolean") return value ? 1 : 0;
  if (typeof value === "string") {
    const n = Number(value.trim());
    return Number.isFinite(n) && value.trim() !== "" ? n : null;
  }
  return null;
}

/** Plain (format-agnostic) rendering of a value. */
export function plain(value: unknown): string {
  if (value === null || value === undefined) return NULL_DISPLAY;
  if (typeof value === "number") return trimFloat(value);
  if (typeof value === "bigint") return value.toString();
  if (typeof value === "boolean") return value ? "true" : "false";
  if (value instanceof Date) return value.toISOString();
  return String(value);
}

/** Apply a format to a raw value, mirroring `Format::apply`. */
export function applyFormat(format: Format | undefined, value: unknown): string {
  return applyFormatAt(format, value, Date.now());
}

export function applyFormatAt(
  format: Format | undefined,
  value: unknown,
  nowMs: number,
): string {
  if (!format) return plain(value);
  switch (format.kind) {
    case "number": {
      const n = toNumber(value);
      return n === null
        ? plain(value)
        : formatNumber(n, format.decimal_places, format.thousands_separator ?? true);
    }
    case "currency": {
      const n = toNumber(value);
      if (n === null) return plain(value);
      const prefix = format.prefix ?? "$";
      const suffix = format.suffix ?? "";
      const body = format.compact
        ? formatCompact(Math.abs(n), format.decimal_places ?? 1)
        : formatNumber(Math.abs(n), format.decimal_places ?? 2, true);
      const sign = n < 0 ? "-" : "";
      return `${sign}${prefix}${body}${suffix}`;
    }
    case "percent": {
      const n = toNumber(value);
      if (n === null) return plain(value);
      return `${(n * 100).toFixed(format.decimal_places ?? 1)}%`;
    }
    case "compact": {
      const n = toNumber(value);
      return n === null ? plain(value) : formatCompact(n, 1);
    }
    case "datetime": {
      const dt = toDate(value);
      if (!dt) return plain(value);
      return format.relative
        ? relativeTime(dt.getTime(), nowMs)
        : strftime(dt, format.format ?? "%Y-%m-%d %H:%M");
    }
    case "duration": {
      const n = toNumber(value);
      return n === null ? plain(value) : humanizeDuration(n, format.unit ?? "seconds");
    }
    case "enum": {
      const key = plain(value);
      return format.values[key] ?? key;
    }
    case "custom":
      return format.template.replaceAll("${value}", plain(value));
  }
}

// --- helpers -----------------------------------------------------------------

function trimFloat(n: number): string {
  if (Number.isInteger(n)) return n.toString();
  // Match Rust's trim of trailing zeros without forcing scientific notation.
  return n.toString();
}

function groupThousands(intPart: string): string {
  const neg = intPart.startsWith("-");
  const digits = neg ? intPart.slice(1) : intPart;
  const grouped = digits.replace(/\B(?=(\d{3})+(?!\d))/g, ",");
  return neg ? `-${grouped}` : grouped;
}

function formatNumber(
  n: number,
  decimalPlaces: number | undefined,
  thousands: boolean,
): string {
  const rendered = decimalPlaces === undefined ? trimFloat(n) : n.toFixed(decimalPlaces);
  if (!thousands) return rendered;
  const [intPart, fracPart] = rendered.split(".");
  const grouped = groupThousands(intPart);
  return fracPart !== undefined ? `${grouped}.${fracPart}` : grouped;
}

function formatCompact(n: number, decimalPlaces: number): string {
  const abs = Math.abs(n);
  const sign = n < 0 ? "-" : "";
  const units: [number, string][] = [
    [1e12, "T"],
    [1e9, "B"],
    [1e6, "M"],
    [1e3, "K"],
  ];
  for (const [threshold, suffix] of units) {
    if (abs >= threshold) {
      const scaled = abs / threshold;
      return `${sign}${stripTrailingZeros(scaled.toFixed(decimalPlaces))}${suffix}`;
    }
  }
  return `${sign}${stripTrailingZeros(abs.toFixed(decimalPlaces))}`;
}

function stripTrailingZeros(s: string): string {
  if (!s.includes(".")) return s;
  return s.replace(/\.?0+$/, "");
}

function toDate(value: unknown): Date | null {
  if (value instanceof Date) return value;
  if (typeof value === "bigint") return new Date(Number(value));
  if (typeof value === "number") return new Date(value);
  if (typeof value === "string") {
    const d = new Date(value);
    return Number.isNaN(d.getTime()) ? null : d;
  }
  return null;
}

const PAD = (n: number) => String(n).padStart(2, "0");

/** A small strftime subset covering the patterns the format spec uses. */
function strftime(dt: Date, pattern: string): string {
  const map: Record<string, string> = {
    "%Y": String(dt.getFullYear()),
    "%m": PAD(dt.getMonth() + 1),
    "%d": PAD(dt.getDate()),
    "%H": PAD(dt.getHours()),
    "%M": PAD(dt.getMinutes()),
    "%S": PAD(dt.getSeconds()),
  };
  return pattern.replace(/%[YmdHMS]/g, (m) => map[m] ?? m);
}

function relativeTime(thenMs: number, nowMs: number): string {
  const diff = nowMs - thenMs;
  const future = diff < 0;
  const abs = Math.abs(diff);
  const units: [number, string][] = [
    [86400000 * 365, "year"],
    [86400000 * 30, "month"],
    [86400000 * 7, "week"],
    [86400000, "day"],
    [3600000, "hour"],
    [60000, "minute"],
    [1000, "second"],
  ];
  for (const [ms, name] of units) {
    if (abs >= ms) {
      const count = Math.floor(abs / ms);
      const plural = count === 1 ? name : `${name}s`;
      return future ? `in ${count} ${plural}` : `${count} ${plural} ago`;
    }
  }
  return "just now";
}

function humanizeDuration(amount: number, unit: DurationUnit): string {
  const seconds =
    unit === "milliseconds"
      ? amount / 1000
      : unit === "minutes"
        ? amount * 60
        : unit === "hours"
          ? amount * 3600
          : amount;
  let rem = Math.floor(seconds);
  const h = Math.floor(rem / 3600);
  rem -= h * 3600;
  const m = Math.floor(rem / 60);
  rem -= m * 60;
  const parts: string[] = [];
  if (h) parts.push(`${h}h`);
  if (m) parts.push(`${m}m`);
  if (rem || parts.length === 0) parts.push(`${rem}s`);
  return parts.join(" ");
}
