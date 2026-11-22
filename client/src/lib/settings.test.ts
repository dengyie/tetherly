// Settings page over the MockTransport-backed Api: switches and the name
// form must become the exact PATCH the daemon expects, and controls stay
// focusable while a save is in flight (aria-busy, never `disabled`).
import { fireEvent, render, screen } from "@testing-library/svelte";
import { describe, expect, it } from "vitest";
import Settings from "../routes/Settings.svelte";
import { Api } from "./api/client";
import { MockTransport } from "./transport/mock";
import type { Settings as SettingsData } from "./api/gen/api";

const settings = (over: Partial<SettingsData> = {}): SettingsData => ({
  general: { name: "mac-studio", advertise: true, reconnect: true, loopback_only: false, overlay_enabled: true, tray_owner: "daemon" },
  transfer: { auto_accept: false },
  ...over,
});

function mount(data: SettingsData | null = settings(), responder?: () => unknown) {
  const t = new MockTransport(responder);
  render(Settings, { data, api: new Api(t) });
  return t;
}

describe("settings page", () => {
  it("PATCHes switches and the trimmed node name, skipping a no-op save", async () => {
    const t = mount();
    await fireEvent.click(screen.getByRole("switch", { name: "mDNS 广播" }));
    await fireEvent.click(screen.getByRole("switch", { name: "自动接收文件" }));
    const name = screen.getByLabelText("节点名称") as HTMLInputElement;
    await fireEvent.submit(name.closest("form")!);
    await fireEvent.input(name, { target: { value: "  studio-2  " } });
    await fireEvent.submit(name.closest("form")!);
    expect(t.calls).toEqual([
      { method: "GET", path: "/api/v1/input/gesture", body: undefined },
      { method: "PATCH", path: "/api/v1/settings", body: { advertise: false } },
      { method: "PATCH", path: "/api/v1/settings", body: { auto_accept: true } },
      { method: "PATCH", path: "/api/v1/settings", body: { name: "studio-2" } },
    ]);
  });

  it("PATCHes a remapped gesture binding and echoes the map back", async () => {
    const t = mount();
    // The gesture map self-fetches on mount; flush before the selects exist.
    await new Promise((r) => setTimeout(r, 0));
    const sel = screen.getByRole("combobox", { name: "四指左滑动作" }) as HTMLSelectElement;
    sel.value = "app_switch_next";
    await fireEvent.change(sel);
    expect(t.calls).toEqual([
      { method: "GET", path: "/api/v1/input/gesture", body: undefined },
      { method: "PATCH", path: "/api/v1/input/gesture", body: { swipe4_left: "app_switch_next" } },
    ]);
  });

  it("keeps a focused switch enabled while its PATCH is pending", async () => {
    const t = mount(settings(), () => new Promise(() => {}));
    const sw = screen.getByRole("switch", { name: "自动重连" }) as HTMLButtonElement;
    sw.focus();
    await fireEvent.click(sw);
    expect(t.calls.filter((c) => c.method === "PATCH")).toHaveLength(1);
    expect(sw.disabled).toBe(false);
    expect(sw.getAttribute("aria-busy")).toBe("true");
    expect(document.activeElement).toBe(sw);
  });

  it("shows a waiting state before the first settings.changed", () => {
    mount(null);
    // One placeholder per card (常规 + 传输) until the store fills.
    expect(screen.getAllByText("等待设置数据…")).toHaveLength(2);
  });
});
