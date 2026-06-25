// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Reactive auth state (doc 13). A small rune-backed store the app gates on:
// whether auth is enabled, whether the file still needs its first admin, and the
// current user. `refreshAuth` re-reads `/auth/me`; the app calls it on load and
// after login / logout / setup / auth-config changes.

import { api } from "./api";
import { getSnapshot } from "./snapshotData";
import type { User } from "./types";

export const auth = $state({
  /** True once the first `/auth/me` probe has resolved. */
  loaded: false,
  /** Whether the file enforces login. */
  enabled: false,
  /** Auth on but no users yet → show the first-admin setup form. */
  needsSetup: false,
  /** The signed-in user, or null when anonymous / auth off. */
  user: null as User | null,
});

/** Re-read the server's auth state into the store. */
export async function refreshAuth(): Promise<void> {
  // A baked snapshot opens over file:// with no server — auth never applies.
  if (getSnapshot()) {
    auth.enabled = false;
    auth.needsSetup = false;
    auth.user = null;
    auth.loaded = true;
    return;
  }
  try {
    const me = await api.me();
    auth.enabled = me.auth_enabled;
    auth.needsSetup = me.needs_setup;
    auth.user = me.user;
  } catch {
    // Treat a failed probe as no-auth so the app still renders.
    auth.enabled = false;
    auth.needsSetup = false;
    auth.user = null;
  } finally {
    auth.loaded = true;
  }
}

/** Log out and refresh the gate. */
export async function doLogout(): Promise<void> {
  await api.logout().catch(() => {});
  await refreshAuth();
}

/** Whether the current user may edit (editor or admin, or auth disabled). */
export function canEdit(): boolean {
  if (!auth.enabled) return true;
  return auth.user?.role === "admin" || auth.user?.role === "editor";
}

/** Whether the current user is an admin (or auth disabled → single-user admin). */
export function isAdmin(): boolean {
  if (!auth.enabled) return true;
  return auth.user?.role === "admin";
}
