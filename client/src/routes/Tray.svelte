<script lang="ts">
  // Menubar panel (§6.3 / §225): the same SPA mounts this route in the
  // 380-wide frameless panel — the shell's Tauri popover or the daemon's
  // native frosted NSPanel. Layout follows Magpie's menubar panel: brand
  // header with icon actions, stacked cards (status / 设备 / 传输) on a
  // translucent surface, text-button footer. No navigation.
  import type { DevicesState } from "../lib/stores/devices";
  import type { InputState } from "../lib/stores/input";
  import type { TransfersState } from "../lib/stores/transfers";
  import type { LinkState } from "../lib/transport/transport";
  import type { SettingsState } from "../lib/stores/settings";
  import type { Api } from "../lib/api/client";
  import { onMount } from "svelte";
  import { emitter } from "../lib/transport/traybridge";
  import { brandLogo, brandName } from "../lib/brand";
  import Icon from "../lib/ui/Icon.svelte";

  let {
    devices,
    input,
    transfers,
    settings,
    link,
    api,
  }: {
    devices: DevicesState;
    input: InputState;
    transfers: TransfersState;
    settings: SettingsState;
    link: LinkState;
    api: Api | null;
  } = $props();

  const online = $derived([...devices.byId.values()].filter((d) => d.online && d.trusted));
  // Sections land via events (`transfers.changed`); null until first frame.
  const activeTransfers = $derived(
    transfers.transfers?.transfers.filter((t) => t.phase === "transferring" || t.phase === "verifying") ?? [],
  );
  const paused = $derived(!!settings.settings && !settings.settings.general.advertise);

  // Same wording contract as the Rust tray fold (single source per meaning).
  const stateLine = $derived.by(() => {
    if (link !== "live") return link === "offline" ? "daemon 未连接" : "重新同步…";
    const i = input.input;
    if (!i) return "…";
    if (i.seat === "remote") return i.peer ? `键鼠在 ${i.peer}` : "键鼠被远端接管";
    if (paused) return "共享已暂停";
    return "键鼠在本机";
  });
  const stateSub = $derived.by(() => {
    if (link !== "live") return "等待本机 daemon";
    if (paused) return "设备对局域网不可见";
    if (input.input?.seat === "remote") return "按 Ctrl+Alt+Esc 收回";
    return online.length === 0 ? "共享中 · 等待设备" : `共享中 · ${online.length} 台在线`;
  });
  const tone = $derived(link !== "live" ? "err" : paused ? "warn" : input.input?.seat === "remote" ? "info" : "ok");

  let busy = $state(false);
  async function toggleShare() {
    if (!api || busy) return;
    const on = settings.settings?.general.advertise ?? true;
    busy = true;
    try {
      await api.patchSettings({ advertise: !on });
      emitter.send("tray-action", { action: "toggled", value: !on });
    } catch (e) {
      emitter.send("tray-action", { action: "error", message: String(e) });
    } finally {
      busy = false;
    }
  }

  const pct = (t: { transferred: number; total: number }) => (t.total > 0 ? Math.floor((t.transferred / t.total) * 100) : 0);
  const openMain = () => emitter.send("tray-action", { action: "open-main" });
  const quit = () => emitter.send("tray-action", { action: "quit" });
  const refresh = () => location.reload();

  // Content-fitted height (§225): like Magpie, the panel is as tall as its
  // cards. The host clamps and animates; above the max the view scrolls.
  let topEl = $state<HTMLElement | null>(null);
  let viewEl = $state<HTMLElement | null>(null);
  let stackEl = $state<HTMLElement | null>(null);
  let footEl = $state<HTMLElement | null>(null);
  let lastHeight = 0;
  function reportHeight() {
    if (!topEl || !viewEl || !stackEl || !footEl) return;
    const cs = getComputedStyle(viewEl);
    const pad = (parseFloat(cs.paddingTop) || 0) + (parseFloat(cs.paddingBottom) || 0);
    const h = Math.ceil(topEl.offsetHeight + stackEl.offsetHeight + pad + footEl.offsetHeight);
    if (h <= 0 || h === lastHeight) return;
    lastHeight = h;
    emitter.send("tray-action", { action: "resize", height: h });
  }
  onMount(() => {
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(() => reportHeight());
    for (const el of [topEl, stackEl, footEl]) if (el) ro.observe(el);
    reportHeight();
    return () => ro.disconnect();
  });
</script>

