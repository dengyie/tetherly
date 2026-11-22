<script lang="ts">
  import type { DevicesState } from "../lib/stores/devices";
  import type { SystemState } from "../lib/stores/system";
  import type { InputState } from "../lib/stores/input";
  import { cursorAway, isDriven, isDriving, peerName } from "../lib/stores/input";
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
  // §229: the card follows the physical cursor (`cursor_here`), not
  // `channel_open`/`seat`, which are node-centric and read inverted on the
  // target side. 收回 is a controller-only affordance.
  const away = $derived(cursorAway(input.input));
  const driving = $derived(isDriving(input.input));
  const driven = $derived(isDriven(input.input));
  const peer = $derived(peerName(input.input, devices.byId));
  const seatLabel = $derived.by(() => {
    if (!input.input) return "…";
    if (!away) return "本机";
    return peer ? `对端 ${peer}` : "对端";
  });
  const seatSub = $derived.by(() => {
    if (away) return driving ? "键鼠已跨到其他设备，可随时收回。" : "键鼠在对端本机，对端推到屏幕边缘即可跨到这里。";
    if (driven) return `${peer ?? "对端"} 正在控制本机。`;
    return "键鼠当前归本机，推到屏幕边缘即可跨屏。";
  });
  const canRelease = $derived(away && driving);

  let pin = $state("");
  let pinBusy = $state(false);
  async function runPin(op: () => Promise<string>) {
    pinBusy = true;
    await withToast(async () => {
      pin = await op();
    });
    pinBusy = false;
  }
  const pinActive = $derived(pairing.pairing?.pin_active === true);
  // `GET /pairing/pin` is 404 while no PIN is active (§223): with nothing to
  // show, the primary action generates one instead of surfacing that error.
  const showPin = () => runPin(async () => (pinActive ? await api.pin() : await api.rotatePin()).pin);
  const rotatePin = () => runPin(async () => (await api.rotatePin()).pin);
  const revokePin = () => runPin(async () => {
    await api.revokePin();
    return "";
  });
  // Drop the displayed digits once the daemon reports the window closed
  // (expiry or a revoke from elsewhere); never on the inactive→active edge,
  // which races the pairing.changed event right after generation.
  let wasActive = false;
  $effect(() => {
    if (wasActive && !pinActive) pin = "";
    wasActive = pinActive;
  });
  const releaseInput = () => withToast(() => api.releaseInput(), "已收回键鼠");
</script>

<PageHeader subtitle="本机状态、输入归属与配对入口" />

<div class="grid grid-cols-2 gap-3 sm:grid-cols-4">
  <div class="list row stat"><span class="k">在线设备</span><AnimatedCounter target={online.length} class="v" /></div>
  <div class="list row stat"><span class="k">已配对</span><AnimatedCounter target={trusted.length} class="v" /></div>
  <div class="list row stat">
    <span class="k">输入归属</span>
    <span class="v truncate {away || driven ? 'text-amber-600 dark:text-amber-400' : ''}" style="font-size: 16px">{seatLabel}</span>
  </div>
  <div class="list row stat">
    <span class="k">运行时长</span>
    <span class="v truncate" style="font-size: 16px">{system.system ? uptime(system.system.uptime_secs) : "…"}</span>
  </div>
</div>

<div class="grid gap-x-4 lg:grid-cols-2">
  <div>
    <div class="list-title">本机</div>
    <div class="list">
      {#if system.system}
        <div class="row compact"><div class="main name">名称</div><span class="val">{system.system.name}</span></div>
        <div class="row compact"><div class="main name">版本</div><span class="val">{system.system.version} <span class="dim">· api {system.system.api}</span></span></div>
        <div class="row compact"><div class="main name">平台</div><span class="val">{system.system.platform}</span></div>
        <div class="row compact"><div class="main name">对端</div><span class="val">{system.system.peers.live} 在线 · {system.system.peers.trusted} 已信任</span></div>
        <div class="row compact"><div class="main name">设备 ID</div><span class="val mono break-all text-[11px]">{system.system.device_id}</span></div>
      {:else}
        <div class="empty">等待快照…</div>
      {/if}
    </div>
  </div>

  <div>
    <div class="list-title">输入归属</div>
    <div class="list">
      <div class="row">
        <span class="ic"><Icon name="keyboard" size={16} /></span>
        <div class="main">
          <div class="name">{seatLabel}</div>
          <div class="sub">{seatSub}</div>
        </div>
        {#if canRelease}
          <button type="button" class="btn btn-sm" onclick={releaseInput}><Icon name="arrow-left" size={13} />收回</button>
        {/if}
      </div>
    </div>

    <div class="list-title">
      配对
      <span class="acts">
        {#if pairing.pairing?.pin_active}
          <StatusIndicator status="online" label="PIN 接受窗口开启中" />
          <span class="dim mono text-[11.5px]">{pairing.pairing.pin_expires_in_secs}s</span>
        {:else}
          <StatusIndicator status="offline" label="PIN 未激活" />
        {/if}
      </span>
    </div>
    <div class="list">
      {#if pin}
        <div class="row justify-center py-4">
          <div class="text-center">
            <div class="pin">{pin}</div>
            <div class="sub mt-1">在另一台设备上输入此 PIN 完成配对</div>
          </div>
        </div>
      {/if}
      <div class="row">
        <span class="ic"><Icon name="key" size={16} /></span>
        <div class="main">
          <div class="name">配对 PIN</div>
          <div class="sub">在另一台设备上输入 PIN 即可信任本机。</div>
        </div>
        <div class="acts">
          <button type="button" class="btn btn-sm btn-primary" disabled={pinBusy} onclick={showPin}><Icon name={pinActive ? "eye" : "key"} size={13} />{pinActive ? "显示 PIN" : "生成 PIN"}</button>
          <button type="button" class="btn btn-sm" disabled={pinBusy || !pinActive} onclick={rotatePin}>轮换</button>
          <button type="button" class="btn btn-sm btn-text btn-danger" disabled={pinBusy || !pinActive} onclick={revokePin}>撤销</button>
        </div>
      </div>
    </div>
  </div>
</div>

<div class="list-title">在线设备（{online.length}）</div>
<div class="list">
  {#if online.length === 0}
    <EmptyState compact icon="monitor" title="没有在线设备" description="已配对设备上线后会出现在这里。" />
  {:else}
    {#each online as d (d.id)}
      <div class="row hov">
        <span class="dot" data-state="online"></span>
        <div class="main">
          <div class="name">{d.name}</div>
          <div class="sub">{d.platform ?? "?"}{d.path ? ` · ${d.path}` : ""}</div>
        </div>
        <span class="val mono">{d.rtt_ms ?? "—"} ms</span>
      </div>
    {/each}
  {/if}
</div>
