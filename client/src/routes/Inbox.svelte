<script lang="ts">
  import type { Inbox } from "../lib/api/gen/api";

  let { data }: { data: Inbox | null } = $props();

  function bytes(n: number): string {
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
    if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
    return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
  }
  function when(secs: number): string {
    if (!secs) return "—";
    return new Date(secs * 1000).toLocaleString();
  }
</script>

<h1>收件箱</h1>
<section class="card">
  <h2>已接收文件（{data?.count ?? 0} · {bytes(data?.total_bytes ?? 0)}）</h2>
  {#if !data || data.files.length === 0}
    <p class="dim">收件箱是空的。接收到的文件会出现在这里。</p>
  {:else}
    <ul>
      {#each data.files as f (f.name)}
        <li>
          <strong>{f.name}</strong>
          <span class="dim">{bytes(f.size)} · {when(f.modified_at_secs)}</span>
          {#if !f.is_safe}<span class="warn">文件名不安全</span>{/if}
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
  .warn { color: var(--danger, #c0392b); font-size: 12px; }
</style>
