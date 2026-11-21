// Browser transport: same-origin fetch with the bearer token, and a
// hand-rolled SSE reader (EventSource cannot send Authorization headers).
// Reconnect policy per ADR-139 §5.2: 1s -> 2s -> 5s -> 10s, capped.

import { API_PREFIX } from "../api/gen/api";
import { ApiError, type LinkState, type Method, type ServerEvent, type Transport, unwrapEventData } from "./transport";

const BACKOFF_MS = [1000, 2000, 5000, 10000];
export const SSE_HANDSHAKE_TIMEOUT_MS = 10_000;
export const SSE_IDLE_TIMEOUT_MS = 45_000; // §4.3 rule 5: 15s ping, 45s = dead

let token: string | null = null;
export function setApiToken(t: string | null) {
  token = t;
}

export class HttpTransport implements Transport {
  async request<T>(method: Method, path: string, body?: unknown): Promise<T> {
    const headers: Record<string, string> = { "Tetherly-Client": "web/1.0.0" };
    if (token) headers.Authorization = `Bearer ${token}`;
    if (body !== undefined) headers["Content-Type"] = "application/json";
    // `path` is already the full `/api/v1/...` route (ApiClient speaks the same
    // paths to the Tauri proxy, which requires that prefix) — do not prepend.
    const resp = await fetch(path, {
      method,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    if (resp.status === 204) return undefined as T;
    const json = await resp.json();
    if (!resp.ok) {
      const err = (json as { error?: { code: string; message: string } }).error;
      throw new ApiError(resp.status, err?.code ?? "internal", err?.message ?? resp.statusText);
    }
    return json as T;
  }

  subscribe(
    lastSeq: number | null,
    on: (e: ServerEvent) => void,
    onState: (s: LinkState) => void,
  ): () => void {
    let closed = false;
    let attempt = 0;
    let retryTimer: ReturnType<typeof setTimeout> | undefined;
    let activeController: AbortController | undefined;

    const connect = () => {
      if (closed) return;
      onState(attempt === 0 && lastSeq === null ? "connecting" : "resyncing");
      const url = new URL(API_PREFIX + "/events", location.origin);
      if (lastSeq !== null) url.searchParams.set("last_event_id", String(lastSeq));
      const headers: Record<string, string> = { "Tetherly-Client": "web/1.0.0" };
      if (token) headers.Authorization = `Bearer ${token}`;
      const controller = new AbortController();
      activeController = controller;
      const handshakeTimer = setTimeout(() => controller.abort(), SSE_HANDSHAKE_TIMEOUT_MS);

      fetch(url, { signal: controller.signal, headers })
        .then(async (resp) => {
          clearTimeout(handshakeTimer);
          const contentType = resp.headers.get("content-type")?.split(";", 1)[0].trim().toLowerCase();
          if (!resp.ok || !resp.body || contentType !== "text/event-stream") {
            throw new Error(`events ${resp.status} (${contentType ?? "missing content-type"})`);
          }
          onState("live");
          attempt = 0;
          const reader = resp.body.getReader();
          const decoder = new TextDecoder();
          let buf = "";
          let idleTimer: ReturnType<typeof setTimeout> | undefined;
          const armIdleWatchdog = () => {
            if (idleTimer !== undefined) clearTimeout(idleTimer);
            idleTimer = setTimeout(() => {
              // Cancelling the reader makes the pending read finish with done;
              // the common stream-closed path then schedules a reconnect.
              void reader.cancel();
            }, SSE_IDLE_TIMEOUT_MS);
          };
          try {
            armIdleWatchdog();
            // SSE frames are id/event/data lines; parse incrementally.
            for (;;) {
              const { done, value } = await reader.read();
              if (done) throw new Error("stream closed");
              // A comment/keepalive is still liveness data, so reset on every
              // received chunk rather than only on parsed application events.
              armIdleWatchdog();
              buf += decoder.decode(value, { stream: true });
              let boundary: { index: number; length: number } | null;
              while ((boundary = findSseBoundary(buf)) !== null) {
                const frame = buf.slice(0, boundary.index);
                buf = buf.slice(boundary.index + boundary.length);
                const ev = parseFrame(frame);
                if (ev) {
                  if (lastSeq !== null && ev.id <= lastSeq) continue;
                  lastSeq = ev.id;
                  on(ev);
                }
              }
            }
          } finally {
            if (idleTimer !== undefined) clearTimeout(idleTimer);
          }
        })
        .catch((err) => {
          clearTimeout(handshakeTimer);
          if (closed) return;
          onState("offline");
          const delay = BACKOFF_MS[Math.min(attempt, BACKOFF_MS.length - 1)];
          attempt += 1;
          // lastSeq survives the reconnect: the server replays or re-snapshots.
          retryTimer = setTimeout(() => {
            retryTimer = undefined;
            connect();
          }, delay);
          console.error("[events]", err);
        });
    };
    connect();
    return () => {
      closed = true;
      if (retryTimer !== undefined) clearTimeout(retryTimer);
      if (activeController !== undefined) activeController.abort();
    };
  }
}

function findSseBoundary(buffer: string): { index: number; length: number } | null {
  const lf = buffer.indexOf("\n\n");
  const crlf = buffer.indexOf("\r\n\r\n");
  if (lf < 0 && crlf < 0) return null;
  if (lf < 0 || (crlf >= 0 && crlf < lf)) return { index: crlf, length: 4 };
  return { index: lf, length: 2 };
}

export function parseFrame(frame: string): ServerEvent | null {
  let id = 0;
  let type = "message";
  let data = "";
  for (const line of frame.split("\n")) {
    if (line.startsWith("id:")) id = Number(line.slice(3).trim());
    else if (line.startsWith("event:")) type = line.slice(6).trim();
    else if (line.startsWith("data:")) data += line.slice(5).trimStart();
    else if (line.startsWith("retry:") || line.startsWith(":")) continue;
  }
  if (!data) return null;
  try {
    // The wire data is the EventEnvelope (`{seq, type, data: …}`, §4.3);
    // stores and MockTransport speak in bare payloads, so unwrap the inner
    // `data` field and hand it over with the envelope's type.
    const parsed = JSON.parse(data);
    if (parsed && typeof parsed === "object" && "seq" in parsed) {
      if (parsed.seq !== id || !("type" in parsed) || !("data" in parsed)) return null;
      return { id, type, data: unwrapEventData(type, parsed.data) };
    }
    return { id, type, data: parsed };
  } catch (err) {
    console.error("[events]", err);
    return null;
  }
}
