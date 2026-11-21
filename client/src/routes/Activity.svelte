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
  const statusTone: Record<string, string> = {
    pending: "border-amber-200 bg-amber-50 text-amber-700 dark:border-amber-800 dark:bg-amber-950/40 dark:text-amber-400",
    opened: "border-emerald-200 bg-emerald-50 text-emerald-700 dark:border-emerald-800 dark:bg-emerald-950/40 dark:text-emerald-400",
    rejected: "border-rose-200 bg-rose-50 text-rose-700 dark:border-rose-800 dark:bg-rose-950/40 dark:text-rose-400",
  };

  const empty = $derived(
    !data ||
      (data.candidates.length === 0 && data.replies.length === 0 && data.messages.length === 0 && data.handoffs.length === 0),
  );
</script>

<PageHeader title="动态" subtitle="设备间的通知、消息和投递" icon="activity" />

<div class="grid gap-4 lg:grid-cols-2">
  <section class="card">
    <h2 class="card-title"><Icon name="bell" size={15} class="text-zinc-400" />通知候选</h2>
    {#if (data?.candidates.length ?? 0) === 0}
      <EmptyState compact icon="bell" title="暂无通知候选" />
    {:else}
      <ul class="m-0 list-none p-0">
        {#each data?.candidates ?? [] as c (c.id)}
          <li class="row items-start">
            <div class="min-w-0 flex-1">
              <div class="flex items-center gap-2">
                <span class="font-medium">{c.app_name || c.app_id}</span>
                {#if c.has_otp}<span class="badge border-brand-200 bg-brand-50 text-brand-700 dark:border-brand-800 dark:bg-brand-950/40 dark:text-brand-300"><Icon name="key" size={11} />含验证码</span>{/if}
              </div>
              <div class="text-[13px]">{c.title}</div>
              <div class="text-xs dim">来自 {c.source} · {whenMs(c.expires_ms)} 过期</div>
            </div>
          </li>
        {/each}
      </ul>
    {/if}
  </section>

  <section class="card">
    <h2 class="card-title"><Icon name="message" size={15} class="text-zinc-400" />待取回复</h2>
    {#if (data?.replies.length ?? 0) === 0}
      <EmptyState compact icon="message" title="没有等待取回的快速回复" />
    {:else}
      <ul class="m-0 list-none p-0">
        {#each data?.replies ?? [] as r (r.id)}
          <li class="row">
            <span class="font-medium">{r.app_id}</span>
            <span class="text-xs dim">来自 {r.source}</span>
          </li>
        {/each}
      </ul>
    {/if}
  </section>

  <section class="card">
    <h2 class="card-title"><Icon name="send" size={15} class="text-zinc-400" />直连消息</h2>
    {#if (data?.messages.length ?? 0) === 0}
      <EmptyState compact icon="send" title="暂无消息记录" />
    {:else}
      <ul class="m-0 list-none p-0">
        {#each data?.messages ?? [] as m (m.id)}
          <li class="row items-start">
            <span class="badge shrink-0">{m.direction === "inbound" ? "收" : "发"}</span>
            <div class="min-w-0 flex-1">
              <div class="break-words text-[13px]">{m.text}</div>
              <div class="text-xs dim">{m.peer} · {whenMs(m.timestamp_ms)}</div>
            </div>
            {#if !m.read}<span class="inline-flex h-2 w-2 shrink-0 rounded-full bg-brand-500" title="未读"></span>{/if}
          </li>
        {/each}
      </ul>
    {/if}
  </section>

  <section class="card">
    <h2 class="card-title"><Icon name="external-link" size={15} class="text-zinc-400" />资源投递</h2>
    {#if (data?.handoffs.length ?? 0) === 0}
      <EmptyState compact icon="external-link" title="暂无来自其他设备的链接投递" />
    {:else}
      <ul class="m-0 list-none p-0">
        {#each data?.handoffs ?? [] as h (h.id)}
          <li class="row items-start">
            <div class="min-w-0 flex-1">
              <div class="truncate font-medium">{h.title || h.url}</div>
              <div class="text-xs dim">{h.sender_alias || h.sender_id} · {whenMs(h.received_ms)}</div>
            </div>
            <span class="badge {statusTone[h.status] ?? ''}">{statusLabel[h.status] ?? h.status}</span>
          </li>
        {/each}
      </ul>
    {/if}
  </section>
</div>
{#if empty}
  <p class="m-0 text-center text-xs dim">设备间的通知、消息和投递都会出现在这里。</p>
{/if}
