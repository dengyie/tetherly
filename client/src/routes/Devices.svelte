<script lang="ts">
  import type { Device } from "../lib/api/gen/api";
  import type { Api } from "../lib/api/client";

  let { list, api }: { list: Device[]; api: Api } = $props();
  const trusted = $derived(list.filter((d) => d.trusted && !d.blocked));
  const blocked = $derived(list.filter((d) => d.blocked));

  let busyId: string | null = $state(null);
  let notice = $state("");

  async function run(id: string, op: () => Promise<unknown>, ok: string) {
    busyId = id;
    notice = "";
    try {
      await op();
      notice = ok;
    } catch (e) {
      notice = e instanceof Error ? e.message : String(e);
    } finally {
      busyId = null;
    }
  }
  const connect = (d: Device) =>
    run(d.id, () => api.connectDevice(d.id), `正在连接 ${d.name}…`);
  const block = (d: Device) =>
    run(d.id, () => api.blockDevice(d.id), `已屏蔽 ${d.name}`);
  const unblock = (d: Device) =>
    run(d.id, () => api.unblockDevice(d.id), `已解除屏蔽 ${d.name}`);
  const forget = (d: Device) =>
    run(d.id, () => api.forgetDevice(d.id), `已忘记 ${d.name}`);
</script>

<h1>设备</h1>
{#if notice}<p class="notice">{notice}</p>{/if}
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
          <span class="actions">
            {#if !d.online}
              <button disabled={busyId === d.id} onclick={() => connect(d)}>连接</button>
            {/if}
            <button disabled={busyId === d.id} onclick={() => block(d)}>屏蔽</button>
            <button class="danger" disabled={busyId === d.id} onclick={() => forget(d)}>忘记</button>
          </span>
        </li>
      {/each}
    </ul>
  {/if}
</section>
{#if blocked.length > 0}
  <section class="card">
    <h2>已屏蔽（{blocked.length}）</h2>
    <ul>
      {#each blocked as d (d.id)}
        <li>
          <span class="dot off"></span>
          <strong>{d.name}</strong>
          <span class="actions">
            <button disabled={busyId === d.id} onclick={() => unblock(d)}>解除屏蔽</button>
          </span>
        </li>
      {/each}
    </ul>
  </section>
{/if}

<style>
  .card { background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius); padding: var(--pad); margin-bottom: 12px; }
  h2 { margin: 0 0 8px; font-size: 15px; }
  .dim { color: var(--text-dim); }
  .notice { color: var(--text-dim); font-size: 13px; }
  ul { list-style: none; margin: 0; padding: 0; }
  li { padding: 6px 0; display: flex; gap: 8px; align-items: baseline; flex-wrap: wrap; }
  .dot { width: 8px; height: 8px; border-radius: 50%; flex: none; align-self: center; }
  .ok { background: var(--ok); }
  .off { background: var(--offline); }
  .caps { color: var(--text-dim); font-size: 12px; }
  .actions { margin-left: auto; display: flex; gap: 6px; }
  button {
    border: 1px solid var(--border); background: var(--bg); color: var(--text);
    padding: 3px 10px; border-radius: 6px; cursor: pointer; font: inherit; font-size: 13px;
  }
  button:disabled { opacity: 0.5; cursor: default; }
  button.danger { color: var(--danger, #c0392b); }
</style>
