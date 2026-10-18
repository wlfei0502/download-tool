import { defineConfig } from "tsup";

export default defineConfig([
  {
    entry: ["src/index.ts"],
    format: ["esm", "cjs"],
    dts: true,
    sourcemap: true,
    clean: true,
    target: "es2020",
    outDir: "dist",
  },
  {
    entry: { "dltool-sdk": "src/index.ts" },
    format: ["iife"],
    globalName: "DownloadToolSdk",
    sourcemap: true,
    target: "es2020",
    outDir: "dist",
    minify: true,
    outExtension: () => ({ js: ".iife.js" }),
    // Also expose as window.DownloadTool for a shorter CDN API.
    footer: {
      js: "globalThis.DownloadTool=globalThis.DownloadToolSdk;",
    },
  },
]);
