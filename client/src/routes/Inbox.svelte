<script lang="ts">
  import type { Inbox } from "../lib/api/gen/api";
  import type { Api } from "../lib/api/client";

  let { data, api }: { data: Inbox | null; api: Api } = $props();

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

  let busyName: string | null = $state(null);
  let notice = $state("");
  async function run(name: string, op: () => Promise<unknown>, ok: string) {
    busyName = name;
    notice = "";
    try {
      await op();
      notice = ok;
    } catch (e) {
      notice = e instanceof Error ? e.message : String(e);
    } finally {
      busyName = null;
    }
  }
  const open = (f: { name: string }) =>
    run(f.name, () => api.openInboxFile(f.name), `已打开 ${f.name}`);
  const del = (f: { name: string }) =>
    run(f.name, () => api.deleteInboxFile(f.name), `已删除 ${f.name}`);
  const clearAll = () =>
    run("__all__", () => api.clearInbox(), "收件箱已清空");
</script>

<h1>收件箱</h1>
{#if notice}<p class="notice">{notice}</p>{/if}
<section class="card">
  <h2>
    已接收文件（{data?.count ?? 0} · {bytes(data?.total_bytes ?? 0)}）
    {#if (data?.count ?? 0) > 0}
      <button class="clear" disabled={busyName === "__all__"} onclick={clearAll}>清空</button>
    {/if}
  </h2>
  {#if !data || data.files.length === 0}
    <p class="dim">收件箱是空的。接收到的文件会出现在这里。</p>
  {:else}
    <ul>
      {#each data.files as f (f.name)}
        <li>
          <strong>{f.name}</strong>
          <span class="dim">{bytes(f.size)} · {when(f.modified_at_secs)}</span>
          {#if !f.is_safe}<span class="warn">文件名不安全</span>{/if}
          <span class="actions">
            {#if f.is_safe}
              <button disabled={busyName === f.name} onclick={() => open(f)}>打开</button>
            {/if}
            <button class="danger" disabled={busyName === f.name} onclick={() => del(f)}>删除</button>
          </span>
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
  .warn { color: var(--danger, #c0392b); font-size: 12px; }
  .actions { margin-left: auto; display: flex; gap: 6px; }
  button {
    border: 1px solid var(--border); background: var(--bg); color: var(--text);
    padding: 3px 10px; border-radius: 6px; cursor: pointer; font: inherit; font-size: 13px;
  }
  button:disabled { opacity: 0.5; cursor: default; }
  button.danger { color: var(--danger, #c0392b); }
  button.clear { margin-left: auto; font-size: 12px; }
</style>
