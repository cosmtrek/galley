import { defineConfig } from "vite";

// The share page script, built after the workbench into the same dist/ so the server embeds both.
// A single IIFE with a fixed name: the server serves it at /static/share.js and adds its own version query.
export default defineConfig({
  publicDir: false,
  build: {
    outDir: "dist",
    emptyOutDir: false,
    lib: {
      entry: "src/share.ts",
      formats: ["iife"],
      name: "galleyShare",
      fileName: () => "share.js",
    },
  },
});
