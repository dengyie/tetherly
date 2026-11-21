<script lang="ts">
  import type { Inbox } from "../lib/api/gen/api";
  import type { Api } from "../lib/api/client";
  import { bytes, whenSecs } from "../lib/fmt";
  import { withToast } from "../lib/ui/toast";
  import Icon from "../lib/ui/Icon.svelte";
  import PageHeader from "../lib/ui/PageHeader.svelte";
  import EmptyState from "../lib/ui/EmptyState.svelte";

  let { data, api }: { data: Inbox | null; api: Api } = $props();

  let busyName: string | null = $state(null);
  async function run(name: string, op: () => Promise<unknown>, ok: string) {
    busyName = name;
    await withToast(op, ok);
    busyName = null;
  }
  const open = (f: { name: string }) => run(f.name, () => api.openInboxFile(f.name), `已打开 ${f.name}`);
  const del = (f: { name: string }) => run(f.name, () => api.deleteInboxFile(f.name), `已删除 ${f.name}`);
  const clearAll = () => run("__all__", () => api.clearInbox(), "收件箱已清空");
</script>

<PageHeader title="收件箱" subtitle="其他设备发来的文件" icon="inbox">
  {#if (data?.count ?? 0) > 0}
    <button type="button" class="btn btn-sm" disabled={busyName === "__all__"} onclick={clearAll}><Icon name="trash" size={13} />清空</button>
  {/if}
</PageHeader>

<section class="card">
  <h2 class="card-title"><Icon name="file" size={15} class="text-zinc-400" />已接收文件（{data?.count ?? 0} · {bytes(data?.total_bytes ?? 0)}）</h2>
  {#if !data || data.files.length === 0}
    <EmptyState icon="inbox" title="收件箱是空的" description="接收到的文件会出现在这里。" />
  {:else}
    <ul class="m-0 list-none p-0">
      {#each data.files as f (f.name)}
        <li class="row">
          <span class="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-zinc-100 dark:bg-zinc-800 text-zinc-500"><Icon name="file" size={15} /></span>
          <div class="min-w-0 flex-1">
            <div class="truncate font-medium">{f.name}</div>
            <div class="text-xs dim">{bytes(f.size)} · {whenSecs(f.modified_at_secs)}</div>
          </div>
          {#if !f.is_safe}
            <span class="badge border-amber-200 bg-amber-50 text-amber-700 dark:border-amber-800 dark:bg-amber-950/40 dark:text-amber-400"><Icon name="alert-triangle" size={11} />文件名不安全</span>
          {/if}
          <div class="flex items-center gap-1.5">
            {#if f.is_safe}
              <button type="button" class="btn btn-sm" disabled={busyName === f.name} onclick={() => open(f)}><Icon name="external-link" size={13} />打开</button>
            {/if}
            <button type="button" class="btn btn-danger btn-sm" disabled={busyName === f.name} onclick={() => del(f)}>删除</button>
          </div>
        </li>
      {/each}
    </ul>
  {/if}
</section>
