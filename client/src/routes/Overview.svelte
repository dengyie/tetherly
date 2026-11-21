<script lang="ts">
  import type { DevicesState } from "../lib/stores/devices";
  import type { SystemState } from "../lib/stores/system";
  import type { InputState } from "../lib/stores/input";
  import type { PairingState } from "../lib/stores/pairing";
  import type { Api } from "../lib/api/client";
  import { uptime } from "../lib/fmt";
  import { withToast } from "../lib/ui/toast";
  import Icon from "../lib/ui/Icon.svelte";
  import PageHeader from "../lib/ui/PageHeader.svelte";
  import EmptyState from "../lib/ui/EmptyState.svelte";
  import AnimatedCounter from "../lib/ui/AnimatedCounter.svelte";
  import StatusIndicator from "../lib/ui/StatusIndicator.svelte";

  let {
    devices,
    system,
    input,
    pairing,
    api,
  }: {
    devices: DevicesState;
    system: SystemState;
    input: InputState;
    pairing: PairingState;
    api: Api;
  } = $props();
  const all = $derived([...devices.byId.values()]);
  const online = $derived(all.filter((d) => d.online && d.trusted));
  const trusted = $derived(all.filter((d) => d.trusted && !d.blocked));
  const seatLabel = $derived.by(() => {
    const i = input.input;
    if (!i) return "…";
    if (!i.channel_open) return "本机";
    return i.peer ? `对端 ${i.peer}` : i.seat;
  });
  const seatRemote = $derived(!!input.input?.channel_open);

  let pin = $state("");
  let pinBusy = $state(false);
  async function runPin(op: () => Promise<string>) {
    pinBusy = true;
    await withToast(async () => {
      pin = await op();
    });
    pinBusy = false;
  }
  const showPin = () => runPin(async () => (await api.pin()).pin);
  const rotatePin = () => runPin(async () => (await api.rotatePin()).pin);
  const revokePin = () => runPin(async () => {
    await api.revokePin();
    return "";
  });
  const releaseInput = () => withToast(() => api.releaseInput(), "已收回键鼠");
</script>

<PageHeader title="概览" subtitle="本机状态、输入归属与配对入口" icon="home" />

<div class="grid grid-cols-2 gap-3 sm:grid-cols-4">
  <div class="stat-tile">
    <span class="text-[11px] dim">在线设备</span>
    <AnimatedCounter target={online.length} class="text-2xl font-semibold" />
  </div>
  <div class="stat-tile">
    <span class="text-[11px] dim">已配对</span>
    <AnimatedCounter target={trusted.length} class="text-2xl font-semibold" />
  </div>
  <div class="stat-tile">
    <span class="text-[11px] dim">输入归属</span>
    <span class="truncate text-base font-semibold {seatRemote ? 'text-amber-600 dark:text-amber-400' : ''}">{seatLabel}</span>
  </div>
  <div class="stat-tile">
    <span class="text-[11px] dim">运行时长</span>
    <span class="truncate text-base font-semibold">{system.system ? uptime(system.system.uptime_secs) : "…"}</span>
  </div>
</div>

<div class="grid gap-4 lg:grid-cols-2">
  <section class="card">
    <h2 class="card-title"><Icon name="cpu" size={15} class="text-zinc-400" />本机</h2>
    {#if system.system}
      <dl class="m-0 grid grid-cols-[88px_1fr] gap-y-1.5 text-[13px]">
        <dt class="dim">名称</dt><dd class="m-0 font-medium">{system.system.name}</dd>
        <dt class="dim">版本</dt><dd class="m-0">{system.system.version}（api {system.system.api}）</dd>
        <dt class="dim">平台</dt><dd class="m-0">{system.system.platform}</dd>
        <dt class="dim">对端</dt><dd class="m-0">{system.system.peers.live} 在线 · {system.system.peers.trusted} 已信任</dd>
        <dt class="dim">设备 ID</dt><dd class="m-0 mono break-all text-xs">{system.system.device_id}</dd>
      </dl>
    {:else}
      <p class="m-0 dim">等待快照…</p>
    {/if}
  </section>

  <section class="card">
    <h2 class="card-title"><Icon name="keyboard" size={15} class="text-zinc-400" />输入归属</h2>
    <div class="flex items-center justify-between gap-3">
      <div>
        <div class="text-sm font-medium">{seatLabel}</div>
        <p class="m-0 mt-0.5 text-xs dim">
          {seatRemote ? "键鼠已跨到其他设备，可随时收回。" : "键鼠当前归本机，推到屏幕边缘即可跨屏。"}
        </p>
      </div>
      {#if seatRemote}
        <button type="button" class="btn" onclick={releaseInput}><Icon name="arrow-left" size={14} />收回</button>
      {/if}
    </div>
    <h2 class="card-title mt-5"><Icon name="key" size={15} class="text-zinc-400" />配对</h2>
    <div class="flex items-center gap-2 text-xs">
      {#if pairing.pairing?.pin_active}
        <StatusIndicator status="online" label="PIN 接受窗口开启中" />
        <span class="dim">{pairing.pairing.pin_expires_in_secs}s 后关闭</span>
      {:else}
        <StatusIndicator status="offline" label="PIN 未激活" />
      {/if}
    </div>
    {#if pin}
      <div class="mt-3 rounded-xl border border-dashed border-indigo-300/70 dark:border-indigo-700/60 bg-indigo-50/60 dark:bg-indigo-950/30 px-4 py-3 text-center">
        <div class="mono text-2xl font-semibold tracking-[0.4em] text-indigo-700 dark:text-indigo-300">{pin}</div>
        <div class="mt-1 text-[11px] dim">在另一台设备上输入此 PIN 完成配对</div>
      </div>
    {/if}
    <div class="mt-3 flex flex-wrap gap-2">
      <button type="button" class="btn btn-primary" disabled={pinBusy} onclick={showPin}><Icon name="eye" size={14} />显示 PIN</button>
      <button type="button" class="btn" disabled={pinBusy} onclick={rotatePin}><Icon name="refresh" size={14} />轮换</button>
      <button type="button" class="btn btn-danger" disabled={pinBusy} onclick={revokePin}><Icon name="x" size={14} />撤销</button>
    </div>
  </section>
</div>

<section class="card">
  <h2 class="card-title"><Icon name="wifi" size={15} class="text-zinc-400" />在线设备（{online.length}）</h2>
  {#if online.length === 0}
    <EmptyState compact icon="monitor" title="没有在线设备" description="已配对设备上线后会出现在这里。" />
  {:else}
    <ul class="m-0 list-none p-0">
      {#each online as d (d.id)}
        <li class="row">
          <span class="relative flex h-2 w-2 shrink-0">
            <span class="absolute inline-flex h-full w-full animate-ping rounded-full bg-emerald-400 opacity-60"></span>
            <span class="relative inline-flex h-2 w-2 rounded-full bg-emerald-500"></span>
          </span>
          <span class="font-medium">{d.name}</span>
          <span class="badge">{d.platform ?? "?"}</span>
          <span class="ml-auto text-xs dim mono">{d.rtt_ms ?? "—"} ms · {d.path ?? ""}</span>
        </li>
      {/each}
    </ul>
  {/if}
</section>
