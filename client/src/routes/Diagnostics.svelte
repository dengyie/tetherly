<script lang="ts">
  import { onMount } from "svelte";
  import type { Api } from "../lib/api/client";
  import type { Diagnostics, LogTail } from "../lib/api/gen/api";

  let { api }: { api: Api } = $props();

  let diag: Diagnostics | null = $state(null);
  let logs: LogTail | null = $state(null);
  let err = $state("");
  let filterInput = $state("");
  let busy = $state(false);

  async function refresh() {
    busy = true;
    err = "";
    try {
      [diag, logs] = await Promise.all([api.diagnostics(), api.logs(200)]);
    } catch (e) {
      err = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }

  async function applyFilter() {
    busy = true;
    err = "";
    try {
      await api.setLogLevel({ filter: filterInput });
      await refresh();
    } catch (e) {
      err = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }

  async function resetFilter() {
    busy = true;
    err = "";
    try {
      await api.setLogLevel({ reset: true });
      filterInput = "";
      await refresh();
    } catch (e) {
      err = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }

  onMount(refresh);
</script>

<h1>诊断</h1>
{#if err}<p class="err">{err}</p>{/if}
<section class="card">
  <h2>事件流</h2>
  {#if diag}
    <p>当前 seq：<span class="mono">{diag.event_seq}</span></p>
    <h3>旧路由命中（迁移监控）</h3>
    {#if diag.legacy_routes.length === 0}
      <p class="dim">没有旧 /api/* 命中</p>
    {:else}
      <table>
        <thead><tr><th>方法</th><th>路由</th><th>调用方</th><th>次数</th></tr></thead>
        <tbody>
          {#each diag.legacy_routes as h}
            <tr><td class="mono">{h.method}</td><td class="mono">{h.route}</td><td>{h.caller}</td><td>{h.count}</td></tr>
          {/each}
        </tbody>
      </table>
    {/if}
  {:else}
    <p class="dim">{busy ? "加载中…" : "等待数据…"}</p>
  {/if}
</section>
<section class="card">
  <h2>日志级别</h2>
  <form onsubmit={(e) => { e.preventDefault(); void applyFilter(); }}>
    <input placeholder="例如 info,tetherly_node=debug" bind:value={filterInput} />
    <button disabled={busy || !filterInput.trim()}>应用</button>
    <button type="button" disabled={busy} onclick={resetFilter}>恢复默认</button>
  </form>
</section>
<section class="card">
  <h2>
    日志尾部
    <button class="mini" disabled={busy} onclick={refresh}>刷新</button>
  </h2>
  {#if logs}
    <pre>{logs.lines.join("\n")}</pre>
  {:else}
    <p class="dim">{busy ? "加载中…" : "暂无日志"}</p>
  {/if}
</section>

<style>
  .card { background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius); padding: var(--pad); margin-bottom: 12px; }
  h2 { margin: 0 0 8px; font-size: 15px; display: flex; align-items: center; gap: 8px; }
  h3 { font-size: 13px; margin: 12px 0 6px; color: var(--text-dim); }
  .dim { color: var(--text-dim); }
  .err { color: #c0392b; font-size: 13px; }
  .mono { font-family: ui-monospace, monospace; font-size: 12px; }
  table { width: 100%; border-collapse: collapse; font-size: 13px; }
  th, td { text-align: left; padding: 4px 8px; border-bottom: 1px solid var(--border); }
  form { display: flex; gap: 8px; }
  input { flex: 1; font: inherit; padding: 6px 10px; border: 1px solid var(--border); border-radius: 6px; background: var(--bg); color: var(--text); }
  button { border: 1px solid var(--border); background: var(--bg); color: var(--text); padding: 4px 12px; border-radius: 6px; cursor: pointer; font: inherit; }
  button:disabled { opacity: 0.5; cursor: default; }
  .mini { font-size: 12px; padding: 2px 8px; }
  pre { background: var(--bg); border: 1px solid var(--border); border-radius: 6px; padding: 10px; font-size: 11px; line-height: 1.5; max-height: 320px; overflow: auto; margin: 0; }
</style>
