import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

const backend = process.env.GALLEY_BACKEND ?? "http://127.0.0.1:7860";

export default defineConfig({
  base: "/app/",
  plugins: [svelte()],
  server: {
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
