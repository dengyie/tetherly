# Plan — desktop client redesign

Spec: `docs/superpowers/specs/2026-11-22-desktop-client-redesign-design.md`

1. Tooling: add `tailwindcss` + `@tailwindcss/vite`, `src/app.css`, vite
   plugin, inline-asset limit; verify build still emits only
   `assets/app.js` + `assets/index.css`.
2. `src/lib/ui/*`: port icons, StatusIndicator, Glass, ThemeToggle/theme,
   toast store + ToastStack, EmptyState, ProgressBar, AnimatedCounter,
   CommandPalette, Switch. Tests: `src/lib/ui/ui.test.ts`.
3. `Api`: add `layout()`, `topology()`, `linkEdge()`, `unlinkEdge()`,
   `syncTopology()`, `patchTopologyConfig()`, `patchTopologyPolicy()`,
   `disconnectDevice()`.
4. Shell: `App.svelte` sidebar + header (StatusIndicator, `.link`,
   ThemeToggle, ⌘K) + CommandPalette + ToastStack; `main.ts` applies the
   stored theme before mount.
5. Routes: Overview, Devices, Transfers, Inbox, Activity, Settings,
   Diagnostics rebuilt with the kit; Tray restyled (contract unchanged).
6. 布局 editor (`Layout.svelte`) with tests `src/lib/layout.test.ts`
   driving MockTransport: link / unlink / config / policy calls.
7. e2e mock server: add a `layout.changed` frame.
8. Verify: `pnpm check && pnpm test && pnpm build && test -z "$(git status
   --porcelain -- dist)"` is NOT expected to be zero-diff before commit
   (dist changes are the point); run `pnpm build` then commit `dist/` with
   the sources, re-run `build:ci` to prove zero diff.
