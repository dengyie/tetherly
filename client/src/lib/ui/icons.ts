// Ported from awesome-ui/vanilla/UiIcon.js. Icon geometry follows the Tabler
// Icons visual conventions (MIT licensed): https://github.com/tabler/tabler-icons
// The kit rule is "icons only through uiIcon" — the extra names below extend
// the same set in the same style for this app's domain (keyboard/mouse mesh).
const ICONS: Record<string, string> = {
  "alert-triangle": '<path d="M12 9v4"/><path d="M12 17h.01"/><path d="M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0Z"/>',
  "arrow-down": '<path d="m6 9 6 6 6-6"/>',
  "arrow-left": '<path d="m15 6-6 6 6 6"/>',
  "arrow-right": '<path d="m9 6 6 6-6 6"/>',
  "arrow-up": '<path d="m6 15 6-6 6 6"/>',
  check: '<path d="m5 12 4 4L19 6"/>',
  "check-circle": '<circle cx="12" cy="12" r="9"/><path d="m8 12 2.5 2.5L16 9"/>',
  "chevron-down": '<path d="m6 9 6 6 6-6"/>',
  "chevron-left": '<path d="m14 6-6 6 6 6"/>',
  "chevron-right": '<path d="m10 6 6 6-6 6"/>',
  "chevron-up": '<path d="m6 15 6-6 6 6"/>',
  "circle-x": '<circle cx="12" cy="12" r="9"/><path d="m9 9 6 6m0-6-6 6"/>',
  clock: '<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 3"/>',
  code: '<path d="m8 9-3 3 3 3m8-6 3 3-3 3M14 5l-4 14"/>',
  copy: '<rect x="8" y="8" width="11" height="11" rx="2"/><path d="M16 8V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h2"/>',
  cpu: '<rect x="5" y="5" width="14" height="14" rx="2"/><path d="M9 9h6v6H9zM9 1v4m6-4v4M9 19v4m6-4v4M1 9h4m-4 6h4M19 9h4m-4 6h4"/>',
  eye: '<path d="M2.5 12s3.5-6 9.5-6 9.5 6 9.5 6-3.5 6-9.5 6-9.5-6Z"/><circle cx="12" cy="12" r="2.5"/>',
  "external-link": '<path d="M14 5h5v5M19 5l-8 8M19 14v3a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2h3"/>',
  globe: '<circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3a14 14 0 0 1 0 18M12 3a14 14 0 0 0 0 18"/>',
  "git-branch": '<path d="M6 3v12a3 3 0 0 0 3 3h6a3 3 0 0 0 3-3V9"/><circle cx="6" cy="3" r="2"/><circle cx="18" cy="7" r="2"/><circle cx="12" cy="18" r="2"/>',
  image: '<rect x="3" y="4" width="18" height="16" rx="2"/><circle cx="8.5" cy="9" r="1.5"/><path d="m21 15-4-4L5 20"/>',
  layers: '<path d="m12 3 9 5-9 5-9-5 9-5Zm-9 9 9 5 9-5m-18 4 9 5 9-5"/>',
  lock: '<rect x="5" y="10" width="14" height="11" rx="2"/><path d="M8 10V7a4 4 0 0 1 8 0v3"/>',
  loader: '<path d="M12 3v3m0 12v3M3 12h3m12 0h3M5.6 5.6l2.1 2.1m8.6 8.6 2.1 2.1m0-12.8-2.1 2.1m-8.6 8.6-2.1 2.1"/>',
  minus: '<path d="M5 12h14"/>',
  moon: '<path d="M20 15.5A8 8 0 0 1 8.5 4 8 8 0 0 0 20 15.5Z"/>',
  monitor: '<rect x="3" y="4" width="18" height="13" rx="2"/><path d="M8 21h8m-4-4v4"/>',
  paperclip: '<path d="m20 11-7.5 7.5a5 5 0 0 1-7-7L13 4a3.5 3.5 0 0 1 5 5l-7.5 7.5a2 2 0 0 1-3-3L14 7"/>',
  plus: '<path d="M12 5v14M5 12h14"/>',
  refresh: '<path d="M20 11a8 8 0 0 0-14.5-4L3 10m0-5v5h5M4 13a8 8 0 0 0 14.5 4L21 14m0 5v-5h-5"/>',
  search: '<circle cx="10" cy="10" r="7"/><path d="m21 21-6-6"/>',
  sliders: '<path d="M4 8h4m4 0h8M4 16h8m4 0h4M8 6v4m8 4v4"/>',
  sparkles: '<path d="m12 3-1.2 4.8L6 9l4.8 1.2L12 15l1.2-4.8L18 9l-4.8-1.2L12 3Z"/>',
  square: '<rect x="6" y="6" width="12" height="12" rx="1"/>',
  sun: '<circle cx="12" cy="12" r="4"/><path d="M12 2v2m0 16v2M2 12h2m16 0h2"/>',
  terminal: '<path d="m4 17 6-6-6-6m8 14h8"/>',
  "thumbs-down": '<path d="M7 10v10H5a2 2 0 0 1-2-2v-6a2 2 0 0 1 2-2h2Zm0 0 3-7a2 2 0 0 1 2 2v3h6a2 2 0 0 1 2 2l-1 7a2 2 0 0 1-2 2h-7l-3-3"/>',
  "thumbs-up": '<path d="M7 14V4a2 2 0 0 1 2-2h2l1 6h6a2 2 0 0 1 2 2l-1 7a2 2 0 0 1-2 2H9l-2-3H5a2 2 0 0 1-2-2v-6a2 2 0 0 1 2-2h2"/>',
  tool: '<path d="M14.7 6.3a4 4 0 0 0-5.4 5.4L4 17l3 3 5.3-5.3a4 4 0 0 0 5.4-5.4l-2.2 2.2-2.7-.7-.7-2.7 2.2-2.2Z"/>',
  user: '<path d="M8 7a4 4 0 1 0 8 0 4 4 0 0 0-8 0"/><path d="M6 21v-2a4 4 0 0 1 4-4h4a4 4 0 0 1 4 4v2"/>',
  x: '<path d="m6 6 12 12M18 6 6 18"/>',
  zap: '<path d="M13 3 4 14h7l-1 7 9-11h-7l1-7Z"/>',
  // ---- app additions (same Tabler geometry conventions) ----
  activity: '<path d="M3 12h4l3-8 4 16 3-8h4"/>',
  bell: '<path d="M10 5a2 2 0 1 1 4 0 7 7 0 0 1 4 6v3a4 4 0 0 0 2 3H4a4 4 0 0 0 2-3v-3a7 7 0 0 1 4-6"/><path d="M9 17v1a3 3 0 0 0 6 0v-1"/>',
  download: '<path d="M4 17v2a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-2"/><path d="m7 11 5 5 5-5M12 4v12"/>',
  file: '<path d="M14 3v4a1 1 0 0 0 1 1h4"/><path d="M17 21H7a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h7l5 5v11a2 2 0 0 1-2 2Z"/>',
  home: '<path d="m5 12-2 0 9-9 9 9h-2"/><path d="M5 12v7a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2v-7"/><path d="M9 21v-6a2 2 0 0 1 2-2h2a2 2 0 0 1 2 2v6"/>',
  inbox: '<rect x="4" y="4" width="16" height="16" rx="2"/><path d="M4 13h3l3 3h4l3-3h3"/>',
  key: '<circle cx="8" cy="15" r="4"/><path d="m10.85 12.15 7.65-7.65M18 5l2 2M15 8l2 2"/>',
  keyboard: '<rect x="2" y="6" width="20" height="12" rx="2"/><path d="M6 10h0m4 0h0m4 0h0m4 0h0M6 14h0m4 0h8"/>',
  layout: '<rect x="4" y="4" width="16" height="16" rx="2"/><path d="M4 12h16M12 12v8"/>',
  link: '<path d="M10 14a3.5 3.5 0 0 0 5 0l4-4a3.5 3.5 0 0 0-5-5l-.5.5"/><path d="M14 10a3.5 3.5 0 0 0-5 0l-4 4a3.5 3.5 0 0 0 5 5l.5-.5"/>',
  message: '<path d="M4 21V8a3 3 0 0 1 3-3h10a3 3 0 0 1 3 3v6a3 3 0 0 1-3 3H8l-4 4"/><path d="M8 9h8m-8 4h6"/>',
  mouse: '<rect x="6" y="3" width="12" height="18" rx="6"/><path d="M12 7v4"/>',
  pause: '<rect x="6" y="5" width="4" height="14" rx="1"/><rect x="14" y="5" width="4" height="14" rx="1"/>',
  play: '<path d="M7 4v16l13-8z"/>',
  power: '<path d="M7 6a7.75 7.75 0 1 0 10 0"/><path d="M12 4v8"/>',
  send: '<path d="m10 14 11-11"/><path d="M21 3 14.5 21a.55.55 0 0 1-1 0L10 14l-7-3.5a.55.55 0 0 1 0-1L21 3"/>',
  settings: '<path d="M10.3 4.3c.4-1.7 3-1.7 3.4 0a1.7 1.7 0 0 0 2.6 1.1c1.5-.9 3.3.9 2.4 2.4a1.7 1.7 0 0 0 1.1 2.6c1.7.4 1.7 3 0 3.4a1.7 1.7 0 0 0-1.1 2.6c.9 1.5-.9 3.3-2.4 2.4a1.7 1.7 0 0 0-2.6 1.1c-.4 1.7-3 1.7-3.4 0a1.7 1.7 0 0 0-2.6-1.1c-1.5.9-3.3-.9-2.4-2.4a1.7 1.7 0 0 0-1.1-2.6c-1.7-.4-1.7-3 0-3.4a1.7 1.7 0 0 0 1.1-2.6c-.9-1.5.9-3.3 2.4-2.4a1.7 1.7 0 0 0 2.6-1.1"/><circle cx="12" cy="12" r="3"/>',
  shield: '<path d="M12 3a12 12 0 0 0 8.5 3A12 12 0 0 1 12 21 12 12 0 0 1 3.5 6 12 12 0 0 0 12 3"/>',
  "shield-off": '<path d="M17.7 17.7A12 12 0 0 1 12 21 12 12 0 0 1 3.5 6a12 12 0 0 0 4.9-1.2M9.3 3.7A12 12 0 0 0 12 3a12 12 0 0 0 8.5 3 12 12 0 0 1-1 6.4"/><path d="M3 3l18 18"/>',
  trash: '<path d="M4 7h16M10 11v6m4-6v6M5 7l1 12a2 2 0 0 0 2 2h8a2 2 0 0 0 2-2l1-12M9 7V4a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v3"/>',
  unlink: '<path d="M10 14a3.5 3.5 0 0 0 5 0l4-4a3.5 3.5 0 0 0-5-5l-.5.5"/><path d="M14 10a3.5 3.5 0 0 0-5 0l-4 4a3.5 3.5 0 0 0 5 5l.5-.5"/><path d="M16 21v-2m3-2h2M3 5h2m2-2v2"/>',
  upload: '<path d="M4 17v2a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-2"/><path d="m7 9 5-5 5 5M12 4v12"/>',
  wifi: '<path d="M12 18h.01"/><path d="M9.2 15.2a4 4 0 0 1 5.6 0"/><path d="M6.3 12.3a8 8 0 0 1 11.4 0"/><path d="M3.5 9.5a12 12 0 0 1 17 0"/>',
};

export type IconName = keyof typeof ICONS | string;

const escapeAttribute = (value: unknown) =>
  String(value).replace(/[&<>'"]/g, (char) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", "'": "&#39;", '"': "&quot;" })[char] as string);

export type IconOptions = { size?: number; strokeWidth?: number; label?: string; className?: string };

export function hasIcon(name: string): boolean {
  return name in ICONS;
}

export function uiIcon(name: string, { size = 20, strokeWidth = 1.8, label = "", className = "" }: IconOptions = {}): string {
  const aria = label ? `aria-label="${escapeAttribute(label)}" role="img"` : 'aria-hidden="true"';
  return `<svg width="${escapeAttribute(size)}" height="${escapeAttribute(size)}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="${escapeAttribute(strokeWidth)}" stroke-linecap="round" stroke-linejoin="round" class="${escapeAttribute(className)}" ${aria}>${ICONS[name] || ""}</svg>`;
}
export default uiIcon;
