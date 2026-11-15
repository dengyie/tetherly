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
