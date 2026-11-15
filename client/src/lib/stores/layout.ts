// Layout store: display rect, configured edges, crossing config and policy.
// Written only through applyEvent (pure reducers); `layout.changed` replaces
// the whole section.
import type { Layout } from "../api/gen/api";
import type { ServerEvent } from "../transport/transport";

export type LayoutState = {
  layout: Layout | null;
  loaded: boolean;
};

export function initialLayout(): LayoutState {
  return { layout: null, loaded: false };
}

export function applyEvent(state: LayoutState, ev: ServerEvent): LayoutState {
  if (ev.type === "layout.changed") {
    return { layout: ev.data as Layout, loaded: true };
  }
  return state;
}