<div class="tp" data-testid="tray-panel">
  <header class="tp-top" bind:this={topEl}>
    <div class="tp-brand">
      <img src={brandLogo} alt="" width="18" height="18" draggable="false" />
      <span>{brandName}</span>
    </div>
    <div class="tp-actions">
      <button type="button" class="tp-ic" title="刷新" aria-label="刷新" onclick={refresh}><Icon name="refresh" size={14} /></button>
      <button type="button" class="tp-ic" title="打开主窗口" aria-label="打开主窗口" onclick={openMain}><Icon name="external-link" size={14} /></button>
    </div>
  </header>

  <main class="tp-view" bind:this={viewEl}>
    <div class="tp-stack" bind:this={stackEl}>
    <section class="list">
      <div class="row">
        <span class="tp-dot {tone}"></span>
        <div class="main">
          <div class="name" data-testid="tray-state">{stateLine}</div>
          <div class="sub">{stateSub}</div>
        </div>
        <div class="acts">
          <button type="button" class="btn {paused ? 'btn-primary' : ''}" disabled={!api} aria-busy={busy} onclick={toggleShare} data-testid="tray-toggle">{paused ? "恢复共享" : "暂停共享"}</button>
        </div>
      </div>
    </section>

    <section class="list">
      <div class="list-title">设备</div>
      {#if online.length === 0}
        <div class="empty">没有在线设备</div>
      {:else}
        {#each online as d (d.id)}
          <div class="row compact" data-testid="tray-device">
            <span class="ic"><Icon name="monitor" size={14} /></span>
            <div class="main"><div class="name truncate">{d.name}</div></div>
            <span class="val mono">{d.rtt_ms != null ? `${d.rtt_ms}ms` : ""}</span>
          </div>
        {/each}
      {/if}
    </section>

    <section class="list">
      <div class="list-title">传输</div>
      {#if activeTransfers.length === 0}
        <div class="empty">没有进行中的传输</div>
      {:else}
        {#each activeTransfers as t (t.transfer_id)}
          <div class="row" data-testid="tray-transfer">
            <span class="ic"><Icon name={t.direction === "inbound" ? "download" : "upload"} size={14} /></span>
            <div class="main">
              <div class="name truncate">{t.filename}</div>
              <div class="tp-bar"><i style="width: {pct(t)}%"></i></div>
            </div>
            <span class="val mono">{pct(t)}%</span>
          </div>
        {/each}
      {/if}
    </section>
    </div>
  </main>

  <footer class="tp-foot" bind:this={footEl}>
    <button type="button" class="tp-txt" onclick={openMain} data-testid="tray-open">打开主窗口</button>
    <button type="button" class="tp-txt" onclick={quit} data-testid="tray-quit">退出</button>
  </footer>
</div>

<style>
  /* Magpie menubar-panel tokens (captured 2026-11-22): translucent surface,
     edge-shadow cards (no border), 46px header, 57px footer. */
  .tp {
    --card-edge: 0 0 0 1px rgba(0, 0, 0, 0.05), 0 1px 2px rgba(0, 0, 0, 0.04);
    display: flex;
    flex-direction: column;
    height: 100%;
    background: color-mix(in srgb, var(--bg) 93%, transparent);
    color: var(--fg);
    font-size: 13px;
    border-radius: 12px;
    overflow: hidden;
  }
  :global(html.dark) .tp {
    --card-edge: inset 0 0 0 1px rgba(255, 255, 255, 0.06);
  }
  .tp-top {
    display: flex;
    align-items: center;
    min-height: 46px;
    padding: 6px 6px 4px 12px;
    flex-shrink: 0;
  }
  .tp-brand {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-weight: 600;
  }
  .tp-brand img { width: 18px; height: 18px; border-radius: 5px; }
  .tp-actions { margin-left: auto; display: inline-flex; gap: 2px; }
  .tp-ic {
    width: 36px;
    height: 36px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: 0;
    border-radius: 7px;
    background: transparent;
    color: var(--muted);
    cursor: pointer;
    transition: background 0.15s var(--ease), color 0.15s var(--ease);
  }
  .tp-ic:hover { background: var(--pill-hover); color: var(--fg); }
  .tp-view {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 2px 12px 12px;
  }
  .tp-stack {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .tp .list { border: 0; border-radius: 10px; box-shadow: var(--card-edge); flex-shrink: 0; }
  .tp .row { min-height: 40px; padding: 7px 12px; gap: 9px; }
  .tp .row.compact { min-height: 36px; padding: 5px 12px; }
  .tp .row .name { font-size: 13.5px; }
  .tp .row .val { font-size: 11.5px; color: var(--muted); }
  .tp .row .ic { width: 22px; height: 22px; display: inline-flex; align-items: center; justify-content: center; color: var(--muted); flex-shrink: 0; }
  .tp .list-title { padding: 10px 12px 2px; }
  .tp-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
    margin: 0 4px;
    background: var(--ok);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--ok) 18%, transparent);
  }
  .tp-dot.warn { background: var(--warn); box-shadow: 0 0 0 3px color-mix(in srgb, var(--warn) 18%, transparent); }
  .tp-dot.err { background: var(--err); box-shadow: 0 0 0 3px color-mix(in srgb, var(--err) 18%, transparent); }
  .tp-dot.info { background: #3b82f6; box-shadow: 0 0 0 3px rgba(59, 130, 246, 0.18); }
  .tp-bar {
    margin-top: 5px;
    height: 3px;
    border-radius: 2px;
    background: var(--seg-track);
    overflow: hidden;
  }
  .tp-bar > i { display: block; height: 100%; background: var(--accent); border-radius: 2px; transition: width 0.3s var(--ease); }
  .tp-foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    min-height: 44px;
    padding: 5px 8px 7px 8px;
    border-top: 1px solid var(--line-2);
    flex-shrink: 0;
  }
  .tp-txt {
    border: 0;
    background: transparent;
    color: var(--ctl-fg);
    font: inherit;
    font-size: 12px;
    padding: 8px 10px;
    border-radius: 6px;
    cursor: pointer;
    transition: background 0.15s var(--ease), color 0.15s var(--ease);
  }
  .tp-txt:hover { background: var(--pill-hover); color: var(--fg); }
</style>
