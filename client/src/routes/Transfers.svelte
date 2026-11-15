<script lang="ts">
  import type { Transfers } from "../lib/api/gen/api";
  import type { Api } from "../lib/api/client";

  let { data, api }: { data: Transfers | null; api: Api } = $props();

  const phaseLabel: Record<string, string> = {
    transferring: "传输中",
    verifying: "校验中",
    completed: "已完成",
    failed: "失败",
  };
  const dirLabel: Record<string, string> = {
    inbound: "接收",
    outbound: "发送",
  };
  function pct(t: { total: number; transferred: number }): string {
    if (t.total === 0) return "—";
    return `${Math.min(100, Math.round((t.transferred / t.total) * 100))}%`;
  }
  function bytes(n: number): string {
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
    if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
    return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
  }
  const active = $derived(
    (data?.transfers ?? []).filter(
      (t) => t.phase === "transferring" || t.phase === "verifying",
    ),
  );

  let busyId: string | null = $state(null);
  let notice = $state("");
  async function cancel(t: { transfer_id: string; filename: string }) {
    busyId = t.transfer_id;
    notice = "";
    try {
      await api.cancelTransfer(t.transfer_id);
      notice = `已取消 ${t.filename}`;
    } catch (e) {
      notice = e instanceof Error ? e.message : String(e);
    } finally {
      busyId = null;
    }
  }
  async function clearAll() {
    busyId = "__all__";
    notice = "";
    try {
      await api.clearTransfers();
      notice = "已清空历史记录";
    } catch (e) {
      notice = e instanceof Error ? e.message : String(e);
    } finally {
      busyId = null;
    }
  }
</script>

<h1>传输</h1>
{#if notice}<p class="notice">{notice}</p>{/if}
<section class="card">
  <h2>
    传输队列（活跃 {data?.active_count ?? 0} · 历史 {data?.terminal_count ?? 0}）
    {#if (data?.terminal_count ?? 0) > 0}
      <button class="clear" disabled={busyId === "__all__"} onclick={clearAll}>清空历史</button>
    {/if}
  </h2>
  {#if !data || data.transfers.length === 0}
    <p class="dim">暂无传输记录。</p>
  {:else}
    <ul>
      {#each data.transfers as t (t.transfer_id)}
        <li>
          <span class="dir" data-dir={t.direction}>{dirLabel[t.direction] ?? t.direction}</span>
          <strong>{t.filename}</strong>
          <span class="dim">{bytes(t.transferred)} / {bytes(t.total)} · {pct(t)} · {bytes(t.speed_bps)}/s</span>
          <span class="phase" data-phase={t.phase}>{phaseLabel[t.phase] ?? t.phase}</span>
          {#if t.error}<span class="err">{t.error}</span>{/if}
          {#if active.some((a) => a.transfer_id === t.transfer_id)}
            <button disabled={busyId === t.transfer_id} onclick={() => cancel(t)}>取消</button>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .card { background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius); padding: var(--pad); margin-bottom: 12px; }
  h2 { margin: 0 0 8px; font-size: 15px; display: flex; align-items: center; gap: 10px; }
  .dim { color: var(--text-dim); }
  .notice { color: var(--text-dim); font-size: 13px; }
  ul { list-style: none; margin: 0; padding: 0; }
  li { padding: 6px 0; display: flex; gap: 8px; align-items: baseline; flex-wrap: wrap; }
  .dir { font-size: 12px; border: 1px solid var(--border); border-radius: 4px; padding: 0 6px; }
  .dir[data-dir="outbound"] { color: var(--text-dim); }
  .phase { margin-left: auto; font-size: 12px; color: var(--text-dim); }
  .phase[data-phase="completed"] { color: var(--ok); }
  .phase[data-phase="failed"] { color: var(--danger, #c0392b); }
  .err { color: var(--danger, #c0392b); font-size: 12px; }
  button {
    border: 1px solid var(--border); background: var(--bg); color: var(--text);
    padding: 3px 10px; border-radius: 6px; cursor: pointer; font: inherit; font-size: 13px;
  }
  button:disabled { opacity: 0.5; cursor: default; }
  button.clear { margin-left: auto; font-size: 12px; }
</style>
