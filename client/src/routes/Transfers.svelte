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
  const phaseBadge: Record<string, string> = { failed: "badge-err", completed: "badge-ok", transferring: "badge-accent", verifying: "badge-warn" };
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

<PageHeader subtitle="设备间文件传输队列" />

{#if data}
  <div class="grid grid-cols-2 gap-3 sm:grid-cols-4">
    <div class="list row stat"><span class="k">活跃</span><span class="v">{data.active_count}</span></div>
    <div class="list row stat"><span class="k">历史</span><span class="v">{data.terminal_count}</span></div>
    <div class="list row stat"><span class="k">已发送</span><span class="v" style="font-size: 16px">{bytes(data.stats.bytes_sent)}</span></div>
    <div class="list row stat"><span class="k">已接收</span><span class="v" style="font-size: 16px">{bytes(data.stats.bytes_received)}</span></div>
  </div>
{/if}

<div class="list-title">
  传输队列 <span class="dim font-normal">活跃 {data?.active_count ?? 0} · 历史 {data?.terminal_count ?? 0}</span>
  {#if (data?.terminal_count ?? 0) > 0}
    <span class="acts"><button type="button" class="btn btn-sm btn-text" disabled={busyId === "__all__"} onclick={clearAll}><Icon name="trash" size={12} />清空历史</button></span>
  {/if}
</div>
<div class="list">
  {#if !data || data.transfers.length === 0}
    <EmptyState icon="send" title="暂无传输记录" description="拖拽文件到设备，或从其他设备发送，进度会显示在这里。" />
  {:else}
    {#each data.transfers as t (t.transfer_id)}
      <div class="row hov">
        <span class="ic lg {t.direction === 'inbound' ? 'text-sky-600' : 'text-leaf-600'}"><Icon name={t.direction === "inbound" ? "download" : "upload"} size={15} /></span>
        <div class="main">
          <div class="flex items-center gap-2">
            <span class="name truncate">{t.filename}</span>
            <span class="badge">{dirLabel[t.direction] ?? t.direction}</span>
          </div>
          <div class="mt-1.5">
            <ProgressBar value={pctNum(t)} status={barStatus(t.phase)} label="{bytes(t.transferred)} / {bytes(t.total)} · {bytes(t.speed_bps)}/s" />
          </div>
          {#if t.error}<div class="sub text-[var(--err)]">{t.error}</div>{/if}
        </div>
        <div class="acts">
          <span class="badge {phaseBadge[t.phase] ?? ''}">{phaseLabel[t.phase] ?? t.phase}</span>
          {#if isActive(t.phase)}
            <button type="button" class="btn btn-sm" disabled={busyId === t.transfer_id} onclick={() => cancel(t)}>取消</button>
          {/if}
        </div>
      </div>
    {/each}
  {/if}
</div>
