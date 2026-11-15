// Devices store: state lives ONLY here and changes ONLY through
// applySnapshot / applyEvent (pure reducers, unit-tested). Pages never
// poll; the event stream is the single writer (ADR-139 §5.3).
import type { Device, Snapshot } from "../api/gen/api";
import type { ServerEvent } from "../transport/transport";

export type DevicesState = {
  byId: Map<string, Device>;
  loaded: boolean;
};

export function initialDevices(): DevicesState {
  return { byId: new Map(), loaded: false };
}

export function applySnapshot(state: DevicesState, snap: Snapshot): DevicesState {
  const byId = new Map<string, Device>();
  for (const d of snap.devices) byId.set(d.id, d);
  return { byId, loaded: true };
}

export function applyEvent(state: DevicesState, ev: ServerEvent): DevicesState {
  switch (ev.type) {
    case "snapshot":
      return applySnapshot(state, ev.data as Snapshot);
    case "device.updated": {
      const d = ev.data as Device;
      const byId = new Map(state.byId);
      byId.set(d.id, d);
      return { ...state, byId };
    }
    case "device.removed": {
      const id = (ev.data as { id: string }).id;
      const byId = new Map(state.byId);
      byId.delete(id);
      return { ...state, byId };
    }
    default:
      return state;
  }
}
