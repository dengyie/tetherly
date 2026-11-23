// Tauri shell transport (ADR-139 §5.2): requests go through
// invoke("api", …) — the Rust side attaches the bearer token, which never
// enters the WebView's JS context — and events arrive on the ONE SSE
// connection the Rust side owns, forwarded as `tetherly://event` /
// `tetherly://link` emits. Uses window.__TAURI__ (withGlobalTauri), so no
// npm dependency is needed.

import { ApiError, type LinkState, type Method, type ServerEvent, type Transport, unwrapEventData } from "./transport";

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
        // Rust forwards the WireEvent `{seq, type, data}`; the SPA's
        // ServerEvent names the cursor `id` (what `lastSeq` is fed from).
        const ev = e.payload as { seq?: unknown; type?: unknown; data?: unknown } | null;
        if (!ev || typeof ev !== "object" || typeof ev.seq !== "number" || typeof ev.type !== "string") return;
        const sev: ServerEvent = { id: ev.seq, type: ev.type, data: unwrapEventData(ev.type, ev.data) };
        on(sev);
      }),
      tauri().event.listen("tetherly://link", (e) => {
        const s = e.payload as LinkState;
        if (s === "connecting" || s === "live" || s === "resyncing" || s === "offline") onState(s);
      }),
    ];
    // Tauri emits are broadcast-and-forget and the Rust SSE task handshakes
    // in milliseconds — the opening Connecting→Live pair lands BEFORE this
    // WebView's listeners exist, and the SPA would sit on "连接中" forever
    // (production 2026-11-23). Pull the current state once; the push
    // listeners above own every later transition.
    void tauri()
      .core.invoke<string>("link_state")
      .then((s) => {
        if (s === "connecting" || s === "live" || s === "resyncing" || s === "offline") onState(s);
      })
      .catch(() => {});
    return () => {
      for (const p of unsubs) p.then((u) => u()).catch(() => {});
    };
  }
}
