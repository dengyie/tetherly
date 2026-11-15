// Transfers store: active + terminal file transfers. Written only through
// applyEvent (pure reducers); `transfers.changed` replaces the whole section.
import type { Transfers } from "../api/gen/api";
import type { ServerEvent } from "../transport/transport";

export type TransfersState = {
  transfers: Transfers | null;
  loaded: boolean;
};

export function initialTransfers(): TransfersState {
  return { transfers: null, loaded: false };
}

export function applyEvent(state: TransfersState, ev: ServerEvent): TransfersState {
  if (ev.type === "transfers.changed") {
    return { transfers: ev.data as Transfers, loaded: true };
  }
  return state;
}
