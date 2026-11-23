<script lang="ts">
  import type { Shelf, ShelfItem, Transfers } from "../lib/api/gen/api";
  import type { Api } from "../lib/api/client";
  import { bytes } from "../lib/fmt";
  import { pushToast, withToast } from "../lib/ui/toast";
  import { listenFileDrops } from "../lib/shelf";
  import Icon from "../lib/ui/Icon.svelte";
  import PageHeader from "../lib/ui/PageHeader.svelte";
  import EmptyState from "../lib/ui/EmptyState.svelte";
  import ProgressBar from "../lib/ui/ProgressBar.svelte";

  let { data, api, shelfTick = 0 }: { data: Transfers | null; api: Api; shelfTick?: number } = $props();

  // v0.2.89 共享架: dropping files here only registers path/size/mtime; the
  // bytes move when another device presses 下载 (the owner re-checks then).
  let shelf: Shelf | null = $state(null);
  let dropOver = $state(false);
  async function loadShelf() {
    try {
      shelf = await api.shelf();
    } catch {
      // Older daemon or transient link loss: keep the last view.
    }
  }
  $effect(() => {
    void shelfTick;
    void loadShelf();
  });
  async function addPaths(paths: string[]) {
    try {
      const r = await api.addShelf(paths);
      if (r.added.length > 0) pushToast(`已加入共享架 ${r.added.length} 个文件`, "success");
      for (const e of r.errors.slice(0, 3)) pushToast(e, "error", 6000);
      await loadShelf();
    } catch (e) {
      pushToast(e instanceof Error ? e.message : String(e), "error");
    }
  }
  $effect(() => listenFileDrops((p) => void addPaths(p), (o) => (dropOver = o)));
  const unshare = (it: ShelfItem) => run(`mine:${it.id}`, async () => {
    await api.removeShelf(it.id);
    await loadShelf();
  }, `已移出共享 ${it.name}`);
  const fetchItem = (owner: string, it: ShelfItem) =>
    run(`${owner}:${it.id}`, () => api.fetchShelf(owner, it.id), `已请求下载 ${it.name}，完成后在收件箱查看`);

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
  共享架 <span class="dim font-normal">把文件拖进此窗口即可共享，其他设备点「下载」时才真正传输</span>
</div>
<div class="list shelf-drop" class:over={dropOver}>
  {#if !shelf || shelf.mine.length === 0}
    <EmptyState icon="upload" title="我的共享为空" description="拖拽文件到此窗口，登记路径后会出现在其他已信任设备的列表中。" />
  {:else}
    {#each shelf.mine as it (it.id)}
      <div class="row hov">
        <span class="ic lg text-leaf-600"><Icon name="file" size={15} /></span>
        <div class="main">
          <div class="flex items-center gap-2">
            <span class="name truncate">{it.name}</span>
            {#if !it.available}<span class="badge badge-warn">已失效</span>{/if}
          </div>
          <div class="sub">{bytes(it.size)}</div>
        </div>
        <div class="acts">
          <button type="button" class="btn btn-sm btn-text" disabled={busyId === `mine:${it.id}`} onclick={() => unshare(it)}><Icon name="x" size={12} />移除</button>
        </div>
      </div>
    {/each}
  {/if}
</div>

{#if shelf && shelf.peers.length > 0}
  {#each shelf.peers as peer (peer.device_id)}
    <div class="list-title">{peer.name} 的共享 <span class="dim font-normal">{peer.items.length} 个文件</span></div>
    <div class="list">
      {#each peer.items as it (it.id)}
        <div class="row hov">
          <span class="ic lg text-sky-600"><Icon name="file" size={15} /></span>
          <div class="main">
            <div class="flex items-center gap-2">
              <span class="name truncate">{it.name}</span>
              {#if !it.available}<span class="badge badge-warn">已失效</span>{/if}
            </div>
            <div class="sub">{bytes(it.size)}</div>
          </div>
          <div class="acts">
            <button type="button" class="btn btn-sm" disabled={!it.available || busyId === `${peer.device_id}:${it.id}`} onclick={() => fetchItem(peer.device_id, it)}><Icon name="download" size={12} />下载</button>
          </div>
        </div>
      {/each}
    </div>
  {/each}
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

<style>
  .shelf-drop {
    transition: box-shadow 120ms ease, background-color 120ms ease;
  }
  .shelf-drop.over {
    box-shadow: inset 0 0 0 2px var(--accent, #4f8cff);
  }
</style>
