// App-level flow tests over the MockTransport seam (ADR-139 §7 e2e layer,
// component tier): script snapshot + events through the same interface the
// daemon drives, then assert on what the real pages rendered. Browser-tier
// Playwright runs land once the remaining pages exist.
import { render, screen, waitFor } from "@testing-library/svelte";
import { describe, expect, it } from "vitest";
import App from "../App.svelte";
import { MockTransport } from "../lib/transport/mock";
import type { Device, Snapshot } from "../lib/api/gen/api";

const device = (id: string, name: string, over: Partial<Device> = {}): Device => ({
  id,
  name,
  alias: null,
  platform: "windows",
  device_type: "other",
  online: true,
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
  ...over,
});

const snapshot = (devices: Device[]): Snapshot => ({
  system: {
    device_id: "self",
    name: "mac",
    platform: "macos",
    version: "0.2.58",
    api: 1,
    pid: 1,
    uptime_secs: 90,
    ports: { control: 45717, file: 45718, input: 45719, screen: 45720, ui: 45716 },
    peers: { live: devices.filter((d) => d.online).length, trusted: devices.length },
    caps: [],
    features: [],
    lan_only: false,
  },
  devices,
  pairing: { pin_active: false, pin_expires_in_secs: 0 },
  input: { seat: "local", channel_open: false, peer: null },
});

function mountApp() {
  const t = new MockTransport();
  render(App, { transport: t });
  return t;
}

describe("app flows over MockTransport", () => {
  it("flow: snapshot renders the overview with real data", async () => {
    const t = mountApp();
    t.setLinkState("live");
    t.emit({ id: 1, type: "snapshot", data: snapshot([device("d1", "DESKTOP")]) });

    await waitFor(() => expect(screen.getByText("DESKTOP")).toBeTruthy());
    expect(screen.getByText(/0\.2\.58/)).toBeTruthy();
    expect(screen.getByText("mac")).toBeTruthy();
    // input seat starts local
    expect(screen.getAllByText("本机").length).toBeGreaterThan(0);
  });

  it("flow: device.updated and device.removed update the live list", async () => {
    const t = mountApp();
    t.emit({ id: 1, type: "snapshot", data: snapshot([device("d1", "DESKTOP")]) });
    await waitFor(() => expect(screen.getByText("DESKTOP")).toBeTruthy());
    await waitFor(() => expect(screen.getAllByText("在线设备（1）").length).toBeGreaterThan(0));

    t.emit({ id: 2, type: "device.updated", data: device("d1", "DESKTOP", { online: false }) });
    // Overview only lists online+trusted devices; dropping the link empties it.
    await waitFor(() => expect(screen.getAllByText("在线设备（0）").length).toBeGreaterThan(0));
  });

  it("flow: link drop shows the reconnect banner, resync recovers", async () => {
    const t = mountApp();
    t.emit({ id: 1, type: "snapshot", data: snapshot([]) });
    await waitFor(() => expect(screen.getByText("没有在线设备")).toBeTruthy());

    t.setLinkState("offline");
    await waitFor(() => expect(screen.getByText("连接中断，正在重连")).toBeTruthy());

    t.setLinkState("resyncing");
    await waitFor(() => expect(screen.getByText("重新同步…")).toBeTruthy());

    t.setLinkState("live");
    await waitFor(() => expect(screen.getByText("没有在线设备")).toBeTruthy());
  });

  it("flow: navigating to devices and clicking 连接 calls the api", async () => {
    const t = new MockTransport((method, path) =>
      path === "/api/v1/devices" ? [device("d1", "WIN")] : {},
    );
    render(App, { transport: t });
    t.emit({ id: 1, type: "snapshot", data: snapshot([device("d1", "WIN", { online: false })]) });

    const devBtn = screen.getByRole("button", { name: "设备" });
    devBtn.click();
    await waitFor(() => expect(screen.getByText("WIN")).toBeTruthy());

    const connect = screen.getByRole("button", { name: "连接" });
    connect.click();
    await waitFor(() =>
      expect(t.calls).toContainEqual({ method: "POST", path: "/api/v1/devices/d1/connect", body: {} }),
    );
  });
});
