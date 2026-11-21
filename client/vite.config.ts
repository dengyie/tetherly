import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import tailwindcss from "@tailwindcss/vite";

// M4: the daemon hosts the built SPA at `/` (the default console); base
// keeps asset URLs rooted there. File names are hash-free so the daemon can
// include_str!/include_bytes! them at compile time without a build-time
// manifest. The daemon serves exactly index.html + assets/app.js +
// assets/index.css, so every image must inline (CSP allows `img-src data:`).
export default defineConfig({
  plugins: [svelte(), tailwindcss()],
  base: "/",
  build: {
    outDir: "dist",
    emptyOutDir: true,
    assetsDir: "assets",
    assetsInlineLimit: 64 * 1024,
    cssCodeSplit: false,
    rollupOptions: {
      output: {
        entryFileNames: "assets/app.js",
        chunkFileNames: "assets/[name].js",
        // The daemon embeds exactly `assets/index.css`; with cssCodeSplit off
        // rollup would otherwise call the single stylesheet `style.css`.
        assetFileNames: (info) =>
          info.names?.some((n) => n.endsWith(".css")) ? "assets/index.css" : "assets/[name][extname]",
      },
    },
  },
});
