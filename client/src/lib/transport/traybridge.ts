// Tray bridge: typed channel between the tray panel window (SPA /tray
// route) and the shell's Rust side. Outbound actions go through the same
// global event system the shell already uses (`tetherly://tray-action`);
// inbound state arrives via `tetherly://event` / `tetherly://link`, which
// TauriTransport already unwraps — the tray window runs the full App so it
// needs no extra wiring for those.
//
// Non-Tauri environments (browser tab, vitest) degrade to a no-op sink so
// the component renders everywhere.

type TrayAction = { action: "open-main" } | { action: "toggled"; value: boolean } | { action: "error"; message: string };

type TauriEventApi = {
  event: {
    emit(event: string, payload: unknown): Promise<void>;
  };
};

function tauriEvent(): TauriEventApi | null {
  const t = (globalThis as { __TAURI__?: TauriEventApi }).__TAURI__;
  return t?.event ? t : null;
}

export const TRAY_ACTION_CHANNEL = "tetherly://tray-action";

export const emitter = {
  send(action: string, extra: Record<string, unknown> = {}): void {
    const t = tauriEvent();
    const payload = { action, ...extra };
    if (t) {
      void t.event.emit(TRAY_ACTION_CHANNEL, payload).catch(() => {});
    } else if (typeof console !== "undefined") {
      // Browser/vitest: keep the path observable without a shell.
      console.debug("[tray-action]", payload);
    }
  },
};

export type { TrayAction };
