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

<PageHeader subtitle="其他设备发来的文件" />

<div class="list-title">
  已接收文件 <span class="dim font-normal">{data?.count ?? 0} · {bytes(data?.total_bytes ?? 0)}</span>
  {#if (data?.count ?? 0) > 0}
    <span class="acts"><button type="button" class="btn btn-sm btn-text" disabled={busyName === "__all__"} onclick={clearAll}><Icon name="trash" size={12} />清空</button></span>
  {/if}
</div>
<div class="list">
  {#if !data || data.files.length === 0}
    <EmptyState icon="inbox" title="收件箱是空的" description="接收到的文件会出现在这里。" />
  {:else}
    {#each data.files as f (f.name)}
      <div class="row hov">
        <span class="ic lg"><Icon name="file" size={15} /></span>
        <div class="main">
          <div class="name truncate">{f.name}</div>
          <div class="sub">{bytes(f.size)} · {whenSecs(f.modified_at_secs)}</div>
        </div>
        {#if !f.is_safe}
          <span class="badge badge-warn"><Icon name="alert-triangle" size={11} />文件名不安全</span>
        {/if}
        <div class="acts">
          {#if f.is_safe}
            <button type="button" class="btn btn-sm" disabled={busyName === f.name} onclick={() => open(f)}><Icon name="external-link" size={12} />打开</button>
          {/if}
          <button type="button" class="btn btn-sm btn-text btn-danger" disabled={busyName === f.name} onclick={() => del(f)}>删除</button>
        </div>
      </div>
    {/each}
  {/if}
</div>
