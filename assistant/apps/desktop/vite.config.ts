/// <reference types="vitest" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// Puerto fijo: tauri.conf.json apunta a http://localhost:1420
export default defineConfig({
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  server: { port: 1420, strictPort: true, host: "127.0.0.1" },
  envPrefix: ["VITE_", "TAURI_ENV_"],
  build: { target: "es2022", sourcemap: false, chunkSizeWarningLimit: 1000 },
  test: { environment: "jsdom", globals: false, include: ["src/**/*.test.{ts,tsx}"], setupFiles: ["src/test-setup.ts"] },
});
