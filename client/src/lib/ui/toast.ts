// Toast store (ported behaviour of awesome-ui/vanilla/ToastStack.js): push
// returns an id, toasts auto-dismiss after `duration` ms (0 = sticky).
import { writable } from "svelte/store";

export type ToastType = "success" | "error" | "warning" | "info";
export type Toast = { id: number; message: string; type: ToastType };

export const toasts = writable<Toast[]>([]);
let seq = 0;
const timers = new Map<number, ReturnType<typeof setTimeout>>();

export function pushToast(message: string, type: ToastType = "info", duration = 4000): number {
  const id = ++seq;
  toasts.update((list) => [...list, { id, message, type }]);
  if (duration > 0) timers.set(id, setTimeout(() => dismissToast(id), duration));
  return id;
}

export function dismissToast(id: number): void {
  const t = timers.get(id);
  if (t) {
    clearTimeout(t);
    timers.delete(id);
  }
  toasts.update((list) => list.filter((x) => x.id !== id));
}

export function clearToasts(): void {
  for (const t of timers.values()) clearTimeout(t);
  timers.clear();
  toasts.set([]);
}

/** Run an async op and report its outcome as a toast; returns success. */
export async function withToast(op: () => Promise<unknown>, ok?: string): Promise<boolean> {
  try {
    await op();
    if (ok) pushToast(ok, "success");
    return true;
  } catch (e) {
    pushToast(e instanceof Error ? e.message : String(e), "error", 6000);
    return false;
  }
}
