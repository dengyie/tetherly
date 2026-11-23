// v0.2.89 共享架 (shelf): registering a file only records its path/size/mtime;
// bytes move when another device fetches it. Two small seams live here so
// the Transfers page stays declarative and both are unit-testable.

type DragDropPayload = { type: string; paths?: string[] };
type TauriWebviewApi = {
  webview?: {
    getCurrentWebview(): {
      onDragDropEvent(handler: (e: { payload: DragDropPayload }) => void): Promise<() => void>;
    };
  };
};

/** The shelf view must be refetched: the daemon invalidated it, or the
 * stream (re)connected (a snapshot), after which nothing was missed. */
export function isShelfInvalidation(ev: { type: string; data: unknown }): boolean {
  if (ev.type === "snapshot") return true;
  if (ev.type !== "invalidated") return false;
  const topic = (ev.data as { topic?: unknown } | null)?.topic;
  return topic === "shelf";
}

/** Native file drops onto the shell window (Tauri delivers real paths; a
 * browser build has none, so this is a no-op there). `onHover` tracks the
 * drag-over highlight. Returns an unlisten function. */
export function listenFileDrops(
  onPaths: (paths: string[]) => void,
  onHover: (over: boolean) => void,
): () => void {
  const wv = (globalThis as { __TAURI__?: TauriWebviewApi }).__TAURI__?.webview;
  if (!wv) return () => {};
  let unlisten: (() => void) | null = null;
  let stopped = false;
  void wv
    .getCurrentWebview()
    .onDragDropEvent(({ payload }) => {
      if (payload.type === "enter" || payload.type === "over") onHover(true);
      else if (payload.type === "leave") onHover(false);
      else if (payload.type === "drop") {
        onHover(false);
        const paths = (payload.paths ?? []).filter((p) => typeof p === "string" && p.length > 0);
        if (paths.length > 0) onPaths(paths);
      }
    })
    .then((u) => {
      if (stopped) u();
      else unlisten = u;
    })
    .catch(() => {});
  return () => {
    stopped = true;
    unlisten?.();
  };
}
