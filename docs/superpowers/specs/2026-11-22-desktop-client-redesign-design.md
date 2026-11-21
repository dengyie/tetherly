# Desktop client redesign (Windows / macOS) — design

Date: 2026-11-22 · Branch: `feat/desktop-client-redesign` · Scope: `client/` only

## Intent

The Tauri 2 + Svelte 5 desktop client works but reads as a debugging console:
flat text lists, no visual hierarchy, no real 布局 (screen arrangement) editor,
a bare tray panel. The user wants a client that "满足要求": a modern,
first-party-feeling desktop UI on both platforms, assembled from the
`awesome-ui` component kit rather than hand-rolled widgets.

## Goals

- Modern shell: glass sidebar + content area, status badge for the daemon
  link, theme toggle (auto/light/dark), ⌘K command palette for navigation.
- Every page rebuilt with cards, badges, empty states, progress bars, toasts.
- 布局 page becomes a visual arrangement editor driven by the existing
  `/api/v1/topology/*` write API (link / unlink edges, crossing config,
  policy, sync).
- Tray panel restyled, same wording/testid contract as the Rust tray fold.
- Works offline under the Tauri CSP (no CDN): Tailwind is bundled.

## Non-goals / anti-goals

- No daemon or Rust shell changes (`src-tauri`, `uihttp/next.rs` untouched).
- No change to the transport / store / Api layering (ADR-139). Pages still
  never fetch on their own; the event stream stays the single writer.
- No release / deploy in this change.
- Anti-goal: breaking the embedded-asset contract (`assets/app.js`,
  `assets/index.css`, `index.html` are the only files the daemon serves).

## Constraints

- Daemon embeds exactly `dist/index.html`, `dist/assets/app.js`,
  `dist/assets/index.css`; `dist/` is committed and CI requires zero diff
  after `pnpm build`. Images therefore must inline (data URI) — CSP allows
  `img-src 'self' data:`.
- Existing test contracts (vitest + Playwright) keep passing: nav buttons
  named 概览/设备/传输/收件箱/动态/设置/布局/诊断, `main h1` per page,
  `header .link` empty when live, tray testids and wording, device/connect
  flows.
- awesome-ui ships only React / Vue / Vanilla; components are ported to
  Svelte 5 preserving their Tailwind class structure. Icons only through the
  ported `uiIcon` set (no emoji / unicode glyphs / ad-hoc SVG).

## Approaches considered

1. **Keep the existing hand-written CSS, polish it.** Cheapest, but keeps
   every page bespoke and gives no component system; rejected.
2. **Load awesome-ui Vanilla Web Components directly inside Svelte.** Zero
   port effort, but the kit's classes need Tailwind at runtime anyway, Web
   Components fight Svelte reactivity (attribute-string props, innerHTML
   re-renders) and are awkward under jsdom tests; rejected.
3. **Tailwind v4 bundled + awesome-ui components ported to Svelte 5
   (chosen).** One stylesheet, native Svelte props/snippets, testable in
   jsdom, same visual language as the kit.

## Decisions

- `tailwindcss@4` + `@tailwindcss/vite`; entry `src/app.css` with
  `@custom-variant dark (&:where(.dark, .dark *))` so the ThemeToggle's
  `html.dark` drives dark mode. Existing `--accent` etc. tokens kept.
- Ported components live in `src/lib/ui/`: `icons.ts` (+`Icon.svelte`),
  `StatusIndicator`, `Glass` (LiquidGlass), `ThemeToggle` (+`theme.ts`),
  `ToastStack` (+`toast.ts` store), `EmptyState`, `ProgressBar`,
  `AnimatedCounter`, `CommandPalette`, `Switch`.
- Theme "auto" follows `prefers-color-scheme` only (the kit's hour-of-day
  heuristic is dropped for a desktop utility).
- Brand mark: the project's own `src-tauri/icons/32x32.png`, inlined.
- 布局 editor: local display drawn to aspect ratio, four edge slots; assign
  by click (picker) or drag from the device list → `POST /topology/links`;
  remove → `POST /topology/unlinks`; sliders/toggles → `PATCH
  /topology/config`; policy switches → `PATCH /topology/policy`; 同步 →
  `POST /topology/sync`. The daemon already publishes `layout.changed`
  after each (`apiv1/project.rs` maps TopologyChanged/TopologyPolicyChanged
  → Layout invalidation), so the page stays store-driven.
- Settings page gains switches for `advertise` / `reconnect` /
  `auto_accept` (all in `SettingsPatch`).
- Page-level `notice` strings become toasts.

## Left to the builder

Exact spacing, icon choices per nav item, slider ranges (kept within the
daemon's accepted u32 domain), responsive collapse of the sidebar below the
Tauri window minimum (720 px).
