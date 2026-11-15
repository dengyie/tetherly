// Pairing store: whether an accept window is open and how long it lives.
// The PIN itself never travels the event stream (contract §4.2); it is
// fetched on demand by the page.
import type { Pairing, Snapshot } from "../api/gen/api";
import type { ServerEvent } from "../transport/transport";

export type PairingState = {
  pairing: Pairing | null;
  loaded: boolean;
};

export function initialPairing(): PairingState {
  return { pairing: null, loaded: false };
}

export function applySnapshot(state: PairingState, snap: Snapshot): PairingState {
  return { pairing: snap.pairing, loaded: true };
}

export function applyEvent(state: PairingState, ev: ServerEvent): PairingState {
  switch (ev.type) {
    case "snapshot":
      return applySnapshot(state, ev.data as Snapshot);
    case "pairing.changed":
      return { pairing: ev.data as Pairing, loaded: true };
    default:
      return state;
  }
}
