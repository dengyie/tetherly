<script lang="ts">
  import type { DevicesState } from "../lib/stores/devices";

  let { devices }: { devices: DevicesState } = $props();
  const all = $derived([...devices.byId.values()]);
  const online = $derived(all.filter((d) => d.online && d.trusted));
  const self = $derived(all.find((d) => d.id === "__self__")); // placeholder until model exposes self
</script>

<h1>概览</h1>
<section class="card">
  <h2>本机</h2>
  <p class="dim">版本、地址与链路信息（system store 接入后显示）</p>
</section>
<section class="card">
  <h2>输入归属</h2>
  <p class="dim">当前由谁控制键鼠（input store 接入后显示）</p>
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
  ul { list-style: none; margin: 0; padding: 0; }
  li { padding: 4px 0; }
  .dot { display: inline-block; width: 8px; height: 8px; border-radius: 50%; margin-right: 8px; }
  .ok { background: var(--ok); }
</style>
