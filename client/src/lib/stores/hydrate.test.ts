import { describe, expect, it, vi } from "vitest";
import { hydrateSections, sectionOf, type SectionApi } from "./hydrate";
import type { ServerEvent } from "../transport/transport";

const layout = { display: { width: 1, height: 1, scale_factor: 1 }, edges: [], config: {}, policy: {} };
const settings = { general: { name: "n" }, transfer: {} };
const transfers = { active_count: 0, terminal_count: 0, transfers: [], stats: {} };
const inbox = { count: 0, total_bytes: 0, files: [] };
const activity = { candidates: [], replies: [], messages: [], handoffs: [] };

function api(over: Partial<Record<keyof SectionApi, () => Promise<unknown>>> = {}): SectionApi {
  return {
    layout: async () => layout,
    settings: async () => settings,
    transfers: async () => transfers,
    inbox: async () => inbox,
    activity: async () => activity,
    ...over,
  } as unknown as SectionApi;
}

describe("hydrateSections", () => {
  it("applies every section as a synthetic *.changed event with the given seq", async () => {
    const events: ServerEvent[] = [];
    const applied = await hydrateSections(api(), () => false, (e) => events.push(e), 42);
    expect(applied.sort()).toEqual(["activity", "inbox", "layout", "settings", "transfers"]);
    expect(events.map((e) => e.type).sort()).toEqual([
      "activity.changed",
      "inbox.changed",
      "layout.changed",
      "settings.changed",
      "transfers.changed",
    ]);
    expect(events.every((e) => e.id === 42)).toBe(true);
    expect(events.find((e) => e.type === "settings.changed")?.data).toBe(settings);
  });

  it("drops a result when a real event for that section landed meanwhile", async () => {
    const events: ServerEvent[] = [];
    await hydrateSections(api(), (s) => s === "layout", (e) => events.push(e), 1);
    expect(events.some((e) => e.type === "layout.changed")).toBe(false);
    expect(events.length).toBe(4);
  });

  it("skips failed and malformed sections without throwing", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const events: ServerEvent[] = [];
    const applied = await hydrateSections(
      api({
        settings: async () => ({}), // MockTransport default responder shape
        inbox: async () => {
          throw new Error("boom");
        },
      }),
      () => false,
      (e) => events.push(e),
      1,
    );
    expect(applied.sort()).toEqual(["activity", "layout", "transfers"]);
    expect(events.some((e) => e.type === "settings.changed" || e.type === "inbox.changed")).toBe(false);
    expect(warn).toHaveBeenCalledTimes(2);
    warn.mockRestore();
  });
});

describe("sectionOf", () => {
  it("maps hydrated change events and ignores the rest", () => {
    expect(sectionOf("layout.changed")).toBe("layout");
    expect(sectionOf("activity.changed")).toBe("activity");
    expect(sectionOf("input.changed")).toBeNull();
    expect(sectionOf("snapshot")).toBeNull();
    expect(sectionOf("device.updated")).toBeNull();
  });
});
