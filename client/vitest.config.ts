import { defineConfig } from "vitest/config";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig({
  plugins: [svelte(), tailwindcss()],
  resolve: {
    // Svelte 5 ships server-first exports; component tests need the client
    // build or mount() is unavailable in jsdom.
    conditions: ["browser"],
  },
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.ts"],
    // @testing-library/svelte auto-cleanup hooks onto global afterEach.
    globals: true,
    css: false,
  },
});
