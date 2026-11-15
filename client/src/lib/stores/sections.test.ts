import { describe, expect, it } from "vitest";
import type { Snapshot } from "../api/gen/api";
import { applyEvent, applySnapshot, initialPairing } from "./pairing";
import { applyEvent as evInput, applySnapshot as snapInput, initialInput } from "./input";
import { applyEvent as evSystem, applySnapshot as snapSystem, initialSystem } from "./system";

const snapshot = (over: Partial<Snapshot> = {}): Snapshot => ({
  system: {
    device_id: "self",
    name: "mac",
    platform: "macos",
    version: "0.2.58",
    api: 1,
    pid: 1,
    uptime_secs: 1,
    ports: { control: 45717, file: 45718, input: 45719, screen: 45720, ui: 45716 },
    peers: { live: 0, trusted: 0 },
    caps: [],
    features: [],
  },
  devices: [],
  pairing: { pin_active: false, pin_expires_in_secs: 0 },
  input: { seat: "local", channel_open: false, peer: null },
  ...over,
});

describe("system store", () => {
  it("loads from the snapshot and replaces on system.changed", () => {
    let s = initialSystem();
    expect(s.loaded).toBe(false);
    s = snapSystem(s, snapshot());
    expect(s.loaded).toBe(true);
    expect(s.system?.name).toBe("mac");
    s = evSystem(s, { id: 2, type: "system.changed", data: { ...s.system!, name: "renamed" } });
    expect(s.system?.name).toBe("renamed");
  });

  it("ignores unrelated events", () => {
    const s = snapSystem(initialSystem(), snapshot());
    expect(evSystem(s, { id: 3, type: "device.removed", data: { id: "x" } })).toBe(s);
  });
});

describe("pairing store", () => {
  it("loads from the snapshot and replaces on pairing.changed", () => {
    let s = initialPairing();
    s = applySnapshot(s, snapshot());
    expect(s.pairing?.pin_active).toBe(false);
    s = applyEvent(s, {
      id: 4,
      type: "pairing.changed",
      data: { pin_active: true, pin_expires_in_secs: 120 },
    });
    expect(s.pairing?.pin_active).toBe(true);
    expect(s.pairing?.pin_expires_in_secs).toBe(120);
  });

  it("ignores unrelated events", () => {
    const s = applySnapshot(initialPairing(), snapshot());
    expect(applyEvent(s, { id: 5, type: "input.changed", data: { seat: "x", channel_open: false, peer: null } })).toBe(s);
  });
});

describe("input store", () => {
  it("loads from the snapshot and replaces on input.changed", () => {
    let s = initialInput();
    s = snapInput(s, snapshot());
    expect(s.input?.seat).toBe("local");
    s = evInput(s, {
      id: 6,
      type: "input.changed",
      data: { seat: "peer", channel_open: true, peer: "dev-1" },
    });
    expect(s.input?.seat).toBe("peer");
    expect(s.input?.peer).toBe("dev-1");
  });

  it("ignores unrelated events", () => {
    const s = snapInput(initialInput(), snapshot());
    expect(evInput(s, { id: 7, type: "pairing.changed", data: { pin_active: false, pin_expires_in_secs: 0 } })).toBe(s);
  });
});
