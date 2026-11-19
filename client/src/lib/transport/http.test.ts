import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  HttpTransport,
  parseFrame,
  setApiToken,
  SSE_IDLE_TIMEOUT_MS,
} from "./http";

function response(status: number, contentType: string | null, body: unknown = null): Response {
  return {
    ok: status >= 200 && status < 300,
    status,
    statusText: status === 200 ? "OK" : status === 401 ? "Unauthorized" : "Error",
    headers: new Headers(contentType ? { "content-type": contentType } : undefined),
    body,
  } as unknown as Response;
}

function streamResponse(reader: { read: () => Promise<unknown>; cancel: () => Promise<void> }) {
  return response(200, "text/event-stream; charset=utf-8", {
    getReader: () => reader,
  });
}

function frame(seq: number, type: string, data: unknown) {
  return `id: ${seq}\nevent: ${type}\ndata: ${JSON.stringify({ seq, type, data })}\n\n`;
}

async function flush() {
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
}

describe("parseFrame", () => {
  it("unwraps v1 envelopes and keeps legacy bare JSON payloads", () => {
    expect(parseFrame(frame(4, "snapshot", { gen: 1 }))).toEqual({
      id: 4,
      type: "snapshot",
      data: { gen: 1 },
    });
    expect(parseFrame("id: 4\nevent: legacy\ndata: {\"ok\":true}\n\n")).toEqual({
      id: 4,
      type: "legacy",
      data: { ok: true },
    });
    expect(parseFrame("id: 4\nevent: legacy\ndata: {\"type\":\"legacy\",\"ok\":true}\n\n")).toEqual({
      id: 4,
      type: "legacy",
      data: { type: "legacy", ok: true },
    });
    expect(parseFrame("id: 4\r\nevent: legacy\r\ndata: {\"type\":\"legacy\",\"ok\":true}\r\n\r\n")).toEqual({
      id: 4,
      type: "legacy",
      data: { type: "legacy", ok: true },
    });
  });

  it("rejects malformed or mismatched v1 sequence/type envelopes", () => {
    expect(parseFrame("id: 4\nevent: x\ndata: {not-json}\n\n")).toBeNull();
    expect(
      parseFrame(
        "id: 4\nevent: x\ndata: {\"seq\":5,\"type\":\"x\",\"data\":null}\n\n",
      ),
    ).toBeNull();
  });
});

describe("HttpTransport SSE", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    setApiToken(null);
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  it.each([
    [401, "text/event-stream"],
    [500, "text/event-stream"],
    [200, "application/json"],
  ])("rejects %s or non-SSE handshakes before live", async (status, contentType) => {
    const fetchMock = vi
      .spyOn(globalThis, "fetch")
      .mockResolvedValue(response(status, contentType));
    const states: string[] = [];
    const stop = new HttpTransport().subscribe(null, () => {}, (state) => states.push(state));

    await flush();
    expect(states).toEqual(["connecting", "offline"]);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    stop();
  });

  it("cancels a silent established stream and schedules reconnect", async () => {
    let resolveRead: ((value: { done: boolean; value?: unknown }) => void) | undefined;
    const reader = {
      read: vi.fn(
        () =>
          new Promise<{ done: boolean; value?: unknown }>((resolve) => {
            resolveRead = resolve;
          }),
      ),
      cancel: vi.fn(async () => {
        resolveRead?.({ done: true });
      }),
    };
    const fetchMock = vi.spyOn(globalThis, "fetch").mockResolvedValue(streamResponse(reader));
    const states: string[] = [];
    const stop = new HttpTransport().subscribe(null, () => {}, (state) => states.push(state));

    await flush();
    expect(states).toEqual(["connecting", "live"]);
    await vi.advanceTimersByTimeAsync(SSE_IDLE_TIMEOUT_MS);
    await flush();

    expect(reader.cancel).toHaveBeenCalledTimes(1);
    expect(states).toEqual(["connecting", "live", "offline"]);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    stop();
  });

  it("filters duplicate and rewound sequences before delivery", async () => {
    const reader = {
      read: vi
        .fn()
        .mockResolvedValueOnce({
          done: false,
          value: new TextEncoder().encode(
            frame(7, "snapshot", { n: 7 }) +
              frame(7, "snapshot", { n: 7 }) +
              frame(6, "snapshot", { n: 6 }) +
              frame(8, "snapshot", { n: 8 }),
          ),
        })
        .mockResolvedValueOnce({ done: true, value: undefined }),
      cancel: vi.fn(async () => {}),
    };
    vi.spyOn(globalThis, "fetch").mockResolvedValue(streamResponse(reader));
    const events: unknown[] = [];
    const stop = new HttpTransport().subscribe(null, (event) => events.push(event), () => {});

    await flush();
    expect(events).toEqual([
      { id: 7, type: "snapshot", data: { n: 7 } },
      { id: 8, type: "snapshot", data: { n: 8 } },
    ]);
    stop();
  });

  it("resumes from the latest sequence after a stream closes", async () => {
    const firstReader = {
      read: vi
        .fn()
        .mockResolvedValueOnce({ done: false, value: new TextEncoder().encode(frame(7, "snapshot", { gen: 1 })) })
        .mockResolvedValueOnce({ done: true, value: undefined }),
      cancel: vi.fn(async () => {}),
    };
    const secondReader = {
      read: vi
        .fn()
        .mockResolvedValueOnce({ done: false, value: new TextEncoder().encode(frame(8, "snapshot", { gen: 2 })) })
        .mockResolvedValueOnce({ done: true, value: undefined }),
      cancel: vi.fn(async () => {}),
    };
    const fetchMock = vi
      .spyOn(globalThis, "fetch")
      .mockResolvedValueOnce(streamResponse(firstReader))
      .mockResolvedValueOnce(streamResponse(secondReader));
    const events: unknown[] = [];
    const stop = new HttpTransport().subscribe(null, (event) => events.push(event), () => {});

    await flush();
    expect(events).toEqual([{ id: 7, type: "snapshot", data: { gen: 1 } }]);
    await vi.advanceTimersByTimeAsync(1000);
    await flush();

    expect(events).toEqual([
      { id: 7, type: "snapshot", data: { gen: 1 } },
      { id: 8, type: "snapshot", data: { gen: 2 } },
    ]);
    expect(String(fetchMock.mock.calls[1]?.[0])).toContain("last_event_id=7");
    stop();
  });
});
