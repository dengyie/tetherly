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

<PageHeader title="诊断" subtitle="事件流、旧路由迁移监控与日志" icon="terminal">
  <button type="button" class="btn btn-sm" disabled={busy} onclick={refresh}><Icon name="refresh" size={13} class={busy ? "animate-spin" : ""} />刷新</button>
</PageHeader>

<div class="grid gap-4 lg:grid-cols-2">
  <section class="card">
    <h2 class="card-title"><Icon name="activity" size={15} class="text-zinc-400" />事件流</h2>
    {#if diag}
      <p class="m-0 text-[13px]">当前 seq：<span class="mono">{diag.event_seq}</span></p>
      <h3 class="mb-1.5 mt-3 text-xs font-semibold dim">旧路由命中（迁移监控）</h3>
      {#if diag.legacy_routes.length === 0}
        <p class="m-0 text-xs dim">没有旧 /api/* 命中</p>
      {:else}
        <table class="w-full border-collapse text-xs">
          <thead><tr class="text-left dim"><th class="py-1 pr-2 font-medium">方法</th><th class="py-1 pr-2 font-medium">路由</th><th class="py-1 pr-2 font-medium">调用方</th><th class="py-1 font-medium">次数</th></tr></thead>
          <tbody>
            {#each diag.legacy_routes as h}
              <tr class="border-t border-zinc-100 dark:border-zinc-800"><td class="mono py-1 pr-2">{h.method}</td><td class="mono py-1 pr-2">{h.route}</td><td class="py-1 pr-2">{h.caller}</td><td class="mono py-1">{h.count}</td></tr>
            {/each}
          </tbody>
        </table>
      {/if}
    {:else}
      <p class="m-0 dim">{busy ? "加载中…" : "等待数据…"}</p>
    {/if}
  </section>

  <section class="card">
    <h2 class="card-title"><Icon name="sliders" size={15} class="text-zinc-400" />日志级别</h2>
    <form class="flex gap-2" onsubmit={(e) => { e.preventDefault(); void applyFilter(); }}>
      <input class="input flex-1 mono" placeholder="例如 info,tetherly_node=debug" bind:value={filterInput} aria-label="日志过滤器" />
      <button type="submit" class="btn btn-primary" disabled={busy || !filterInput.trim()}>应用</button>
      <button type="button" class="btn" disabled={busy} onclick={resetFilter}>恢复默认</button>
    </form>
    <p class="m-0 mt-2 text-xs dim">tracing EnvFilter 语法；留空则沿用启动时的级别。</p>
  </section>
</div>

<section class="card">
  <h2 class="card-title"><Icon name="code" size={15} class="text-zinc-400" />日志尾部</h2>
  {#if logs && logs.lines.length > 0}
    <pre class="m-0 max-h-96 overflow-auto rounded-xl bg-zinc-950 p-3 text-[11px] leading-relaxed text-zinc-200 mono">{logs.lines.join("\n")}</pre>
  {:else}
    <EmptyState compact icon="code" title={busy ? "加载中…" : "暂无日志"} />
  {/if}
</section>
