<script lang="ts">
  import type { DevicesState } from "../lib/stores/devices";
  import type { SystemState } from "../lib/stores/system";
  import type { InputState } from "../lib/stores/input";

  let {
    devices,
    system,
    input,
  }: { devices: DevicesState; system: SystemState; input: InputState } = $props();
  const online = $derived([...devices.byId.values()].filter((d) => d.online && d.trusted));
  const seatLabel = $derived.by(() => {
    const i = input.input;
    if (!i) return "…";
    if (!i.channel_open) return "本机";
    return i.peer ? `对端 ${i.peer}` : i.seat;
  });
</script>

<h1>概览</h1>
<section class="card">
  <h2>本机</h2>
  {#if system.system}
    <dl>
      <dt>名称</dt><dd>{system.system.name}</dd>
      <dt>版本</dt><dd>{system.system.version}（api {system.system.api}）</dd>
      <dt>平台</dt><dd>{system.system.platform}</dd>
      <dt>运行时长</dt><dd>{Math.floor(system.system.uptime_secs / 60)} 分钟</dd>
      <dt>设备 ID</dt><dd class="mono">{system.system.device_id}</dd>
    </dl>
  {:else}
    <p class="dim">等待快照…</p>
  {/if}
</section>
<section class="card">
  <h2>输入归属</h2>
  <p>{seatLabel}</p>
</section>
<section class="card">
  <h2>在线设备（{online.length}）</h2>
  {#if online.length === 0}
    <p class="dim">没有在线设备</p>
  {:else}
    <ul>
      {#each online as d (d.id)}
        <li><span class="dot ok"></span>{d.name} <span class="dim">{d.rtt_ms ?? "—"} ms · {d.path ?? ""}</span></li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .card { background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius); padding: var(--pad); margin-bottom: 12px; }
  h2 { margin: 0 0 8px; font-size: 15px; }
  .dim { color: var(--text-dim); }
  dl { display: grid; grid-template-columns: 88px 1fr; row-gap: 4px; margin: 0; }
  dt { color: var(--text-dim); }
  dd { margin: 0; }
  .mono { font-family: ui-monospace, monospace; font-size: 12px; }
  ul { list-style: none; margin: 0; padding: 0; }
  li { padding: 4px 0; }
  .dot { display: inline-block; width: 8px; height: 8px; border-radius: 50%; margin-right: 8px; }
  .ok { background: var(--ok); }
</style>
