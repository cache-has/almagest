import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// The bundle is served by `almagest-server` from `frontend/dist` (embedded via
// rust-embed). In dev, proxy the JSON API to a locally-running `almagest serve`
// so the SPA and the Rust server can run side by side.
export default defineConfig({
  plugins: [svelte()],
  build: {
    outDir: "dist",
    emptyOutDir: true,
    // Cap chunk-size warnings generously — ECharts + Arrow are large but bundled
    // deliberately into the single-file deliverable.
    chunkSizeWarningLimit: 2048,
  },
  server: {
    proxy: {
      "/api": {
        target: "http://127.0.0.1:8080",
        changeOrigin: true,
        ws: true,
      },
    },
  },
});
