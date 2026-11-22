// Tray panel (§6.3): the compact /tray route mounted in the 360×480
// frameless window. The shell opens it with the #tray fragment; these tests
// mount App with that fragment pre-set and drive the MockTransport seam.
import { render, screen, waitFor } from "@testing-library/svelte";
import { afterEach, describe, expect, it } from "vitest";
import App from "../App.svelte";
import { MockTransport } from "../lib/transport/mock";
import type { Device, Input, Snapshot, Transfer, Transfers } from "../lib/api/gen/api";

// #tray must be present BEFORE mount (the shell opens the window with it).
function mountTray() {
  location.hash = "#tray";
  const t = new MockTransport();
  render(App, { transport: t });
  return t;
}

afterEach(() => {
  location.hash = "";
});

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

const localInput: Input = { seat: "local", channel_open: false, peer: null, role: null, cursor_here: true };

const snapshot = (devices: Device[], input: Input = localInput): Snapshot => ({
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
  input,
});

const transfer = (over: Partial<Transfer>): Transfer => ({
  transfer_id: "t1",
  peer: "d1",
  direction: "inbound",
  filename: "report.pdf",
  total: 100,
  transferred: 62,
  speed_bps: 0,
  phase: "transferring",
  error: null,
  started_ms: 0,
  updated_ms: 0,
  ...over,
});

