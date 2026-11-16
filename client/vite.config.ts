import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// M4: the daemon hosts the built SPA at `/` (the default console); base
// keeps asset URLs rooted there. File names are hash-free so the daemon can
// include_str!/include_bytes! them at compile time without a build-time
// manifest.
export default defineConfig({
  plugins: [svelte()],
  base: "/",
  build: { outDir: "dist", emptyOutDir: true, assetsDir: "assets", rollupOptions: { output: { entryFileNames: "assets/app.js", chunkFileNames: "assets/[name].js", assetFileNames: "assets/[name][extname]" } } },
});
