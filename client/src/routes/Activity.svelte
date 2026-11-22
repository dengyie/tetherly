<script lang="ts">
  import type { Activity } from "../lib/api/gen/api";
  import { whenMs } from "../lib/fmt";
  import Icon from "../lib/ui/Icon.svelte";
  import PageHeader from "../lib/ui/PageHeader.svelte";
  import EmptyState from "../lib/ui/EmptyState.svelte";

  let { data }: { data: Activity | null } = $props();

  const statusLabel: Record<string, string> = {
    pending: "待处理",
    opened: "已打开",
    dismissed: "已忽略",
    rejected: "已拒绝",
  };
  const statusTone: Record<string, string> = { pending: "badge-warn", opened: "badge-ok", rejected: "badge-err" };

  const empty = $derived(
    !data ||
      (data.candidates.length === 0 && data.replies.length === 0 && data.messages.length === 0 && data.handoffs.length === 0),
  );
</script>

<PageHeader subtitle="设备间的通知、消息和投递" />

<div class="grid gap-x-4 lg:grid-cols-2">
  <div>
    <div class="list-title">通知候选 <span class="acts dim">{data?.candidates.length ?? 0}</span></div>
    <div class="list">
      {#if (data?.candidates.length ?? 0) === 0}
        <EmptyState compact icon="bell" title="暂无通知候选" />
      {:else}
        {#each data?.candidates ?? [] as c (c.id)}
          <div class="row hov">
            <span class="ic"><Icon name="bell" size={15} /></span>
            <div class="main">
              <div class="flex items-center gap-2">
                <span class="name">{c.app_name || c.app_id}</span>
                {#if c.has_otp}<span class="badge badge-accent"><Icon name="key" size={10} />含验证码</span>{/if}
              </div>
              <div class="text-[12.5px]">{c.title}</div>
              <div class="sub">来自 {c.source} · {whenMs(c.expires_ms)} 过期</div>
            </div>
          </div>
        {/each}
      {/if}
    </div>
  </div>

  <div>
    <div class="list-title">待取回复 <span class="acts dim">{data?.replies.length ?? 0}</span></div>
    <div class="list">
      {#if (data?.replies.length ?? 0) === 0}
        <EmptyState compact icon="message" title="没有等待取回的快速回复" />
      {:else}
        {#each data?.replies ?? [] as r (r.id)}
          <div class="row hov">
            <span class="ic"><Icon name="message" size={15} /></span>
            <div class="main"><div class="name">{r.app_id}</div><div class="sub">来自 {r.source}</div></div>
          </div>
        {/each}
      {/if}
    </div>
  </div>

  <div>
    <div class="list-title">直连消息 <span class="acts dim">{data?.messages.length ?? 0}</span></div>
    <div class="list">
      {#if (data?.messages.length ?? 0) === 0}
        <EmptyState compact icon="send" title="暂无消息记录" />
      {:else}
        {#each data?.messages ?? [] as m (m.id)}
          <div class="row hov">
            <span class="badge shrink-0">{m.direction === "inbound" ? "收" : "发"}</span>
            <div class="main">
              <div class="break-words text-[12.5px]">{m.text}</div>
              <div class="sub">{m.peer} · {whenMs(m.timestamp_ms)}</div>
            </div>
            {#if !m.read}<span class="dot" data-state="connecting" title="未读"></span>{/if}
          </div>
        {/each}
      {/if}
    </div>
  </div>

  <div>
    <div class="list-title">资源投递 <span class="acts dim">{data?.handoffs.length ?? 0}</span></div>
    <div class="list">
      {#if (data?.handoffs.length ?? 0) === 0}
        <EmptyState compact icon="external-link" title="暂无来自其他设备的链接投递" />
      {:else}
        {#each data?.handoffs ?? [] as h (h.id)}
          <div class="row hov">
            <span class="ic"><Icon name="external-link" size={15} /></span>
            <div class="main">
              <div class="name truncate">{h.title || h.url}</div>
              <div class="sub">{h.sender_alias || h.sender_id} · {whenMs(h.received_ms)}</div>
            </div>
            <span class="badge {statusTone[h.status] ?? ''}">{statusLabel[h.status] ?? h.status}</span>
          </div>
        {/each}
      {/if}
    </div>
  </div>
</div>
{#if empty}
  <p class="dim m-0 text-center text-[11.5px]">设备间的通知、消息和投递都会出现在这里。</p>
{/if}
