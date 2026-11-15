// Activity store: the four pending/recent feeds (notification candidates,
// quick replies, direct messages, resource handoffs). Written only through
// applyEvent (pure reducers); `activity.changed` replaces the section.
import type { Activity } from "../api/gen/api";
import type { ServerEvent } from "../transport/transport";

export type ActivityState = {
  activity: Activity | null;
  loaded: boolean;
};

export function initialActivity(): ActivityState {
  return { activity: null, loaded: false };
}

export function applyEvent(state: ActivityState, ev: ServerEvent): ActivityState {
  if (ev.type === "activity.changed") {
    return { activity: ev.data as Activity, loaded: true };
  }
  return state;
}
