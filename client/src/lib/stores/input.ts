// Input store: who holds the cursor (seat + optional peer).
import type { Input, Snapshot } from "../api/gen/api";
import type { ServerEvent } from "../transport/transport";

export type InputState = {
  input: Input | null;
  loaded: boolean;
};

export function initialInput(): InputState {
  return { input: null, loaded: false };
}

export function applySnapshot(state: InputState, snap: Snapshot): InputState {
  return { input: snap.input, loaded: true };
}

// §229 role-aware readers. `seat` is node-centric (controller: remote = my
// cursor left for the peer; target: remote = the controller's cursor entered
// this screen), so views never label by `seat` — they read the daemon-folded
// `cursor_here` plus `role`.
export function cursorAway(i: Input | null): boolean {
  return !!i && !i.cursor_here;
}
/** This node drives the peer (roaming or remote-session controller). */
export function isDriving(i: Input | null): boolean {
  return i?.role === "controller" || i?.role === "session_controller";
}
/** A peer is driving this desktop right now. */
export function isDriven(i: Input | null): boolean {
  return !!i?.cursor_here && i.seat === "remote" && (i.role === "target" || i.role === "session_target");
}
/** Display name for the active peer: live device name, else the raw id. */
export function peerName(i: Input | null, byId: Map<string, { name: string }>): string | null {
  const id = i?.peer;
  if (!id) return null;
  return byId.get(id)?.name ?? id;
}

export function applyEvent(state: InputState, ev: ServerEvent): InputState {
  switch (ev.type) {
    case "snapshot":
      return applySnapshot(state, ev.data as Snapshot);
    case "input.changed":
      return { input: ev.data as Input, loaded: true };
    default:
      return state;
  }
}
