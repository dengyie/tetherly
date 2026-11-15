// Inbox store: files waiting in the daemon's incoming inbox. Written only
// through applyEvent (pure reducers); `inbox.changed` replaces the section.
import type { Inbox } from "../api/gen/api";
import type { ServerEvent } from "../transport/transport";

export type InboxState = {
  inbox: Inbox | null;
  loaded: boolean;
};

export function initialInbox(): InboxState {
  return { inbox: null, loaded: false };
}

export function applyEvent(state: InboxState, ev: ServerEvent): InboxState {
  if (ev.type === "inbox.changed") {
    return { inbox: ev.data as Inbox, loaded: true };
  }
  return state;
}
