/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import { fileURLToPath } from "node:url";

const srcOrdner = fileURLToPath(new URL(".", import.meta.url));
const hauptOrdner = fileURLToPath(new URL("..", import.meta.url));

// Vite-Konfiguration für das Frontend. Der Quellordner ist src/,
// gebaut wird nach dist/ (wird von Tauri eingebunden).
export default defineConfig({
  root: srcOrdner,
  publicDir: fileURLToPath(new URL("../public", import.meta.url)),
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: "127.0.0.1",
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    outDir: fileURLToPath(new URL("../dist", import.meta.url)),
    emptyOutDir: true,
    target: "es2022",
    sourcemap: false,
  },
  test: {
    root: hauptOrdner,
    include: ["tests/**/*.test.ts"],
    environment: "happy-dom",
  },
});
