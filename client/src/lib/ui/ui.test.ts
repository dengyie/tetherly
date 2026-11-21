// Ported awesome-ui primitives: behaviour contracts that pages rely on.
import { fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { hasIcon, uiIcon } from "./icons";
import StatusIndicator from "./StatusIndicator.svelte";
import ProgressBar from "./ProgressBar.svelte";
import EmptyState from "./EmptyState.svelte";
import CommandPalette from "./CommandPalette.svelte";
import ToastStack from "./ToastStack.svelte";
import Switch from "./Switch.svelte";
import { clearToasts, pushToast, toasts, withToast } from "./toast";
import { applyTheme, resolveTheme } from "./theme";
import { get } from "svelte/store";

describe("uiIcon", () => {
  it("renders an inline svg, hidden from AT unless labelled", () => {
    const html = uiIcon("check", { size: 16 });
    expect(html).toContain('<svg width="16"');
    expect(html).toContain('aria-hidden="true"');
    expect(uiIcon("check", { label: "ok" })).toContain('aria-label="ok"');
  });
  it("escapes attributes and knows the app additions", () => {
    expect(uiIcon("x", { className: '"><script>' })).not.toContain("<script>");
    for (const n of ["keyboard", "mouse", "inbox", "layout", "link", "unlink", "settings", "activity"]) {
      expect(hasIcon(n)).toBe(true);
    }
    expect(hasIcon("nope")).toBe(false);
  });
});

describe("StatusIndicator", () => {
  it("maps status to tone and default label", () => {
    const { container, unmount } = render(StatusIndicator, { status: "offline" });
    expect(container.textContent).toContain("Offline");
    expect(container.querySelector("[data-status=offline]")?.className).toContain("rose");
    unmount();
    render(StatusIndicator, { status: "online", label: "已连接", pingMs: 3 });
    expect(screen.getByText("已连接")).toBeTruthy();
    expect(screen.getByText("(3ms)")).toBeTruthy();
  });
});

describe("ProgressBar", () => {
  it("exposes aria-valuenow when determinate, none when indeterminate", () => {
    const { unmount } = render(ProgressBar, { value: 62, label: "report.pdf" });
    expect(screen.getByRole("progressbar").getAttribute("aria-valuenow")).toBe("62");
    expect(screen.getByText("62%")).toBeTruthy();
    unmount();
    render(ProgressBar, { value: null, label: "x" });
    expect(screen.getByRole("progressbar").hasAttribute("aria-valuenow")).toBe(false);
  });
});

describe("EmptyState", () => {
  it("renders title/description and fires the action", async () => {
    const onaction = vi.fn();
    render(EmptyState, { title: "空", description: "描述", actionLabel: "去做", onaction });
    expect(screen.getByText("空")).toBeTruthy();
    expect(screen.getByText("描述")).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "去做" }));
    expect(onaction).toHaveBeenCalledTimes(1);
  });
});

describe("Switch", () => {
  it("is a role=switch reporting aria-checked and toggling", async () => {
    const onchange = vi.fn();
    render(Switch, { checked: false, label: "开关", onchange });
    const sw = screen.getByRole("switch", { name: "开关" });
    expect(sw.getAttribute("aria-checked")).toBe("false");
    await fireEvent.click(sw);
    expect(onchange).toHaveBeenCalledWith(true);
  });
});

describe("CommandPalette", () => {
  const items = [
    { id: "overview", label: "概览", group: "页面", icon: "home" },
    { id: "devices", label: "设备", group: "页面", hint: "已配对设备" },
    { id: "theme-dark", label: "深色主题", group: "外观" },
  ];
  it("filters by query and selects with Enter", async () => {
    const onselect = vi.fn();
    render(CommandPalette, { open: true, items, onselect });
    expect(screen.getByRole("dialog")).toBeTruthy();
    expect(screen.getAllByRole("option").length).toBe(3);
    const input = screen.getByRole("textbox");
    await fireEvent.input(input, { target: { value: "配对" } });
    await waitFor(() => expect(screen.getAllByRole("option").length).toBe(1));
    await fireEvent.keyDown(document, { key: "Enter" });
    expect(onselect).toHaveBeenCalledWith(expect.objectContaining({ id: "devices" }));
  });
  it("Escape closes", async () => {
    const onclose = vi.fn();
    render(CommandPalette, { open: true, items, onclose });
    await fireEvent.keyDown(document, { key: "Escape" });
    expect(onclose).toHaveBeenCalled();
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });
});

describe("toast store + ToastStack", () => {
  beforeEach(() => clearToasts());
  it("push renders, dismiss removes, withToast reports errors", async () => {
    render(ToastStack);
    const id = pushToast("已保存", "success", 0);
    await waitFor(() => expect(screen.getByRole("alert").textContent).toContain("已保存"));
    await fireEvent.click(screen.getByRole("button", { name: "关闭提示" }));
    await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
    expect(get(toasts).find((t) => t.id === id)).toBeUndefined();
    const ok = await withToast(() => Promise.reject(new Error("boom")));
    expect(ok).toBe(false);
    expect(get(toasts)[0]).toMatchObject({ type: "error", message: "boom" });
  });
  it("auto-dismisses after duration", () => {
    vi.useFakeTimers();
    pushToast("x", "info", 100);
    expect(get(toasts).length).toBe(1);
    vi.advanceTimersByTime(150);
    expect(get(toasts).length).toBe(0);
    vi.useRealTimers();
  });
});

describe("theme", () => {
  it("applyTheme toggles html.dark and persists the mode", () => {
    expect(applyTheme("dark")).toBe("dark");
    expect(document.documentElement.classList.contains("dark")).toBe(true);
    expect(localStorage.getItem("tetherly-theme")).toBe("dark");
    expect(applyTheme("light")).toBe("light");
    expect(document.documentElement.classList.contains("dark")).toBe(false);
    expect(["light", "dark"]).toContain(resolveTheme("auto"));
    applyTheme("auto");
  });
});
