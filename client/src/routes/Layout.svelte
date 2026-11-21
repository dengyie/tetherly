<script lang="ts">
  // Visual 布局 editor: the local display sits in the middle, the four edge
  // slots show which peer each edge crosses into. Writes go through the
  // topology API; the daemon answers with `layout.changed`, which is the only
  // thing that updates `data` (store-driven, no local mirror of the layout).
  import type { Layout, LayoutEdge } from "../lib/api/gen/api";
  import type { DevicesState } from "../lib/stores/devices";
  import type { Api } from "../lib/api/client";
  import { withToast } from "../lib/ui/toast";
  import Icon from "../lib/ui/Icon.svelte";
  import PageHeader from "../lib/ui/PageHeader.svelte";
  import EmptyState from "../lib/ui/EmptyState.svelte";
  import Switch from "../lib/ui/Switch.svelte";

  let { data, devices, api }: { data: Layout | null; devices: DevicesState; api: Api } = $props();

  type Edge = "left" | "right" | "top" | "bottom";
  const EDGES: Edge[] = ["left", "right", "top", "bottom"];
  const edgeLabel: Record<Edge, string> = { left: "左", right: "右", top: "上", bottom: "下" };
  const edgeIcon: Record<Edge, string> = { left: "arrow-left", right: "arrow-right", top: "arrow-up", bottom: "arrow-down" };

  const trusted = $derived([...devices.byId.values()].filter((d) => d.trusted && !d.blocked));
  const peerOf = $derived.by(() => {
    const m = new Map<Edge, string | null>();
    for (const e of EDGES) m.set(e, null);
    for (const e of data?.edges ?? []) m.set(e.edge as Edge, e.peer);
    return m;
  });
  function peerName(id: string | null): string {
    if (!id) return "";
    return devices.byId.get(id)?.name ?? id;
  }
  const linkedEdges = $derived(EDGES.filter((e) => peerOf.get(e)));
  // Keep the preview inside a 220×140 box, true to the display aspect ratio.
  const box = $derived.by(() => {
    const w = data?.display.width || 16;
    const h = data?.display.height || 9;
    const k = Math.min(220 / w, 140 / h);
    return { w: Math.round(w * k), h: Math.round(h * k) };
  });

  let busy = $state(false);
  let picking: Edge | null = $state(null);
  let dragOver: Edge | null = $state(null);

  async function run(op: () => Promise<unknown>, ok?: string) {
    busy = true;
    const r = await withToast(op, ok);
    busy = false;
    return r;
  }
  function link(edge: Edge, peer: string) {
    picking = null;
    void run(() => api.linkEdge({ edge, peer }), `已将 ${edgeLabel[edge]}侧连接到 ${peerName(peer)}`);
  }
  function unlink(edge: Edge) {
    void run(() => api.unlinkEdge({ edge }), `已断开 ${edgeLabel[edge]}侧`);
  }
  const sync = () => run(() => api.syncTopology(), "已向对端同步布局");

  function onDrop(e: DragEvent, edge: Edge) {
    e.preventDefault();
    dragOver = null;
    const id = e.dataTransfer?.getData("text/tetherly-device");
    if (id) link(edge, id);
  }
  function onDragStart(e: DragEvent, id: string) {
    e.dataTransfer?.setData("text/tetherly-device", id);
    if (e.dataTransfer) e.dataTransfer.effectAllowed = "link";
  }

  // Sliders commit on `change` (pointer release), not on every `input` tick.
  function num(e: Event): number {
    return Number((e.currentTarget as HTMLInputElement).value);
  }
  const cfg = (patch: Parameters<Api["patchTopologyConfig"]>[0]) => run(() => api.patchTopologyConfig(patch));
  const pol = (patch: Parameters<Api["patchTopologyPolicy"]>[0]) => run(() => api.patchTopologyPolicy(patch));
</script>

<PageHeader title="布局" subtitle="把设备拖到屏幕的某一侧，键鼠推过去就会跨屏" icon="layout">
  <button type="button" class="btn btn-sm" disabled={busy || !data} onclick={sync}><Icon name="refresh" size={13} />同步到对端</button>
</PageHeader>

