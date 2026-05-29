import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri expects a fixed dev port and ignores Vite's HMR websocket when bundling.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      // don't watch the Rust side
      ignored: ["**/src-tauri/**", "**/target/**", "**/examples/**"],
    },
  },
  build: {
    target: "esnext",
    outDir: "dist",
  },
});
