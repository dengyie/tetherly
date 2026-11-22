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
  // Sliders and switches stay enabled while a PATCH is in flight (`aria-busy`
  // instead of `disabled`): disabling a focused control blurs it, which
  // swallowed the next keyboard step in the live keyboard walk.
  function num(e: Event): number {
    return Number((e.currentTarget as HTMLInputElement).value);
  }
  const cfg = (patch: Parameters<Api["patchTopologyConfig"]>[0]) => run(() => api.patchTopologyConfig(patch));
  const pol = (patch: Parameters<Api["patchTopologyPolicy"]>[0]) => run(() => api.patchTopologyPolicy(patch));
</script>

<PageHeader subtitle="把设备拖到屏幕的某一侧，键鼠推过去就会跨屏">
  <button type="button" class="btn btn-sm" disabled={busy || !data} onclick={sync}><Icon name="refresh" size={12} />同步到对端</button>
</PageHeader>

{#if !data}
  <div class="list"><EmptyState icon="layout" title="等待布局数据…" description="daemon 推送 layout.changed 后会显示屏幕与边缘配置。" /></div>
{:else}
  <div class="grid gap-x-4 lg:grid-cols-[1fr_260px]">
    <div>
      <div class="list-title">屏幕边缘 <span class="acts dim">已连接 {linkedEdges.length} / 4</span></div>
      <div class="list pad">
        <div class="grid grid-cols-[128px_1fr_128px] grid-rows-[56px_1fr_56px] items-center justify-items-center gap-2" style="min-height: 272px">
          {#snippet slot(edge: Edge)}
            {@const peer = peerOf.get(edge) ?? null}
            <div
              class="group relative flex min-h-11 w-full max-w-48 items-center justify-center rounded-[9px] border border-dashed px-2 py-1.5 text-[12px] transition
                {dragOver === edge ? 'border-[var(--accent)] bg-[var(--accent-soft)]' : peer ? 'border-solid border-[var(--line)] bg-[var(--pill)]' : 'border-[var(--line)]'}"
              data-edge={edge}
              role="group"
              aria-label="{edgeLabel[edge]}侧"
              ondragover={(e) => { e.preventDefault(); dragOver = edge; }}
              ondragleave={() => (dragOver = null)}
              ondrop={(e) => onDrop(e, edge)}
            >
              {#if peer}
                <span class="inline-flex items-center gap-1.5 truncate font-medium"><Icon name="link" size={12} class="text-[var(--accent)]" />{peerName(peer)}</span>
                <button type="button" class="icon absolute -right-2 -top-2 !h-5 !w-5 bg-[var(--card)] shadow-[var(--seg-edge)] opacity-0 transition group-hover:opacity-100 focus:opacity-100" aria-label="断开{edgeLabel[edge]}侧" disabled={busy} onclick={() => unlink(edge)}><Icon name="x" size={11} /></button>
              {:else}
                <button type="button" class="dim inline-flex items-center gap-1 hover:text-[var(--fg)]" disabled={busy} onclick={() => (picking = picking === edge ? null : edge)}><Icon name="plus" size={12} /><span class="whitespace-nowrap">{edgeLabel[edge]}侧 · 未配置</span></button>
              {/if}
              {#if picking === edge}
                <div class="menu absolute left-1/2 top-full z-30 mt-1 w-44 -translate-x-1/2" role="menu">
                  {#if trusted.length === 0}
                    <div class="dim px-2 py-1.5 text-[12px]">没有可用的已配对设备</div>
                  {:else}
                    {#each trusted as d (d.id)}
                      <button type="button" role="menuitem" class="menu-item" onclick={() => link(edge, d.id)}>
                        <span class="dot" data-state={d.online ? "online" : ""}></span>{d.name}
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
          <div class="relative flex items-center justify-center rounded-[10px] border border-[var(--line)] bg-[var(--pill)]" style="width: {box.w}px; height: {box.h}px" data-testid="layout-display">
            <div class="text-center">
              <div class="text-[12px] font-semibold">本机</div>
              <div class="dim mono text-[11px]">{data.display.width} × {data.display.height}</div>
              <div class="dim text-[10.5px]">缩放 {Math.round(data.display.scale_factor * 100)}%</div>
              {#if data.display_auto === true}
                <span class="badge badge-accent mt-1" title="分辨率由 daemon 从系统读取（§221）"><Icon name="monitor" size={10} />自动检测</span>
              {:else if data.display_auto === false}
                <!-- `undefined` = older daemon without §221: show neither badge nor the button. -->
                <span class="badge mt-1" title="分辨率已手动固定；点击恢复自动可重新跟随系统">手动设置</span>
                <button type="button" class="btn btn-sm btn-text mt-1 block w-full" disabled={busy} onclick={() => cfg({ display_auto: true })}><Icon name="refresh" size={11} />恢复自动</button>
              {/if}
            </div>
          </div>
          {@render slot("right")}

          <div></div>
          {@render slot("bottom")}
          <div></div>
        </div>
        <p class="dim m-0 mt-2 text-[11.5px]">点击边缘可选择设备，或从右侧列表拖动设备到边缘。</p>
      </div>
    </div>

    <div>
      <div class="list-title">已配对设备 <span class="acts dim">{trusted.length}</span></div>
      <div class="list">
        {#if trusted.length === 0}
          <EmptyState compact icon="link" title="还没有已配对设备" />
        {:else}
          {#each trusted as d (d.id)}
            <div class="row hov cursor-grab active:cursor-grabbing" draggable="true" ondragstart={(e) => onDragStart(e, d.id)} data-device-chip={d.id} role="listitem">
              <span class="dot" data-state={d.online ? "online" : ""}></span>
              <span class="name truncate">{d.name}</span>
              <span class="val">{EDGES.filter((e) => peerOf.get(e) === d.id).map((e) => edgeLabel[e]).join("") || ""}</span>
            </div>
          {/each}
        {/if}
      </div>
    </div>
  </div>

  <div class="grid gap-x-4 lg:grid-cols-2">
    <div>
      <div class="list-title">跨屏参数</div>
      <div class="list">
        <div class="row">
          <div class="main"><div class="name">推动阻力</div><div class="sub mono">{data.config.resistance_px} px</div></div>
          <input type="range" class="w-36" min="0" max="200" step="5" value={data.config.resistance_px} aria-busy={busy} onchange={(e) => cfg({ resistance_px: num(e) })} aria-label="推动阻力" />
        </div>
        <div class="row">
          <div class="main"><div class="name">边缘停留</div><div class="sub mono">{data.config.delay_ms} ms</div></div>
          <input type="range" class="w-36" min="0" max="1000" step="25" value={data.config.delay_ms} aria-busy={busy} onchange={(e) => cfg({ delay_ms: num(e) })} aria-label="边缘停留" />
        </div>
        <div class="row">
          <div class="main"><div class="name">角落锁定</div><div class="sub mono">{data.config.corner_lock_size_px} px</div></div>
          <input type="range" class="w-36" min="0" max="200" step="5" value={data.config.corner_lock_size_px} aria-busy={busy} onchange={(e) => cfg({ corner_lock_size_px: num(e) })} aria-label="角落锁定" />
        </div>
        <div class="row">
          <div class="main"><div class="name">远端指针速度</div><div class="sub mono">{data.config.remote_speed_percent}%</div></div>
          <input type="range" class="w-36" min="25" max="300" step="5" value={data.config.remote_speed_percent} aria-busy={busy} onchange={(e) => cfg({ remote_speed_percent: num(e) })} aria-label="远端指针速度" />
        </div>
        <div class="row">
          <div class="main"><div class="name">边缘环绕</div><div class="sub">从最后一侧推出时回到第一侧。</div></div>
          <Switch checked={data.config.wrap_around} aria-busy={busy} label="边缘环绕" onchange={(v) => cfg({ wrap_around: v })} />
        </div>
      </div>
    </div>

    <div>
      <div class="list-title">策略</div>
      <div class="list">
        <div class="row">
          <div class="main"><div class="name">启用跨屏</div><div class="sub">关闭后键鼠不会离开本机。</div></div>
          <Switch checked={data.policy.enabled} aria-busy={busy} label="启用跨屏" onchange={(v) => pol({ enabled: v })} />
        </div>
        <div class="row">
          <div class="main"><div class="name">允许被远端控制</div><div class="sub">其他设备的键鼠可以跨进本机。</div></div>
          <Switch checked={data.policy.allow_inbound_control} aria-busy={busy} label="允许被远端控制" onchange={(v) => pol({ allow_inbound_control: v })} />
        </div>
        <div class="row">
          <div class="main"><div class="name">自动连接已配对设备</div><div class="sub">新配对的设备自动占用一个空闲边缘。</div></div>
          <Switch checked={data.policy.auto_link_paired_peers} aria-busy={busy} label="自动连接已配对设备" onchange={(v) => pol({ auto_link_paired_peers: v })} />
        </div>
      </div>
    </div>
  </div>
{/if}
