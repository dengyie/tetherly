import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// daemon hosts the built SPA at /next/; base keeps asset URLs relative.
// File names are hash-free so the daemon can include_str!/include_bytes! them
// at compile time without a build-time manifest.
export default defineConfig({
  plugins: [svelte()],
  base: "/next/",
  build: { outDir: "dist", emptyOutDir: true, assetsDir: "assets", rollupOptions: { output: { entryFileNames: "assets/app.js", chunkFileNames: "assets/[name].js", assetFileNames: "assets/[name][extname]" } } },
});
