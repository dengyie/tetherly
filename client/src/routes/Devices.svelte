<script lang="ts">
  import type { DevicesState } from "../lib/stores/devices";

  let { devices }: { devices: DevicesState } = $props();
  const all = $derived([...devices.byId.values()]);
  const trusted = $derived(all.filter((d) => d.trusted && !d.blocked));
  const blocked = $derived(all.filter((d) => d.blocked));
</script>

<h1>设备</h1>
<section class="card">
  <h2>已配对（{trusted.length}）</h2>
  {#if trusted.length === 0}
    <p class="dim">还没有已配对设备。在另一台设备上拨号即可配对。</p>
  {:else}
    <ul>
      {#each trusted as d (d.id)}
        <li>
          <span class="dot" class:ok={d.online} class:off={!d.online}></span>
          <strong>{d.name}</strong>
          <span class="dim">{d.platform ?? "?"} · {d.online ? `${d.rtt_ms ?? "—"} ms · ${d.path ?? ""}` : "离线"}</span>
          <span class="caps">{d.caps.join(" · ")}</span>
        </li>
      {/each}
    </ul>
  {/if}
</section>
{#if blocked.length > 0}
  <section class="card">
    <h2>已屏蔽（{blocked.length}）</h2>
    <ul><li class="dim">{blocked.map((d) => d.name).join("、")}</li></ul>
  </section>
{/if}

<style>
  .card { background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius); padding: var(--pad); margin-bottom: 12px; }
  h2 { margin: 0 0 8px; font-size: 15px; }
  .dim { color: var(--text-dim); }
  ul { list-style: none; margin: 0; padding: 0; }
  li { padding: 6px 0; display: flex; gap: 8px; align-items: baseline; flex-wrap: wrap; }
  .dot { width: 8px; height: 8px; border-radius: 50%; flex: none; align-self: center; }
  .ok { background: var(--ok); }
  .off { background: var(--offline); }
  .caps { margin-left: auto; color: var(--text-dim); font-size: 12px; }
</style>
