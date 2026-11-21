// Section hydration (§220). The daemon's subscribe burst is the Snapshot
// alone (`uihttp/v1.rs` → `Event::Snapshot`); layout / settings / transfers /
// inbox / activity are pushed as `*.changed` only when they change (hub.rs
// `Key::Invalidate`). A fresh session would therefore sit on "等待…"
// placeholders until something happened. Pull those sections over REST once
// per (re)connection and feed them to the same reducers as synthetic events.
//
// Ordering: a real `*.changed` that lands while a fetch is in flight is newer
// than the fetch result, so the caller reports whether the section moved
// (`isStale`) and a stale result is dropped. Responses are shape-checked so a
// stub/misbehaving backend can never put a half-object into a store (pages
// dereference `data.general.name` etc.).
import type { Api } from "../api/client";
import type { ServerEvent } from "../transport/transport";

export const HYDRATED_SECTIONS = ["layout", "settings", "transfers", "inbox", "activity"] as const;
export type HydratedSection = (typeof HYDRATED_SECTIONS)[number];

const isObj = (v: unknown): v is Record<string, unknown> => v !== null && typeof v === "object";

/** Minimal shape guards: the fields each page dereferences unconditionally. */
export const SECTION_SHAPE: Record<HydratedSection, (v: unknown) => boolean> = {
  layout: (v) => isObj(v) && isObj(v.display) && Array.isArray(v.edges) && isObj(v.config) && isObj(v.policy),
  settings: (v) => isObj(v) && isObj(v.general) && isObj(v.transfer),
  transfers: (v) => isObj(v) && Array.isArray(v.transfers),
  inbox: (v) => isObj(v) && Array.isArray(v.files),
  activity: (v) =>
    isObj(v) && Array.isArray(v.candidates) && Array.isArray(v.replies) && Array.isArray(v.messages) && Array.isArray(v.handoffs),
};

export type SectionApi = Pick<Api, HydratedSection>;

/**
 * Fetch every hydrated section in parallel and hand each valid, still-fresh
 * result to `apply` as a `<section>.changed` event. Never throws: a failed
 * or malformed section is logged and skipped (the page keeps its
 * placeholder; the next real event or hydration fills it).
 */
export async function hydrateSections(
  api: SectionApi,
  isStale: (section: HydratedSection) => boolean,
  apply: (ev: ServerEvent) => void,
  seq: number,
): Promise<HydratedSection[]> {
  const applied: HydratedSection[] = [];
  await Promise.all(
    HYDRATED_SECTIONS.map(async (section) => {
      let data: unknown;
      try {
        data = await api[section]();
      } catch (err) {
        console.warn(`[hydrate] ${section}:`, err);
        return;
      }
      if (isStale(section)) return;
      if (!SECTION_SHAPE[section](data)) {
        console.warn(`[hydrate] ${section}: unexpected shape, skipped`);
        return;
      }
      apply({ id: seq, type: `${section}.changed`, data });
      applied.push(section);
    }),
  );
  return applied;
}

/** `layout.changed` → `layout`; anything else → null. */
export function sectionOf(type: string): HydratedSection | null {
  const i = type.indexOf(".changed");
  if (i < 0) return null;
  const s = type.slice(0, i);
  return (HYDRATED_SECTIONS as readonly string[]).includes(s) ? (s as HydratedSection) : null;
}
