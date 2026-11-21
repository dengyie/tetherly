// Theme state ported from awesome-ui/vanilla/ThemeToggle.js. "auto" follows
// the OS (prefers-color-scheme) — the kit's hour-of-day heuristic is dropped
// for a desktop utility that should match the host window chrome.
import { writable, get } from "svelte/store";

export type ThemeMode = "auto" | "light" | "dark";
export const STORAGE_KEY = "tetherly-theme";

function safeGet(): ThemeMode {
  try {
    const v = localStorage.getItem(STORAGE_KEY);
    return v === "light" || v === "dark" || v === "auto" ? v : "auto";
  } catch {
    return "auto";
  }
}

export const themeMode = writable<ThemeMode>(safeGet());

export function resolveTheme(mode: ThemeMode): "light" | "dark" {
  if (mode === "light" || mode === "dark") return mode;
  if (typeof window !== "undefined" && typeof window.matchMedia === "function") {
    return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
  }
  return "light";
}

export function applyTheme(mode: ThemeMode = get(themeMode)): "light" | "dark" {
  const resolved = resolveTheme(mode);
  const root = document.documentElement;
  root.dataset.theme = mode;
  root.dataset.resolvedTheme = resolved;
  root.style.colorScheme = resolved;
  root.classList.toggle("dark", resolved === "dark");
  try {
    localStorage.setItem(STORAGE_KEY, mode);
  } catch {
    /* storage may be unavailable (private mode) */
  }
  themeMode.set(mode);
  return resolved;
}

let listening = false;
/** Apply the stored mode now and keep "auto" in sync with OS changes. */
export function initTheme(): void {
  applyTheme(get(themeMode));
  if (listening || typeof window === "undefined" || typeof window.matchMedia !== "function") return;
  listening = true;
  const mq = window.matchMedia("(prefers-color-scheme: dark)");
  const onChange = () => {
    if (get(themeMode) === "auto") applyTheme("auto");
  };
  if (typeof mq.addEventListener === "function") mq.addEventListener("change", onChange);
}
