// Settings store: read-only grouped view of the daemon config. Written only
// through applyEvent (pure reducers); `settings.changed` replaces the section.
import type { Settings } from "../api/gen/api";
import type { ServerEvent } from "../transport/transport";

export type SettingsState = {
  settings: Settings | null;
  loaded: boolean;
};

export function initialSettings(): SettingsState {
  return { settings: null, loaded: false };
}

export function applyEvent(state: SettingsState, ev: ServerEvent): SettingsState {
  if (ev.type === "settings.changed") {
    return { settings: ev.data as Settings, loaded: true };
  }
  return state;
}
