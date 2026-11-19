import { describe, expect, it } from "vitest";
import type { Device, Snapshot } from "../api/gen/api";
import { applyEvent, applySnapshot, initialDevices } from "./devices";

const dev = (id: string, online = true): Device => ({
  id,
  name: id,
  alias: null,
  platform: "windows",
  device_type: "other",
  online,
  trusted: true,
  blocked: false,
  path: "lan",
  addr: null,
  rtt_ms: 3,
  last_seen_ms: null,
  caps: [],
  features: [],
  tags: [],
  group: null,
});

const snap = (devices: Device[]): Snapshot => ({
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
    lan_only: false,
  },
  devices,
  pairing: { pin_active: false, pin_expires_in_secs: 0 },
  input: { seat: "local", channel_open: false, peer: null },
});

describe("devices reducer", () => {
  it("snapshot replaces the whole map", () => {
    const s = applySnapshot(initialDevices(), snap([dev("a"), dev("b")]));
    expect(s.loaded).toBe(true);
    expect(s.byId.size).toBe(2);
  });

  it("device.updated upserts and device.removed deletes", () => {
    let s = applySnapshot(initialDevices(), snap([dev("a")]));
    s = applyEvent(s, { id: 2, type: "device.updated", data: dev("b", false) });
    expect(s.byId.get("b")?.online).toBe(false);
    s = applyEvent(s, { id: 3, type: "device.removed", data: { id: "a" } });
    expect(s.byId.has("a")).toBe(false);
  });

  it("fold(snapshot, events) == fold(snapshot') — reducer purity", () => {
    // events after a snapshot must produce the same state as a fresh
    // snapshot carrying the same final data (ADR-139 §10 property).
    const s0 = initialDevices();
    const e1 = { id: 1, type: "device.updated", data: dev("a") };
    const e2 = { id: 2, type: "device.updated", data: dev("a", false) };
    const viaEvents = applyEvent(applyEvent(applyEvent(s0, { id: 0, type: "snapshot", data: snap([]) }), e1), e2);
    const viaSnapshot = applySnapshot(s0, snap([dev("a", false)]));
    expect([...viaEvents.byId]).toEqual([...viaSnapshot.byId]);
  });

  it("unknown events are ignored (forward compatibility)", () => {
    const s = applySnapshot(initialDevices(), snap([dev("a")]));
    expect(applyEvent(s, { id: 9, type: "op.progress", data: {} })).toBe(s);
  });
});
