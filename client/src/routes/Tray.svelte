<script lang="ts">
  // Tray panel compact view (§6.3): the same SPA mounts this route in the
  // 360×480 frameless tray window. No navigation — one screen: seat status
  // line, trusted-device list, in-flight transfers, pause/open actions.
  import type { DevicesState } from "../lib/stores/devices";
  import type { InputState } from "../lib/stores/input";
  import type { TransfersState } from "../lib/stores/transfers";
  import type { LinkState } from "../lib/transport/transport";
  import type { SettingsState } from "../lib/stores/settings";
  import type { Api } from "../lib/api/client";
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
  const stateTone = $derived(
    link !== "live" ? "bg-rose-500" : paused ? "bg-amber-500" : input.input?.seat === "remote" ? "bg-sky-500" : "bg-emerald-500",
  );

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

<div class="flex h-full flex-col bg-white/80 dark:bg-[#16181d]/85 backdrop-blur-xl text-zinc-800 dark:text-zinc-100" data-testid="tray-panel">
  <div class="flex items-center gap-2.5 px-4 pt-3.5 pb-2.5 border-b border-zinc-200/70 dark:border-zinc-800">
    <img src={brandLogo} alt="" width="22" height="22" class="h-[22px] w-[22px] rounded-md" draggable="false" />
    <span class="text-sm font-semibold tracking-tight">{brandName}</span>
    <span class="ml-auto inline-flex items-center gap-1.5 text-xs text-zinc-500 dark:text-zinc-400">
      <span class="inline-flex h-2 w-2 rounded-full {stateTone}"></span>
      <span data-testid="tray-state">{stateLine}</span>
    </span>
  </div>

  <div class="flex-1 overflow-y-auto px-4 py-3">
    <div class="mb-1.5 flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-wider text-zinc-400"><Icon name="monitor" size={12} />设备</div>
    {#if online.length === 0}
      <div class="rounded-xl bg-zinc-100/80 dark:bg-zinc-800/60 px-3 py-2.5 text-xs text-zinc-500 dark:text-zinc-400">没有在线设备</div>
    {:else}
      <ul class="m-0 list-none p-0 flex flex-col gap-1">
        {#each online as d (d.id)}
          <li class="flex items-center gap-2 rounded-xl bg-zinc-100/80 dark:bg-zinc-800/60 px-3 py-2 text-[13px]" data-testid="tray-device">
            <span class="inline-flex h-2 w-2 rounded-full bg-emerald-500"></span>
            <span class="truncate font-medium">{d.name}</span>
            <span class="ml-auto mono text-[11px] text-zinc-400">{d.rtt_ms != null ? `${d.rtt_ms}ms` : ""}</span>
          </li>
        {/each}
      </ul>
    {/if}

    <div class="mb-1.5 mt-4 flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-wider text-zinc-400"><Icon name="send" size={12} />传输</div>
    {#if activeTransfers.length === 0}
      <div class="rounded-xl bg-zinc-100/80 dark:bg-zinc-800/60 px-3 py-2.5 text-xs text-zinc-500 dark:text-zinc-400">没有进行中的传输</div>
    {:else}
      <ul class="m-0 list-none p-0 flex flex-col gap-1">
        {#each activeTransfers as t (t.transfer_id)}
          <li class="rounded-xl bg-zinc-100/80 dark:bg-zinc-800/60 px-3 py-2 text-[13px]" data-testid="tray-transfer">
            <div class="flex items-center gap-2">
              <Icon name={t.direction === "inbound" ? "download" : "upload"} size={13} class="text-zinc-400" />
              <span class="truncate font-medium">{t.filename}</span>
              <span class="ml-auto mono text-[11px] text-zinc-500">{Math.floor((t.transferred / t.total) * 100)}%</span>
            </div>
            <div class="mt-1.5 h-1 w-full overflow-hidden rounded-full bg-zinc-200 dark:bg-zinc-700">
              <div class="h-full rounded-full bg-indigo-500 transition-all" style="width: {Math.floor((t.transferred / t.total) * 100)}%"></div>
            </div>
          </li>
        {/each}
      </ul>
    {/if}
  </div>

  <div class="grid grid-cols-2 gap-2 border-t border-zinc-200/70 dark:border-zinc-800 px-4 py-3">
    <button type="button" class="btn {paused ? 'btn-primary' : ''}" onclick={toggleShare} data-testid="tray-toggle">
      <Icon name={paused ? "play" : "pause"} size={14} />{paused ? "恢复共享" : "暂停共享"}</button>
    <button type="button" class="btn" onclick={() => emitter.send("tray-action", { action: "open-main" })} data-testid="tray-open">
      <Icon name="external-link" size={14} />打开主窗口</button>
  </div>
</div>
