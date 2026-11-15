import { writable } from "svelte/store";
import type { LinkState } from "../transport/transport";

// The one store that is allowed to be driven by transport callbacks.
export const linkState = writable<LinkState>("connecting");
export const lastSeq = writable<number | null>(null);
