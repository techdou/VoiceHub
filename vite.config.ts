import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";
import react from "@vitejs/plugin-react";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const pkg = JSON.parse(
  readFileSync(fileURLToPath(new URL("./package.json", import.meta.url)), "utf-8"),
);

const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [vue(), react()],
  resolve: { alias: { "@": fileURLToPath(new URL("./vendor/sayit/frontend/src", import.meta.url)) } },
  define: {
    __APP_VERSION__: JSON.stringify(pkg.version),
    __SAYIT_DEFAULT_SERVER_URL__: JSON.stringify("https://sayitapp.site"),
  },
  test: {
    environment: "jsdom",
    include: ["src/**/*.{test,spec}.ts", "vendor/sayit/frontend/src/**/*.test.ts"],
  },
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    host: host || false,
    hmr: host
      ? { protocol: "ws", host, port: 5174 }
      : undefined,
    watch: { ignored: ["**/src-tauri/**", "**/crates/**", "**/refs/**", "**/target/**", "**/vendor/sayit/native/**"] },
  },
  build: {
    rollupOptions: { input: {
      main: fileURLToPath(new URL("./index.html", import.meta.url)),
      overlay: fileURLToPath(new URL("./overlay.html", import.meta.url)),
      trayMenu: fileURLToPath(new URL("./tray-menu.html", import.meta.url)),
    } },
    target: "chrome110",
    minify: "esbuild",
    sourcemap: false,
  },
});
