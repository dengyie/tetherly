// ADR-139 §5.2: the one seam between the SPA and its two runtimes.
// HttpTransport serves the daemon-hosted web build (same-origin fetch +
// hand-rolled SSE with auth headers); TauriTransport (M3) will proxy the
// same calls through invoke(). Business code only ever sees this interface.

export type Method = "GET" | "POST" | "PUT" | "PATCH" | "DELETE";

export type ServerEvent = {
  id: number; // seq
  type: string; // event name, e.g. "device.updated" | "snapshot"
  data: unknown;
};

export type LinkState = "connecting" | "live" | "resyncing" | "offline";

// The daemon serializes `Event` with `#[serde(tag = "type")]` and named
// fields, so the envelope's `data` is `{snapshot: {…}}`, `{device: {…}}`,
// `{layout: {…}}` … — one wrapper key per state-bearing event. Stores speak
// in bare payloads (`Snapshot`, `Device`, `Layout` …), so both transports
// unwrap that key here. `device.removed` (`{id}`), `invalidated` (`{topic}`)
// and `diagnostic` (`{kind, payload}`, flattened) are handed through as-is.
const WRAPPED_EVENT_KEY: Readonly<Record<string, string>> = {
  snapshot: "snapshot",
  "device.updated": "device",
  "pairing.changed": "pairing",
  "input.changed": "input",
  "system.changed": "system",
  "transfers.changed": "transfers",
  "inbox.changed": "inbox",
  "layout.changed": "layout",
  "settings.changed": "settings",
  "activity.changed": "activity",
};

export function unwrapEventData(type: string, data: unknown): unknown {
  const key = WRAPPED_EVENT_KEY[type];
  if (key === undefined || data === null || typeof data !== "object") return data;
  const obj = data as Record<string, unknown>;
  // Legacy/mock streams may already carry the bare payload; only unwrap when
  // the wrapper key is the sole field, so a bare `Snapshot` (which has no
  // `snapshot` key) passes through unchanged.
  if (!(key in obj)) return data;
  const keys = Object.keys(obj);
  if (keys.length !== 1) return data;
  return obj[key];
}

export interface Transport {
  request<T>(method: Method, path: string, body?: unknown): Promise<T>;
  subscribe(
    lastSeq: number | null,
    on: (e: ServerEvent) => void,
    onState: (s: LinkState) => void,
  ): () => void;
}

export class ApiError extends Error {
  readonly code: string;
  readonly status: number;
  constructor(status: number, code: string, message: string) {
    super(message);
    this.status = status;
    this.code = code;
  }
}