{#if !data}
  <section class="card"><EmptyState icon="layout" title="等待布局数据…" description="daemon 推送 layout.changed 后会显示屏幕与边缘配置。" /></section>
{:else}
  <div class="grid gap-4 lg:grid-cols-[1fr_280px]">
    <section class="card">
      <h2 class="card-title"><Icon name="monitor" size={15} class="text-zinc-400" />屏幕边缘</h2>
      <div class="grid grid-cols-[128px_1fr_128px] grid-rows-[56px_1fr_56px] items-center justify-items-center gap-2" style="min-height: 280px">
        {#snippet slot(edge: Edge)}
          {@const peer = peerOf.get(edge) ?? null}
          <div
            class="group relative flex min-h-12 w-full max-w-48 items-center justify-center rounded-xl border-2 border-dashed px-2 py-1.5 text-xs transition
              {dragOver === edge ? 'border-indigo-500 bg-indigo-500/10' : peer ? 'border-indigo-300/70 bg-indigo-500/5 dark:border-indigo-700/60' : 'border-zinc-300/80 dark:border-zinc-700/70'}"
            data-edge={edge}
            role="group"
            aria-label="{edgeLabel[edge]}侧"
            ondragover={(e) => { e.preventDefault(); dragOver = edge; }}
            ondragleave={() => (dragOver = null)}
            ondrop={(e) => onDrop(e, edge)}
          >
            {#if peer}
              <span class="inline-flex items-center gap-1.5 truncate font-medium"><Icon name="link" size={12} class="text-indigo-500" />{peerName(peer)}</span>
              <button type="button" class="btn btn-ghost btn-sm absolute -right-2 -top-2 h-6 w-6 !p-0 opacity-0 transition group-hover:opacity-100 focus:opacity-100" aria-label="断开{edgeLabel[edge]}侧" disabled={busy} onclick={() => unlink(edge)}><Icon name="x" size={12} /></button>
            {:else}
              <button type="button" class="inline-flex items-center gap-1 dim hover:text-indigo-600" disabled={busy} onclick={() => (picking = picking === edge ? null : edge)}><Icon name="plus" size={12} /><span class="whitespace-nowrap">{edgeLabel[edge]}侧 · 未配置</span></button>
            {/if}
            {#if picking === edge}
              <div class="absolute left-1/2 top-full z-30 mt-1 w-44 -translate-x-1/2 rounded-xl border border-zinc-200 bg-white p-1 shadow-xl dark:border-zinc-700 dark:bg-zinc-900" role="menu">
                {#if trusted.length === 0}
                  <div class="px-2 py-1.5 text-xs dim">没有可用的已配对设备</div>
                {:else}
                  {#each trusted as d (d.id)}
                    <button type="button" role="menuitem" class="flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left text-xs hover:bg-zinc-100 dark:hover:bg-zinc-800" onclick={() => link(edge, d.id)}>
                      <span class="inline-flex h-1.5 w-1.5 rounded-full {d.online ? 'bg-emerald-500' : 'bg-zinc-400'}"></span>{d.name}
                    </button>
                  {/each}
                {/if}
              </div>
            {/if}
          </div>
        {/snippet}

        <div></div>
        {@render slot("top")}
        <div></div>

        {@render slot("left")}
        <div class="relative flex items-center justify-center rounded-2xl border border-zinc-300/80 bg-gradient-to-br from-zinc-100 to-zinc-200 shadow-inner dark:border-zinc-700 dark:from-zinc-800 dark:to-zinc-900" style="width: {box.w}px; height: {box.h}px" data-testid="layout-display">
          <div class="text-center">
            <div class="text-xs font-semibold">本机</div>
            <div class="mono text-[11px] dim">{data.display.width} × {data.display.height}</div>
            <div class="text-[10px] dim">缩放 {Math.round(data.display.scale_factor * 100)}%</div>
          </div>
        </div>
        {@render slot("right")}

        <div></div>
        {@render slot("bottom")}
        <div></div>
      </div>
      <p class="m-0 mt-3 text-xs dim">已连接 {linkedEdges.length} / 4 个边缘。点击边缘可选择设备，或从右侧列表拖动设备到边缘。</p>
    </section>

    <section class="card">
      <h2 class="card-title"><Icon name="monitor" size={15} class="text-zinc-400" />已配对设备</h2>
      {#if trusted.length === 0}
        <EmptyState compact icon="link" title="还没有已配对设备" />
      {:else}
        <ul class="m-0 flex list-none flex-col gap-1.5 p-0">
          {#each trusted as d (d.id)}
            <li class="flex cursor-grab items-center gap-2 rounded-xl border border-zinc-200/80 bg-white/60 px-3 py-2 text-[13px] active:cursor-grabbing dark:border-zinc-700/70 dark:bg-zinc-900/40" draggable="true" ondragstart={(e) => onDragStart(e, d.id)} data-device-chip={d.id}>
              <span class="inline-flex h-2 w-2 rounded-full {d.online ? 'bg-emerald-500' : 'bg-zinc-400'}"></span>
              <span class="truncate font-medium">{d.name}</span>
              <span class="ml-auto text-[11px] dim">{EDGES.filter((e) => peerOf.get(e) === d.id).map((e) => edgeLabel[e]).join("") || ""}</span>
            </li>
          {/each}
        </ul>
      {/if}
    </section>
  </div>

  <div class="grid gap-4 lg:grid-cols-2">
    <section class="card">
      <h2 class="card-title"><Icon name="sliders" size={15} class="text-zinc-400" />跨屏参数</h2>
      <div class="flex flex-col gap-3">
        <label class="flex flex-col gap-1 text-xs">
          <span class="flex justify-between"><span>推动阻力</span><span class="mono dim">{data.config.resistance_px} px</span></span>
          <input type="range" min="0" max="200" step="5" value={data.config.resistance_px} disabled={busy} onchange={(e) => cfg({ resistance_px: num(e) })} aria-label="推动阻力" />
        </label>
        <label class="flex flex-col gap-1 text-xs">
          <span class="flex justify-between"><span>边缘停留</span><span class="mono dim">{data.config.delay_ms} ms</span></span>
          <input type="range" min="0" max="1000" step="25" value={data.config.delay_ms} disabled={busy} onchange={(e) => cfg({ delay_ms: num(e) })} aria-label="边缘停留" />
        </label>
        <label class="flex flex-col gap-1 text-xs">
          <span class="flex justify-between"><span>角落锁定</span><span class="mono dim">{data.config.corner_lock_size_px} px</span></span>
          <input type="range" min="0" max="200" step="5" value={data.config.corner_lock_size_px} disabled={busy} onchange={(e) => cfg({ corner_lock_size_px: num(e) })} aria-label="角落锁定" />
        </label>
        <label class="flex flex-col gap-1 text-xs">
          <span class="flex justify-between"><span>远端指针速度</span><span class="mono dim">{data.config.remote_speed_percent}%</span></span>
          <input type="range" min="25" max="300" step="5" value={data.config.remote_speed_percent} disabled={busy} onchange={(e) => cfg({ remote_speed_percent: num(e) })} aria-label="远端指针速度" />
        </label>
        <div class="row">
          <div class="flex-1"><div class="text-[13px]">边缘环绕</div><div class="text-xs dim">从最后一侧推出时回到第一侧。</div></div>
          <Switch checked={data.config.wrap_around} disabled={busy} label="边缘环绕" onchange={(v) => cfg({ wrap_around: v })} />
        </div>
      </div>
    </section>

    <section class="card">
      <h2 class="card-title"><Icon name="shield" size={15} class="text-zinc-400" />策略</h2>
      <ul class="m-0 list-none p-0">
        <li class="row">
          <div class="flex-1"><div class="text-[13px]">启用跨屏</div><div class="text-xs dim">关闭后键鼠不会离开本机。</div></div>
          <Switch checked={data.policy.enabled} disabled={busy} label="启用跨屏" onchange={(v) => pol({ enabled: v })} />
        </li>
        <li class="row">
          <div class="flex-1"><div class="text-[13px]">允许被远端控制</div><div class="text-xs dim">其他设备的键鼠可以跨进本机。</div></div>
          <Switch checked={data.policy.allow_inbound_control} disabled={busy} label="允许被远端控制" onchange={(v) => pol({ allow_inbound_control: v })} />
        </li>
        <li class="row">
          <div class="flex-1"><div class="text-[13px]">自动连接已配对设备</div><div class="text-xs dim">新配对的设备自动占用一个空闲边缘。</div></div>
          <Switch checked={data.policy.auto_link_paired_peers} disabled={busy} label="自动连接已配对设备" onchange={(v) => pol({ auto_link_paired_peers: v })} />
        </li>
      </ul>
    </section>
  </div>
{/if}
