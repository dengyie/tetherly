// Tauri shell transport (ADR-139 §5.2): requests go through
// invoke("api", …) — the Rust side attaches the bearer token, which never
// enters the WebView's JS context — and events arrive on the ONE SSE
// connection the Rust side owns, forwarded as `tetherly://event` /
// `tetherly://link` emits. Uses window.__TAURI__ (withGlobalTauri), so no
// npm dependency is needed.

import { ApiError, type LinkState, type Method, type ServerEvent, type Transport } from "./transport";

type TauriApi = {
  core: {
    invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T>;
  };
  event: {
    listen(event: string, handler: (e: { payload: unknown }) => void): Promise<() => void>;
  };
};

// Exported for tests, which stub this shape onto globalThis.__TAURI__.
export type { TauriApi as TauriBridge };

function tauri(): TauriApi {
  const t = (globalThis as { __TAURI__?: TauriApi }).__TAURI__;
  if (!t) throw new Error("TauriTransport used outside the Tauri shell");
  return t;
}

export function isTauri(): boolean {
  return (globalThis as { __TAURI__?: unknown }).__TAURI__ !== undefined;
}

type ProxyReply = { status: number; body: unknown };

export class TauriTransport implements Transport {
  async request<T>(method: Method, path: string, body?: unknown): Promise<T> {
    const reply = await tauri().core.invoke<ProxyReply>("api", {
      method,
      path,
      // invoke serializes undefined poorly; send null and let Rust unwrap.
      body: body === undefined ? null : body,
    });
    if (reply.status >= 300 || reply.status === 204) {
      if (reply.status === 204) return undefined as T;
      const err = (reply.body as { error?: { code: string; message: string } })?.error;
      throw new ApiError(reply.status, err?.code ?? "internal", err?.message ?? `status ${reply.status}`);
    }
    return reply.body as T;
  }

  subscribe(
    _lastSeq: number | null,
    on: (e: ServerEvent) => void,
    onState: (s: LinkState) => void,
  ): () => void {
    // The Rust side keeps lastSeq; the JS value is ignored (§5.2).
    const unsubs: Promise<() => void>[] = [
      tauri().event.listen("tetherly://event", (e) => {
        const ev = e.payload as ServerEvent;
        if (ev && typeof ev === "object" && "seq" in ev && "type" in ev) on(ev);
      }),
      tauri().event.listen("tetherly://link", (e) => {
        const s = e.payload as LinkState;
        if (s === "connecting" || s === "live" || s === "resyncing" || s === "offline") onState(s);
      }),
    ];
    return () => {
      for (const p of unsubs) p.then((u) => u()).catch(() => {});
    };
  }
}
