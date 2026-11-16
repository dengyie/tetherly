<script lang="ts">
  // Tray panel compact view (§6.3): the same SPA mounts this route in the
  // 360×480 frameless tray window. No navigation — one screen: seat status
  // line, trusted-device list, in-flight transfers, pause/open actions.
  import type { DevicesState } from "../lib/stores/devices";
  import type { InputState } from "../lib/stores/input";
  import type { SystemState } from "../lib/stores/system";
  import type { TransfersState } from "../lib/stores/transfers";
  import type { LinkState } from "../lib/transport/transport";
  import type { SettingsState } from "../lib/stores/settings";
  import type { Api } from "../lib/api/client";
  import { emitter } from "../lib/transport/traybridge";

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

  // Same wording contract as the Rust tray fold (single source per meaning).
  const stateLine = $derived.by(() => {
    if (link !== "live") return link === "offline" ? "daemon 未连接" : "重新同步…";
    const i = input.input;
    if (!i) return "…";
    if (i.seat === "remote") return i.peer ? `键鼠在 ${i.peer}` : "键鼠被远端接管";
    if (settings.settings && !settings.settings.general.advertise) return "共享已暂停";
    return "键鼠在本机";
  });

  async function toggleShare() {
    if (!api) return;
    const on = settings.settings?.general.advertise ?? true;
    try {
      await api.patchSettings({ advertise: !on });
      emitter.send("tray-action", { action: "toggled", value: !on });
    } catch (e) {
      emitter.send("tray-action", { action: "error", message: String(e) });
    }
  }
</script>

<div class="panel" data-testid="tray-panel">
  <div class="state" data-testid="tray-state">{stateLine}</div>

  <div class="section">设备</div>
  {#if online.length === 0}
    <div class="empty">没有在线设备</div>
  {:else}
    <ul class="devices">
      {#each online as d (d.id)}
        <li data-testid="tray-device">
          <span class="name">{d.name}</span>
          <span class="rtt">{d.rtt_ms != null ? `${d.rtt_ms}ms` : ""}</span>
        </li>
      {/each}
    </ul>
  {/if}

  <div class="section">传输</div>
  {#if activeTransfers.length === 0}
    <div class="empty">没有进行中的传输</div>
  {:else}
    <ul class="transfers">
      {#each activeTransfers as t (t.transfer_id)}
        <li data-testid="tray-transfer">
          <span class="name">{t.filename}</span>
          <span class="pct">{Math.floor((t.transferred / t.total) * 100)}%</span>
        </li>
      {/each}
    </ul>
  {/if}

  <div class="actions">
    <button onclick={toggleShare} data-testid="tray-toggle">
      {settings.settings && !settings.settings.general.advertise ? "恢复共享" : "暂停共享"}
    </button>
    <button onclick={() => emitter.send("tray-action", { action: "open-main" })} data-testid="tray-open">
      打开主窗口
    </button>
  </div>
</div>

<style>
  .panel { display: flex; flex-direction: column; gap: 8px; padding: 12px; font-size: 13px; }
  .state { font-weight: 600; }
  .section { color: var(--text-dim); font-size: 11px; text-transform: uppercase; margin-top: 4px; }
  ul { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
  li { display: flex; justify-content: space-between; }
  .rtt, .pct { color: var(--text-dim); }
  .empty { color: var(--text-dim); }
  .actions { display: flex; gap: 8px; margin-top: auto; }
  button {
    border: 1px solid var(--border); background: var(--surface); color: var(--text);
    padding: 6px 12px; border-radius: 8px; cursor: pointer; font: inherit;
  }
</style>
