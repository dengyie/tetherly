// Browser transport: same-origin fetch with the bearer token, and a
// hand-rolled SSE reader (EventSource cannot send Authorization headers).
// Reconnect policy per ADR-139 §5.2: 1s -> 2s -> 5s -> 10s, capped.

import { API_PREFIX } from "../api/gen/api";
import { ApiError, type LinkState, type Method, type ServerEvent, type Transport } from "./transport";

const BACKOFF_MS = [1000, 2000, 5000, 10000];
const SSE_IDLE_TIMEOUT_MS = 45_000; // §4.3 rule 5: 15s ping, 45s = dead

let token: string | null = null;
export function setApiToken(t: string | null) {
  token = t;
}

export class HttpTransport implements Transport {
  async request<T>(method: Method, path: string, body?: unknown): Promise<T> {
    const headers: Record<string, string> = { "Tetherly-Client": "web/1.0.0" };
    if (token) headers.Authorization = `Bearer ${token}`;
    if (body !== undefined) headers["Content-Type"] = "application/json";
    const resp = await fetch(API_PREFIX + path, {
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
    const controller = new AbortController();
    let closed = false;
    let attempt = 0;

    const connect = () => {
      if (closed) return;
      onState(attempt === 0 && lastSeq === null ? "connecting" : "resyncing");
      const url = new URL(API_PREFIX + "/events", location.origin);
      if (lastSeq !== null) url.searchParams.set("last_event_id", String(lastSeq));
      const headers: Record<string, string> = { "Tetherly-Client": "web/1.0.0" };
      if (token) headers.Authorization = `Bearer ${token}`;

      fetch(url, { signal: controller.signal, headers })
        .then(async (resp) => {
          if (!resp.ok || !resp.body) throw new Error(`events ${resp.status}`);
          onState("live");
          attempt = 0;
          const reader = resp.body.getReader();
          const decoder = new TextDecoder();
          let buf = "";
          // SSE frames are id/event/data lines; parse incrementally.
          for (;;) {
            const { done, value } = await reader.read();
            if (done) throw new Error("stream closed");
            buf += decoder.decode(value, { stream: true });
            let idx: number;
            while ((idx = buf.indexOf("\n\n")) >= 0) {
              const frame = buf.slice(0, idx);
              buf = buf.slice(idx + 2);
              const ev = parseFrame(frame);
              if (ev) {
                lastSeq = ev.id;
                on(ev);
              }
            }
          }
        })
        .catch((err) => {
          if (closed || controller.signal.aborted) return;
          onState("offline");
          const delay = BACKOFF_MS[Math.min(attempt, BACKOFF_MS.length - 1)];
          attempt += 1;
          // lastSeq survives the reconnect: the server replays or re-snapshots.
          setTimeout(connect, delay);
          console.error("[events]", err);
        });
    };
    connect();
    return () => {
      closed = true;
      controller.abort();
    };
  }
}

function parseFrame(frame: string): ServerEvent | null {
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
    if (parsed && typeof parsed === "object" && "seq" in parsed && "type" in parsed) {
      return { id, type, data: parsed.data };
    }
    return { id, type, data: parsed };
  } catch (err) {
    console.error("[events]", err);
    return null;
  }
}
