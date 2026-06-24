// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Decode an Arrow IPC stream (the panel-execute response) into plain JS rows the
// panel renderers consume. The server returns
// `application/vnd.apache.arrow.stream`; this is the only place we touch Arrow.

import { tableFromIPC } from "apache-arrow";

export interface ArrowField {
  name: string;
  type: string;
}

export interface ArrowResult {
  fields: ArrowField[];
  rows: Record<string, unknown>[];
  rowCount: number;
}

/** Decode IPC bytes into fields + plain row objects (BigInt coerced to number). */
export function decodeArrow(buffer: ArrayBuffer): ArrowResult {
  const table = tableFromIPC(new Uint8Array(buffer));
  const fields: ArrowField[] = table.schema.fields.map((f) => ({
    name: f.name,
    type: String(f.type),
  }));

  const rows: Record<string, unknown>[] = [];
  for (let i = 0; i < table.numRows; i++) {
    const row = table.get(i);
    if (row) rows.push(normalizeRow(row.toJSON()));
  }
  return { fields, rows, rowCount: table.numRows };
}

/** Coerce Arrow's BigInt (Int64) cells to numbers so formatting/charting work. */
function normalizeRow(obj: Record<string, unknown>): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(obj)) {
    out[k] = typeof v === "bigint" ? Number(v) : v;
  }
  return out;
}
