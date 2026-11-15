// MockTransport (ADR-139 §5.2): replays a scripted stream through the same
// Transport seam the app consumes. Tests feed snapshot + events, then assert
// on what the real reducers + pages rendered. Requests are recorded and
// answered by a per-test responder.
import type { LinkState, Method, ServerEvent, Transport } from "./transport";

export class MockTransport implements Transport {
  readonly calls: { method: Method; path: string; body?: unknown }[] = [];
  private respond: (method: Method, path: string, body?: unknown) => unknown;

  /** Installed by subscribe; tests push events through emit(). */
  emit: (e: ServerEvent) => void = () => {};
  setLinkState: (s: LinkState) => void = () => {};

  constructor(responder: (method: Method, path: string, body?: unknown) => unknown = () => ({})) {
    this.respond = responder;
  }

  async request<T>(method: Method, path: string, body?: unknown): Promise<T> {
    this.calls.push({ method, path, body });
    return this.respond(method, path, body) as T;
  }

  subscribe(
    _lastSeq: number | null,
    on: (e: ServerEvent) => void,
    onState: (s: LinkState) => void,
  ): () => void {
    this.emit = on;
    this.setLinkState = onState;
    return () => {
      this.emit = () => {};
      this.setLinkState = () => {};
    };
  }
}
