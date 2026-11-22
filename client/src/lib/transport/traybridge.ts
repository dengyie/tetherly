// Tray bridge: typed channel between the tray panel window (SPA /tray
// route) and the shell's Rust side. Outbound actions go through the same
// global event system the shell already uses (`tetherly://tray-action`);
// inbound state arrives via `tetherly://event` / `tetherly://link`, which
// TauriTransport already unwraps — the tray window runs the full App so it
// needs no extra wiring for those.
//
// Non-Tauri environments (browser tab, vitest) degrade to a no-op sink so
// the component renders everywhere.

type TrayAction =
  | { action: "open-main" }
  | { action: "close" }
  | { action: "quit" }
  | { action: "resize"; height: number }
  | { action: "toggled"; value: boolean }
  | { action: "error"; message: string };

// Daemon menubar panel (§225): the native macOS panel hosts this route in a
// WKWebView and registers a `tetherlyTray` script-message handler. Tauri's
// own WebKit bridge never exposes that name, so this probe is unambiguous.
type WebKitBridge = { messageHandlers?: { tetherlyTray?: { postMessage(message: string): void } } };

function webkitBridge(): { postMessage(message: string): void } | null {
  return (globalThis as { webkit?: WebKitBridge }).webkit?.messageHandlers?.tetherlyTray ?? null;
}

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
    const wk = webkitBridge();
    const payload = { action, ...extra };
    if (wk) {
      try {
        wk.postMessage(JSON.stringify(payload));
      } catch {
        /* handler torn down mid-flight */
      }
    } else if (t) {
      void t.event.emit(TRAY_ACTION_CHANNEL, payload).catch(() => {});
    } else if (typeof console !== "undefined") {
      // Browser/vitest: keep the path observable without a shell.
      console.debug("[tray-action]", payload);
    }
  },
};

export type { TrayAction };
