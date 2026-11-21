// TauriTransport unit tests: the window.__TAURI__ bridge is stubbed, so the
// same ServerEvent/LinkState wiring the shell emits can be driven from tests
// without a real WebView.
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { ApiError } from "./transport";
import { TauriTransport, isTauri, type TauriBridge } from "./tauri";

type Emit = (channel: string, payload: unknown) => void;

// Install a fake bridge; returns an emit() that replays Rust-side app.emit.
function installBridge(invokeImpl: (cmd: string, args: Record<string, unknown>) => unknown): Emit {
  const listeners = new Map<string, Set<(e: { payload: unknown }) => void>>();
  const bridge: TauriBridge = {
    core: {
      invoke: async <T,>(cmd: string, args?: Record<string, unknown>) =>
        invokeImpl(cmd, args ?? {}) as T,
    },
    event: {
      listen(event, handler) {
        let set = listeners.get(event);
        if (!set) listeners.set(event, (set = new Set()));
        set.add(handler);
        return Promise.resolve(() => set!.delete(handler));
      },
    },
  };
  (globalThis as { __TAURI__?: TauriBridge }).__TAURI__ = bridge;
  return (channel, payload) => {
    for (const h of listeners.get(channel) ?? []) h({ payload });
  };
}

beforeEach(() => {
  (globalThis as { __TAURI__?: unknown }).__TAURI__ = undefined;
});
afterEach(() => {
  (globalThis as { __TAURI__?: unknown }).__TAURI__ = undefined;
});

describe("TauriTransport", () => {
  it("requests proxy through invoke and unwraps the reply", async () => {
    const calls: unknown[] = [];
    installBridge((cmd, args) => {
      calls.push([cmd, args]);
      return { status: 200, body: { ok: true } };
    });
    const t = new TauriTransport();
    const out = await t.request<{ ok: boolean }>("PATCH", "/api/v1/settings", { name: "Desk" });
    expect(out.ok).toBe(true);
    expect(calls[0]).toEqual(["api", { method: "PATCH", path: "/api/v1/settings", body: { name: "Desk" } }]);
  });

  it("turns non-2xx replies into ApiError with the daemon's code", async () => {
    installBridge(() => ({
      status: 404,
      body: { error: { code: "not_found", message: "no such device" } },
    }));
    const t = new TauriTransport();
    const err = await t.request("GET", "/api/v1/devices/x").catch((e) => e);
    expect(err).toBeInstanceOf(ApiError);
    expect((err as ApiError).status).toBe(404);
    expect((err as ApiError).code).toBe("not_found");
  });

  it("204 resolves to undefined without an error", async () => {
    installBridge(() => ({ status: 204, body: null }));
    const t = new TauriTransport();
    await expect(t.request("DELETE", "/api/v1/inbox")).resolves.toBeUndefined();
  });

  it("forwards tetherly://event payloads and link states", async () => {
    const emit = installBridge(() => ({}));
    const t = new TauriTransport();
    const events: unknown[] = [];
    const states: string[] = [];
    t.subscribe(null, (e) => events.push(e), (s) => states.push(s));
    // Simulate the Rust emits; a payload without seq/type is ignored.
    // Rust forwards the envelope's `data` untouched → `{activity: {…}}`.
    emit("tetherly://event", { seq: 3, type: "activity.changed", data: { activity: { k: 1 } } });
    emit("tetherly://link", "offline");
    emit("tetherly://event", { bogus: true });
    emit("tetherly://link", "sideways");
    // …and the SPA sees a ServerEvent: `id` (fed to lastSeq) + bare payload.
    expect(events).toEqual([{ id: 3, type: "activity.changed", data: { k: 1 } }]);
    expect(states).toEqual(["offline"]);
  });

  it("unsubscribe stops the flow", async () => {
    const emit = installBridge(() => ({}));
    const t = new TauriTransport();
    const events: unknown[] = [];
    const stop = t.subscribe(null, (e) => events.push(e), () => {});
    await stop();
    emit("tetherly://event", { seq: 1, type: "x", data: null });
    expect(events).toEqual([]);
  });

  it("isTauri tracks the bridge presence", () => {
    expect(isTauri()).toBe(false);
    installBridge(() => ({}));
    expect(isTauri()).toBe(true);
  });
});
