import { afterEach, describe, expect, it, vi } from "vitest";
import { isShelfInvalidation, listenFileDrops } from "./shelf";

describe("isShelfInvalidation", () => {
  it("fires on the shelf topic and on reconnect snapshots only", () => {
    expect(isShelfInvalidation({ type: "invalidated", data: { topic: "shelf" } })).toBe(true);
    expect(isShelfInvalidation({ type: "snapshot", data: {} })).toBe(true);
    expect(isShelfInvalidation({ type: "invalidated", data: { topic: "layout" } })).toBe(false);
    expect(isShelfInvalidation({ type: "transfers.changed", data: {} })).toBe(false);
    expect(isShelfInvalidation({ type: "invalidated", data: null })).toBe(false);
  });
});

describe("listenFileDrops", () => {
  afterEach(() => {
    delete (globalThis as { __TAURI__?: unknown }).__TAURI__;
  });

  it("is a no-op outside the Tauri shell", () => {
    const stop = listenFileDrops(() => {}, () => {});
    expect(() => stop()).not.toThrow();
  });

  it("forwards dropped paths and hover state, and unlistens", async () => {
    let handler: ((e: { payload: { type: string; paths?: string[] } }) => void) | null = null;
    const unlisten = vi.fn();
    (globalThis as { __TAURI__?: unknown }).__TAURI__ = {
      webview: {
        getCurrentWebview: () => ({
          onDragDropEvent: async (h: typeof handler) => {
            handler = h;
            return unlisten;
          },
        }),
      },
    };
    const paths: string[][] = [];
    const hover: boolean[] = [];
    const stop = listenFileDrops((p) => paths.push(p), (o) => hover.push(o));
    await Promise.resolve();
    await Promise.resolve();
    handler!({ payload: { type: "enter", paths: ["/a"] } });
    handler!({ payload: { type: "drop", paths: ["/a", "", "/b"] } });
    handler!({ payload: { type: "drop", paths: [] } });
    expect(paths).toEqual([["/a", "/b"]]);
    expect(hover).toEqual([true, false, false]);
    stop();
    expect(unlisten).toHaveBeenCalledOnce();
  });
});
