<script lang="ts">
  import { onMount } from "svelte";
  import type { Api } from "../lib/api/client";
  import type { Diagnostics, LogTail } from "../lib/api/gen/api";
  import { withToast } from "../lib/ui/toast";
  import Icon from "../lib/ui/Icon.svelte";
  import PageHeader from "../lib/ui/PageHeader.svelte";
  import EmptyState from "../lib/ui/EmptyState.svelte";

  let { api }: { api: Api } = $props();

  let diag: Diagnostics | null = $state(null);
  let logs: LogTail | null = $state(null);
  let filterInput = $state("");
  let busy = $state(false);

  async function refresh() {
    busy = true;
    await withToast(async () => {
      [diag, logs] = await Promise.all([api.diagnostics(), api.logs(200)]);
    });
    busy = false;
  }
  async function applyFilter() {
    busy = true;
    const ok = await withToast(() => api.setLogLevel({ filter: filterInput }), "日志级别已应用");
    busy = false;
    if (ok) await refresh();
  }
  async function resetFilter() {
    busy = true;
    const ok = await withToast(() => api.setLogLevel({ reset: true }), "已恢复默认日志级别");
    busy = false;
    if (ok) {
      filterInput = "";
      await refresh();
    }
  }
  onMount(() => {
    void refresh();
  });
</script>

<PageHeader subtitle="事件流、旧路由迁移监控与日志">
  <button type="button" class="icon" aria-label="刷新" title="刷新" disabled={busy} onclick={refresh}><Icon name="refresh" size={14} class={busy ? "animate-spin" : ""} /></button>
</PageHeader>

<div class="grid gap-x-4 lg:grid-cols-2">
  <div>
    <div class="list-title">事件流</div>
    <div class="list">
      {#if diag}
        <div class="row compact"><div class="main name">当前 seq</div><span class="val mono">{diag.event_seq}</span></div>
        <div class="row compact"><div class="main name">旧路由命中 <span class="dim font-normal">迁移监控</span></div><span class="val mono">{diag.legacy_routes.length}</span></div>
        {#if diag.legacy_routes.length > 0}
          <div class="row-group border-t border-[var(--line-2)]">
            <table class="w-full border-collapse text-[11.5px]">
              <thead><tr class="dim text-left"><th class="px-3.5 py-1.5 font-medium">方法</th><th class="py-1.5 pr-2 font-medium">路由</th><th class="py-1.5 pr-2 font-medium">调用方</th><th class="py-1.5 pr-3 font-medium">次数</th></tr></thead>
              <tbody>
                {#each diag.legacy_routes as h}
                  <tr class="border-t border-[var(--line-2)]"><td class="mono px-3.5 py-1.5">{h.method}</td><td class="mono py-1.5 pr-2">{h.route}</td><td class="py-1.5 pr-2">{h.caller}</td><td class="mono py-1.5 pr-3">{h.count}</td></tr>
                {/each}
              </tbody>
            </table>
          </div>
        {/if}
      {:else}
        <div class="empty">{busy ? "加载中…" : "等待数据…"}</div>
      {/if}
    </div>
  </div>

  <div>
    <div class="list-title">日志级别</div>
    <div class="list">
      <form class="row flex-wrap" onsubmit={(e) => { e.preventDefault(); void applyFilter(); }}>
        <input class="input mono min-w-0 flex-1" placeholder="例如 info,tetherly_node=debug" bind:value={filterInput} aria-label="日志过滤器" />
        <button type="submit" class="btn btn-sm btn-primary" disabled={busy || !filterInput.trim()}>应用</button>
        <button type="button" class="btn btn-sm" disabled={busy} onclick={resetFilter}>恢复默认</button>
      </form>
      <div class="note">tracing EnvFilter 语法；留空则沿用启动时的级别。</div>
    </div>
  </div>
</div>

<div class="list-title">日志尾部 <span class="acts dim">{logs?.lines.length ?? 0} 行</span></div>
<div class="list">
  {#if logs && logs.lines.length > 0}
    <pre class="log">{logs.lines.join("\n")}</pre>
  {:else}
    <EmptyState compact icon="code" title={busy ? "加载中…" : "暂无日志"} />
  {/if}
</div>
