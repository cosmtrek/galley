import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

const backend = process.env.GALLEY_BACKEND ?? "http://127.0.0.1:7860";

export default defineConfig({
  base: "/app/",
  plugins: [svelte()],
  server: {
    // The reports page bundles the example report; allow just that directory, not the repo (data/ holds secrets).
    fs: { allow: [".", "../examples"] },
    proxy: {
      "/api": backend,
      "/a": backend,
      "/s": backend,
      "/static": backend,
    },
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
  },
  test: {
    include: ["src/**/*.test.ts"],
  },
});