describe("tray panel compact route", () => {
  it("renders the compact view (no nav) with live seat/devices/transfers", async () => {
    const t = mountTray();
    t.setLinkState("live");
    t.emit({ id: 1, type: "snapshot", data: snapshot([device("d1", "DESKTOP")]) });

    await waitFor(() => expect(screen.getByTestId("tray-state").textContent).toBe("键鼠在本机"));
    // Compact: no main-window navigation buttons.
    expect(screen.queryByRole("button", { name: "概览" })).toBeNull();
    expect(screen.getByText("DESKTOP")).toBeTruthy();
    expect(screen.getByText("3ms")).toBeTruthy();
    expect(screen.getByText("没有进行中的传输")).toBeTruthy();
    expect(screen.getByTestId("tray-toggle").textContent).toBe("暂停共享");
  });

  it("takeover + advertise=off wording matches the Rust tray fold", async () => {
    const t = mountTray();
    t.setLinkState("live");
    t.emit({ id: 1, type: "snapshot", data: snapshot([]) });
    await waitFor(() => expect(screen.getByTestId("tray-state").textContent).toBe("键鼠在本机"));

    t.emit({
      id: 2,
      type: "input.changed",
      data: { seat: "remote", channel_open: true, peer: "d1", role: "controller", cursor_here: false },
    });
    await waitFor(() => expect(screen.getByTestId("tray-state").textContent).toBe("键鼠在 d1"));

    t.emit({ id: 3, type: "input.changed", data: localInput });
    t.emit({
      id: 4,
      type: "settings.changed",
      data: {
        general: { name: "mac", advertise: false, reconnect: true, loopback_only: true, overlay_enabled: true, tray_owner: "daemon" },
        transfer: { auto_accept: false },
      },
    });
    await waitFor(() => expect(screen.getByTestId("tray-state").textContent).toBe("共享已暂停"));
    expect(screen.getByTestId("tray-toggle").textContent).toBe("恢复共享");
  });

  it("target side labels by physical cursor, not by seat (§229)", async () => {
    const t = mountTray();
    t.setLinkState("live");
    // Windows target while the Mac controller keeps its cursor at home: this
    // node's seat is still "local", yet the keyboard/mouse are on the Mac.
    t.emit({
      id: 1,
      type: "snapshot",
      data: snapshot([device("d1", "MAC", { platform: "macos" })], {
        seat: "local",
        channel_open: false,
        peer: "d1",
        role: "target",
        cursor_here: false,
      }),
    });
    await waitFor(() => expect(screen.getByTestId("tray-state").textContent).toBe("键鼠在 MAC"));
    // "收回" is a controller-only affordance.
    expect(screen.getByTestId("tray-sub").textContent).not.toContain("收回");

    // The controller's cursor entered this screen: the target is being driven.
    t.emit({
      id: 2,
      type: "input.changed",
      data: { seat: "remote", channel_open: false, peer: "d1", role: "target", cursor_here: true },
    });
    await waitFor(() => expect(screen.getByTestId("tray-state").textContent).toBe("键鼠在本机"));
    expect(screen.getByTestId("tray-sub").textContent).toContain("MAC");

    t.setLinkState("offline");
    await waitFor(() => expect(screen.getByTestId("tray-state").textContent).toBe("daemon 未连接"));
  });

  it("active transfers show progress; finished ones drop off", async () => {
    const t = mountTray();
    t.emit({ id: 1, type: "snapshot", data: snapshot([]) });
    const list: Transfers = {
      active_count: 1,
      terminal_count: 0,
      transfers: [transfer({})],
      stats: { bytes_sent: 0, bytes_received: 0, completed_count: 0, failed_count: 0 },
    };
    t.emit({ id: 2, type: "transfers.changed", data: list });
    await waitFor(() => expect(screen.getByText("report.pdf")).toBeTruthy());
    expect(screen.getByText("62%")).toBeTruthy();

    t.emit({
      id: 3,
      type: "transfers.changed",
      data: {
        active_count: 0,
        terminal_count: 1,
        transfers: [transfer({ phase: "completed" })],
        stats: { bytes_sent: 0, bytes_received: 0, completed_count: 0, failed_count: 0 },
      },
    });
    await waitFor(() => expect(screen.getByText("没有进行中的传输")).toBeTruthy());
  });

  it("pause toggle PATCHes advertise and emits the tray action", async () => {
    const t = new MockTransport((method, path, body) =>
      method === "PATCH" && path === "/api/v1/settings" ? body : {},
    );
    location.hash = "#tray";
    render(App, { transport: t });
    t.emit({ id: 1, type: "snapshot", data: snapshot([]) });
    await waitFor(() => expect(screen.getByTestId("tray-toggle")).toBeTruthy());

    screen.getByTestId("tray-toggle").click();
    await waitFor(() =>
      expect(t.calls).toContainEqual({
        method: "PATCH",
        path: "/api/v1/settings",
        body: { advertise: false },
      }),
    );
  });

  // §225: the daemon's native menubar panel opens `#tray&token=…`. The route
  // must still mount the compact view, the token must leave the URL (kept
  // only in sessionStorage) and `#tray` must survive so a reload stays in
  // panel mode.
  it("daemon handoff #tray&token= mounts the panel and strips the token", async () => {
    sessionStorage.removeItem("tetherly_token");
    location.hash = "#tray&token=abc%2Fdef";
    const t = new MockTransport();
    render(App, { transport: t });
    t.emit({ id: 1, type: "snapshot", data: snapshot([]) });
    await waitFor(() => expect(screen.getByTestId("tray-panel")).toBeTruthy());
    expect(location.hash).toBe("#tray");
    expect(sessionStorage.getItem("tetherly_token")).toBe("abc/def");
    expect(document.documentElement.dataset.tray).toBe("panel");
  });

  // §225: inside the native panel, actions go to the WKWebView script-message
  // handler (`webkit.messageHandlers.tetherlyTray`) as a JSON string.
  it("footer actions post JSON to the native tetherlyTray bridge when present", async () => {
    const posted: string[] = [];
    (globalThis as { webkit?: unknown }).webkit = {
      messageHandlers: { tetherlyTray: { postMessage: (m: string) => posted.push(m) } },
    };
    try {
      location.hash = "#tray";
      const t = new MockTransport();
      render(App, { transport: t });
      t.emit({ id: 1, type: "snapshot", data: snapshot([]) });
      await waitFor(() => expect(screen.getByTestId("tray-open")).toBeTruthy());
      screen.getByTestId("tray-open").click();
      screen.getByTestId("tray-quit").click();
      expect(posted.map((m) => JSON.parse(m).action)).toEqual(["open-main", "quit"]);
    } finally {
      delete (globalThis as { webkit?: unknown }).webkit;
    }
  });
});
