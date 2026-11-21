// Layout editor over the MockTransport-backed Api: every edit must become
// exactly the topology request the daemon expects; the page itself stays
// store-driven (no local mutation of `data`).
import { fireEvent, render, screen } from "@testing-library/svelte";
import { describe, expect, it } from "vitest";
import Layout from "../routes/Layout.svelte";
import { Api } from "./api/client";
import { MockTransport } from "./transport/mock";
import type { Device, Layout as LayoutData } from "./api/gen/api";
import type { DevicesState } from "./stores/devices";

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

const layout = (over: Partial<LayoutData> = {}): LayoutData => ({
  display: { width: 2560, height: 1440, scale_factor: 2 },
  display_auto: true,
  edges: [
    { edge: "left", peer: null },
    { edge: "right", peer: "d1" },
    { edge: "top", peer: null },
    { edge: "bottom", peer: null },
  ],
  config: {
    resistance_px: 40,
    delay_ms: 150,
    corner_lock_size_px: 20,
    wrap_around: false,
    redundancy_band_px: 6,
    slow_dwell_ms: 200,
    fast_push_delta_px: 30,
    entry_return_grace_ms: 300,
    remote_speed_percent: 100,
  },
  policy: { enabled: true, allow_inbound_control: true, auto_link_paired_peers: false },
  ...over,
});

function mountLayout(data: LayoutData | null = layout(), responder?: () => unknown) {
  const t = new MockTransport(responder);
  const devices: DevicesState = {
    byId: new Map([
      ["d1", device("d1", "WIN")],
      ["d2", device("d2", "PAD", { online: false })],
      ["d3", device("d3", "BAD", { blocked: true, trusted: false })],
    ]),
    loaded: true,
  };
  render(Layout, { data, devices, api: new Api(t) });
  return t;
}

describe("layout editor", () => {
  it("resolves edge peers to device names and shows the display", () => {
    mountLayout();
    expect(screen.getByTestId("layout-display").textContent).toContain("2560 × 1440");
    expect(screen.getByTestId("layout-display").textContent).toContain("缩放 200%");
    expect(screen.getByTestId("layout-display").textContent).toContain("自动检测");
    expect(screen.queryByRole("button", { name: /恢复自动/ })).toBeNull();
    const right = screen.getByRole("group", { name: "右侧" });
    expect(right.textContent).toContain("WIN");
    expect(screen.getByRole("group", { name: "左侧" }).textContent).toContain("未配置");
    expect(screen.getByText(/已连接 1 \/ 4/)).toBeTruthy();
  });

  it("picks a device for an empty edge and POSTs topology/links", async () => {
    const t = mountLayout();
    await fireEvent.click(screen.getByRole("button", { name: /左侧 · 未配置/ }));
    const menu = screen.getByRole("menu");
    // Blocked devices are not offered.
    expect(menu.textContent).not.toContain("BAD");
    await fireEvent.click(screen.getByRole("menuitem", { name: /PAD/ }));
    expect(t.calls).toEqual([{ method: "POST", path: "/api/v1/topology/links", body: { edge: "left", peer: "d2" } }]);
  });

  it("unlinks an edge via POST topology/unlinks", async () => {
    const t = mountLayout();
    await fireEvent.click(screen.getByRole("button", { name: "断开右侧" }));
    expect(t.calls).toEqual([{ method: "POST", path: "/api/v1/topology/unlinks", body: { edge: "right" } }]);
  });

  it("drops a dragged device chip onto an edge", async () => {
    const t = mountLayout();
    const top = screen.getByRole("group", { name: "上侧" });
    const dt = { getData: () => "d1", setData: () => {}, effectAllowed: "" };
    await fireEvent.drop(top, { dataTransfer: dt });
    expect(t.calls).toEqual([{ method: "POST", path: "/api/v1/topology/links", body: { edge: "top", peer: "d1" } }]);
  });

  it("commits config sliders on change and policy switches via PATCH", async () => {
    const t = mountLayout();
    const slider = screen.getByLabelText("推动阻力") as HTMLInputElement;
    slider.value = "80";
    await fireEvent.change(slider);
    await fireEvent.click(screen.getByRole("switch", { name: "边缘环绕" }));
    await fireEvent.click(screen.getByRole("switch", { name: "允许被远端控制" }));
    expect(t.calls).toEqual([
      { method: "PATCH", path: "/api/v1/topology/config", body: { resistance_px: 80 } },
      { method: "PATCH", path: "/api/v1/topology/config", body: { wrap_around: true } },
      { method: "PATCH", path: "/api/v1/topology/policy", body: { allow_inbound_control: false } },
    ]);
  });

  it("keeps sliders and switches focusable while a PATCH is in flight", async () => {
    // A pending request must not `disabled` the control the user is on:
    // Chrome blurs disabled elements, so the next ArrowLeft/Space would be lost.
    const t = mountLayout(layout(), () => new Promise(() => {}));
    const slider = screen.getByLabelText("推动阻力") as HTMLInputElement;
    slider.focus();
    slider.value = "45";
    await fireEvent.change(slider);
    expect(t.calls).toHaveLength(1);
    expect(slider.disabled).toBe(false);
    expect(slider.getAttribute("aria-busy")).toBe("true");
    expect(document.activeElement).toBe(slider);
    const sw = screen.getByRole("switch", { name: "边缘环绕" }) as HTMLButtonElement;
    expect(sw.disabled).toBe(false);
    // Buttons that trigger whole-page operations still lock while busy.
    expect((screen.getByRole("button", { name: /同步到对端/ }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("offers 恢复自动 for a manually pinned display and PATCHes display_auto (§221)", async () => {
    const t = mountLayout(layout({ display_auto: false }));
    expect(screen.getByTestId("layout-display").textContent).toContain("手动设置");
    await fireEvent.click(screen.getByRole("button", { name: /恢复自动/ }));
    expect(t.calls).toEqual([{ method: "PATCH", path: "/api/v1/topology/config", body: { display_auto: true } }]);
  });

  it("syncs topology to peers", async () => {
    const t = mountLayout();
    await fireEvent.click(screen.getByRole("button", { name: /同步到对端/ }));
    expect(t.calls).toEqual([{ method: "POST", path: "/api/v1/topology/sync", body: undefined }]);
  });

  it("shows a waiting state before the first layout.changed", () => {
    mountLayout(null);
    expect(screen.getByText("等待布局数据…")).toBeTruthy();
  });
});
