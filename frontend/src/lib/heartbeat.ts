// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Desktop lifecycle: ping the server on an interval so it knows a tab is still
// open. When every tab closes the pings stop and the server's watchdog exits the
// process (the browser-serve approximation of "the app window was closed"). Only
// started when the server advertises `heartbeat_enabled` (desktop mode).

const INTERVAL_MS = 5000;

export function startHeartbeat(): void {
  const ping = () => {
    // Fire-and-forget; keepalive lets the final ping survive page unload.
    fetch("/api/almagest/heartbeat", { method: "POST", keepalive: true }).catch(() => {});
  };
  ping();
  const timer = setInterval(() => {
    // Don't count a backgrounded tab as gone — only ping while visible, but a
    // hidden tab still pings occasionally via the interval to avoid a false exit.
    ping();
  }, INTERVAL_MS);
  // Ping immediately when the tab becomes visible again (after sleep/restore).
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "visible") ping();
  });
  window.addEventListener("beforeunload", () => clearInterval(timer));
}
