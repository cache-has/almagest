// SPDX-License-Identifier: MIT OR Apache-2.0
//
// A tiny hash-based router — no dependency. Routes:
//   #/                 → dashboard list
//   #/view/:id?query   → read-only viewer (query carries shareable param state)
//   #/edit/:id         → Studio editor

import { readable } from "svelte/store";

export type Route =
  | { name: "list" }
  | { name: "view"; id: string; query: string }
  | { name: "edit"; id: string }
  | { name: "notfound"; path: string };

function parse(hash: string): Route {
  const raw = hash.replace(/^#/, "") || "/";
  const qIndex = raw.indexOf("?");
  const path = qIndex >= 0 ? raw.slice(0, qIndex) : raw;
  const query = qIndex >= 0 ? raw.slice(qIndex + 1) : "";
  const segments = path.split("/").filter(Boolean);
  if (segments.length === 0) return { name: "list" };
  if (segments[0] === "view" && segments[1]) {
    return { name: "view", id: decodeURIComponent(segments[1]), query };
  }
  if (segments[0] === "edit" && segments[1]) {
    return { name: "edit", id: decodeURIComponent(segments[1]) };
  }
  return { name: "notfound", path };
}

export const route = readable<Route>(parse(location.hash), (set) => {
  const handler = () => set(parse(location.hash));
  window.addEventListener("hashchange", handler);
  return () => window.removeEventListener("hashchange", handler);
});

export function navigate(path: string): void {
  location.hash = path;
}
