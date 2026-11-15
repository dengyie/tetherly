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
