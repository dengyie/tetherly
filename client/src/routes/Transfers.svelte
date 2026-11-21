<script lang="ts">
  import type { Transfers } from "../lib/api/gen/api";
  import type { Api } from "../lib/api/client";
  import { bytes } from "../lib/fmt";
  import { withToast } from "../lib/ui/toast";
  import Icon from "../lib/ui/Icon.svelte";
  import PageHeader from "../lib/ui/PageHeader.svelte";
  import EmptyState from "../lib/ui/EmptyState.svelte";
  import ProgressBar from "../lib/ui/ProgressBar.svelte";

  let { data, api }: { data: Transfers | null; api: Api } = $props();

  const phaseLabel: Record<string, string> = {
    transferring: "传输中",
    verifying: "校验中",
    completed: "已完成",
    failed: "失败",
  };
  const dirLabel: Record<string, string> = { inbound: "接收", outbound: "发送" };
  function pctNum(t: { total: number; transferred: number }): number | null {
    if (t.total === 0) return null;
    return Math.min(100, Math.round((t.transferred / t.total) * 100));
  }
  const isActive = (p: string) => p === "transferring" || p === "verifying";
  const barStatus = (p: string) => (p === "failed" ? "error" : p === "completed" ? "done" : "streaming");

  let busyId: string | null = $state(null);
  async function run(id: string, op: () => Promise<unknown>, ok: string) {
    busyId = id;
    await withToast(op, ok);
    busyId = null;
  }
  const cancel = (t: { transfer_id: string; filename: string }) =>
    run(t.transfer_id, () => api.cancelTransfer(t.transfer_id), `已取消 ${t.filename}`);
  const clearAll = () => run("__all__", () => api.clearTransfers(), "已清空历史记录");
</script>

<PageHeader title="传输" subtitle="设备间文件传输队列" icon="send">
  {#if (data?.terminal_count ?? 0) > 0}
    <button type="button" class="btn btn-sm" disabled={busyId === "__all__"} onclick={clearAll}><Icon name="trash" size={13} />清空历史</button>
  {/if}
</PageHeader>

{#if data}
  <div class="grid grid-cols-2 gap-3 sm:grid-cols-4">
    <div class="stat-tile"><span class="text-[11px] dim">活跃</span><span class="mono text-xl font-semibold">{data.active_count}</span></div>
    <div class="stat-tile"><span class="text-[11px] dim">历史</span><span class="mono text-xl font-semibold">{data.terminal_count}</span></div>
    <div class="stat-tile"><span class="text-[11px] dim">已发送</span><span class="mono text-xl font-semibold">{bytes(data.stats.bytes_sent)}</span></div>
    <div class="stat-tile"><span class="text-[11px] dim">已接收</span><span class="mono text-xl font-semibold">{bytes(data.stats.bytes_received)}</span></div>
  </div>
{/if}

<section class="card">
  <h2 class="card-title"><Icon name="layers" size={15} class="text-zinc-400" />传输队列（活跃 {data?.active_count ?? 0} · 历史 {data?.terminal_count ?? 0}）</h2>
  {#if !data || data.transfers.length === 0}
    <EmptyState icon="send" title="暂无传输记录" description="拖拽文件到设备，或从其他设备发送，进度会显示在这里。" />
  {:else}
    <ul class="m-0 list-none p-0">
      {#each data.transfers as t (t.transfer_id)}
        <li class="row items-start">
          <span class="mt-0.5 inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-lg {t.direction === 'inbound' ? 'bg-sky-500/10 text-sky-600' : 'bg-violet-500/10 text-violet-600'}">
            <Icon name={t.direction === "inbound" ? "download" : "upload"} size={15} />
          </span>
          <div class="min-w-0 flex-1">
            <div class="flex items-center gap-2">
              <span class="badge">{dirLabel[t.direction] ?? t.direction}</span>
              <span class="truncate font-medium">{t.filename}</span>
              <span class="ml-auto shrink-0 text-xs font-medium {t.phase === 'failed' ? 'text-rose-600' : t.phase === 'completed' ? 'text-emerald-600' : 'text-indigo-600'}">{phaseLabel[t.phase] ?? t.phase}</span>
            </div>
            <div class="mt-1.5">
              <ProgressBar value={pctNum(t)} status={barStatus(t.phase)} label="{bytes(t.transferred)} / {bytes(t.total)} · {bytes(t.speed_bps)}/s" />
            </div>
            {#if t.error}<div class="mt-1 text-xs text-rose-600">{t.error}</div>{/if}
          </div>
          {#if isActive(t.phase)}
            <button type="button" class="btn btn-sm" disabled={busyId === t.transfer_id} onclick={() => cancel(t)}>取消</button>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</section>
