<script lang="ts">
  import type { Transfers } from "../lib/api/gen/api";

  let { data }: { data: Transfers | null } = $props();

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
</script>

<h1>传输</h1>
<section class="card">
  <h2>传输队列（活跃 {data?.active_count ?? 0} · 历史 {data?.terminal_count ?? 0}）</h2>
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
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .card { background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius); padding: var(--pad); margin-bottom: 12px; }
  h2 { margin: 0 0 8px; font-size: 15px; }
  .dim { color: var(--text-dim); }
  ul { list-style: none; margin: 0; padding: 0; }
  li { padding: 6px 0; display: flex; gap: 8px; align-items: baseline; flex-wrap: wrap; }
  .dir { font-size: 12px; border: 1px solid var(--border); border-radius: 4px; padding: 0 6px; }
  .dir[data-dir="outbound"] { color: var(--text-dim); }
  .phase { margin-left: auto; font-size: 12px; color: var(--text-dim); }
  .phase[data-phase="completed"] { color: var(--ok); }
  .phase[data-phase="failed"] { color: var(--danger, #c0392b); }
  .err { color: var(--danger, #c0392b); font-size: 12px; }
</style>
