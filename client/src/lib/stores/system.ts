// System store: the node's own identity/version/ports block. Written only
// through applySnapshot / applyEvent (pure reducers); `system.changed`
// replaces the whole section.
import type { Snapshot, System } from "../api/gen/api";
import type { ServerEvent } from "../transport/transport";

export type SystemState = {
  system: System | null;
  loaded: boolean;
};

export function initialSystem(): SystemState {
  return { system: null, loaded: false };
}

export function applySnapshot(state: SystemState, snap: Snapshot): SystemState {
  return { system: snap.system, loaded: true };
}

export function applyEvent(state: SystemState, ev: ServerEvent): SystemState {
  switch (ev.type) {
    case "snapshot":
      return applySnapshot(state, ev.data as Snapshot);
    case "system.changed":
      return { system: ev.data as System, loaded: true };
    default:
      return state;
  }
}
