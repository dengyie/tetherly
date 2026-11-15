<script lang="ts">
  import type { Activity } from "../lib/api/gen/api";

  let { data }: { data: Activity | null } = $props();

  function when(ms: number): string {
    if (!ms) return "—";
    return new Date(ms).toLocaleTimeString();
  }
  const statusLabel: Record<string, string> = {
    pending: "待处理",
    opened: "已打开",
    dismissed: "已忽略",
    rejected: "已拒绝",
  };

  const empty =
    $derived(
      !data ||
        (data.candidates.length === 0 &&
          data.replies.length === 0 &&
          data.messages.length === 0 &&
          data.handoffs.length === 0),
    );
</script>

<h1>动态</h1>
<section class="card">
  <h2>通知候选</h2>
  {#if (data?.candidates.length ?? 0) === 0}
    <p class="dim">暂无通知候选。</p>
  {:else}
    <ul>
      {#each data?.candidates ?? [] as c (c.id)}
        <li>
          <strong>{c.app_name || c.app_id}</strong>
          <span>{c.title}</span>
          {#if c.has_otp}<span class="otp">含验证码</span>{/if}
          <span class="dim">来自 {c.source} · {when(c.expires_ms)} 过期</span>
        </li>
      {/each}
    </ul>
  {/if}
</section>
<section class="card">
  <h2>待取回复</h2>
  {#if (data?.replies.length ?? 0) === 0}
    <p class="dim">没有等待取回的快速回复。</p>
  {:else}
    <ul>
      {#each data?.replies ?? [] as r (r.id)}
        <li>
          <strong>{r.app_id}</strong>
          <span class="dim">来自 {r.source}</span>
        </li>
      {/each}
    </ul>
  {/if}
</section>
<section class="card">
  <h2>直连消息</h2>
  {#if (data?.messages.length ?? 0) === 0}
    <p class="dim">暂无消息记录。</p>
  {:else}
    <ul>
      {#each data?.messages ?? [] as m (m.id)}
        <li>
          <span class="dir" data-dir={m.direction}>{m.direction === "inbound" ? "收" : "发"}</span>
          <span>{m.text}</span>
          {#if !m.read}<span class="unread">未读</span>{/if}
          <span class="dim">{m.peer} · {when(m.timestamp_ms)}</span>
        </li>
      {/each}
    </ul>
  {/if}
</section>
<section class="card">
  <h2>资源投递</h2>
  {#if (data?.handoffs.length ?? 0) === 0}
    <p class="dim">暂无来自其他设备的链接投递。</p>
  {:else}
    <ul>
      {#each data?.handoffs ?? [] as h (h.id)}
        <li>
          <strong>{h.title || h.url}</strong>
          <span class="dim">{h.sender_alias || h.sender_id} · {when(h.received_ms)}</span>
          <span class="status" data-status={h.status}>{statusLabel[h.status] ?? h.status}</span>
        </li>
      {/each}
    </ul>
  {/if}
</section>
{#if empty}
  <p class="dim center">设备间的通知、消息和投递都会出现在这里。</p>
{/if}

<style>
  .card { background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius); padding: var(--pad); margin-bottom: 12px; }
  h2 { margin: 0 0 8px; font-size: 15px; }
  .dim { color: var(--text-dim); font-size: 13px; }
  .center { text-align: center; margin-top: 24px; }
  ul { list-style: none; margin: 0; padding: 0; }
  li { padding: 6px 0; display: flex; gap: 8px; align-items: baseline; flex-wrap: wrap; }
  .otp { color: var(--warning, #b8860b); font-size: 12px; }
  .unread { color: var(--warning, #b8860b); font-size: 12px; }
  .dir { font-size: 12px; border: 1px solid var(--border); border-radius: 4px; padding: 0 6px; }
  .status { margin-left: auto; font-size: 12px; color: var(--text-dim); }
  .status[data-status="pending"] { color: var(--warning, #b8860b); }
  .status[data-status="opened"] { color: var(--ok); }
</style>
