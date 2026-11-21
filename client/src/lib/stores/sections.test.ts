import { describe, expect, it } from "vitest";
import { initialTransfers, applyEvent as evTransfers } from "./transfers";
import { initialInbox, applyEvent as evInbox } from "./inbox";
import { initialLayout, applyEvent as evLayout } from "./layout";
import { initialSettings, applyEvent as evSettings } from "./settings";
import type {
  Inbox,
  Layout,
  Settings,
  Transfers,
} from "../api/gen/api";

// These four sections have no snapshot form: the daemon pushes full state
// only through their `*.changed` events (M2c contract).

const transfers = (over: Partial<Transfers> = {}): Transfers => ({
  active_count: 0,
  terminal_count: 0,
  transfers: [],
  stats: { bytes_sent: 0, bytes_received: 0, completed_count: 0, failed_count: 0 },
  ...over,
});

const inbox = (over: Partial<Inbox> = {}): Inbox => ({
  count: 0,
  total_bytes: 0,
  files: [],
  ...over,
});

const layout = (over: Partial<Layout> = {}): Layout => ({
  display: { width: 1470, height: 956, scale_factor: 2 },
  display_auto: true,
  edges: [{ edge: "right", peer: null }],
  config: {
    resistance_px: 15,
    delay_ms: 100,
    corner_lock_size_px: 8,
    wrap_around: false,
    redundancy_band_px: 30,
    slow_dwell_ms: 600,
    fast_push_delta_px: 12,
    entry_return_grace_ms: 600,
    remote_speed_percent: 100,
  },
  policy: { enabled: true, allow_inbound_control: true, auto_link_paired_peers: true },
  ...over,
});

const settings = (over: Partial<Settings> = {}): Settings => ({
  general: {
    name: "mac",
    advertise: true,
    reconnect: true,
    loopback_only: false,
    overlay_enabled: true,
    tray_owner: "daemon",
  },
  transfer: { auto_accept: false },
  ...over,
});

describe("transfers store", () => {
  it("replaces the section on transfers.changed", () => {
    let s = initialTransfers();
    expect(s.loaded).toBe(false);
    s = evTransfers(s, { id: 2, type: "transfers.changed", data: transfers({ active_count: 1 }) });
    expect(s.loaded).toBe(true);
    expect(s.transfers?.active_count).toBe(1);
  });

  it("ignores unrelated events", () => {
    const s = evTransfers(initialTransfers(), { id: 3, type: "transfers.changed", data: transfers() });
    expect(evTransfers(s, { id: 4, type: "inbox.changed", data: inbox() })).toBe(s);
  });
});

describe("inbox store", () => {
  it("replaces the section on inbox.changed", () => {
    let s = initialInbox();
    s = evInbox(s, { id: 5, type: "inbox.changed", data: inbox({ count: 1 }) });
    expect(s.loaded).toBe(true);
    expect(s.inbox?.count).toBe(1);
  });

  it("ignores unrelated events", () => {
    const s = evInbox(initialInbox(), { id: 6, type: "inbox.changed", data: inbox() });
    expect(evInbox(s, { id: 7, type: "transfers.changed", data: transfers() })).toBe(s);
  });
});

describe("layout store", () => {
  it("replaces the section on layout.changed", () => {
    let s = initialLayout();
    s = evLayout(s, { id: 8, type: "layout.changed", data: layout() });
    expect(s.loaded).toBe(true);
    expect(s.layout?.display.width).toBe(1470);
    s = evLayout(s, {
      id: 9,
      type: "layout.changed",
      data: layout({ display: { ...layout().display, width: 1920 } }),
    });
    expect(s.layout?.display.width).toBe(1920);
  });

  it("ignores unrelated events", () => {
    const s = evLayout(initialLayout(), { id: 10, type: "layout.changed", data: layout() });
    expect(evLayout(s, { id: 11, type: "settings.changed", data: settings() })).toBe(s);
  });
});

describe("settings store", () => {
  it("replaces the section on settings.changed", () => {
    let s = initialSettings();
    s = evSettings(s, { id: 12, type: "settings.changed", data: settings() });
    expect(s.loaded).toBe(true);
    expect(s.settings?.general.name).toBe("mac");
    s = evSettings(s, {
      id: 13,
      type: "settings.changed",
      data: settings({ general: { ...settings().general, name: "renamed" } }),
    });
    expect(s.settings?.general.name).toBe("renamed");
  });

  it("ignores unrelated events", () => {
    const s = evSettings(initialSettings(), { id: 14, type: "settings.changed", data: settings() });
    expect(evSettings(s, { id: 15, type: "layout.changed", data: layout() })).toBe(s);
  });
});

// ---- M2e: activity store ----
import { initialActivity, applyEvent as evActivity } from "./activity";
import type { Activity } from "../api/gen/api";

const activity = (over: Partial<Activity> = {}): Activity => ({
  candidates: [],
  replies: [],
  messages: [],
  handoffs: [],
  ...over,
});

describe("activity store", () => {
  it("replaces the section on activity.changed", () => {
    let s = initialActivity();
    s = evActivity(s, {
      id: 16,
      type: "activity.changed",
      data: activity({
        messages: [
          {
            id: "m1",
            peer: "tdev_a",
            direction: "inbound",
            text: "hi",
            timestamp_ms: 1,
            read: false,
          },
        ],
        handoffs: [
          {
            id: "h1",
            sender_id: "tdev_a",
            sender_alias: "mac",
            url: "https://x.test",
            title: "page",
            status: "pending",
            received_ms: 2,
            opened_ms: null,
          },
        ],
      }),
    });
    expect(s.loaded).toBe(true);
    expect(s.activity?.messages[0].text).toBe("hi");
    expect(s.activity?.handoffs[0].status).toBe("pending");
    s = evActivity(s, { id: 17, type: "activity.changed", data: activity() });
    expect(s.activity?.messages).toHaveLength(0);
  });

  it("ignores unrelated events", () => {
    const s = evActivity(initialActivity(), {
      id: 18,
      type: "activity.changed",
      data: activity(),
    });
    expect(evActivity(s, { id: 19, type: "settings.changed", data: settings() })).toBe(s);
  });
});
