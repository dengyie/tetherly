<script lang="ts">
  import { onMount } from "svelte";
  import type { GestureMapDto, Settings } from "../lib/api/gen/api";
  import type { Api } from "../lib/api/client";
  import { withToast } from "../lib/ui/toast";
  import PageHeader from "../lib/ui/PageHeader.svelte";
  import Switch from "../lib/ui/Switch.svelte";
  import ThemeToggle from "../lib/ui/ThemeToggle.svelte";

  let { data, api }: { data: Settings | null; api: Api } = $props();

  // Gesture map is not part of the settings event flow; self-fetch once and
  // update in place from the PATCH response.
  let gestureMap = $state<GestureMapDto | null>(null);
  onMount(() => {
    void withToast(() => api.gestureMap().then((m) => (gestureMap = m)));
  });

  async function patchGestureMode(sticky: boolean) {
    await withToast(
      () =>
        api.patchGestureMap({ app_switch_mode: sticky ? "sticky_chord" : "per_swipe" }).then((m) => {
          gestureMap = m;
        }),
      sticky ? "已启用连续切换（Alt 保持）" : "已恢复逐次 Alt+Tab",
    );
  }

  // Local draft mirrors the event-fed store; PATCH result arrives back as a
  // settings.changed event, so edits only need to be held here between
  // submit and the round-trip.
  let nameDraft = $state("");
  let lastName = $state("");
  let saving = $state(false);
  $effect(() => {
    const n = data?.general.name ?? "";
    if (n !== lastName) {
      lastName = n;
      nameDraft = n;
    }
  });

  async function patch(p: Parameters<Api["patchSettings"]>[0], ok?: string) {
    saving = true;
    await withToast(() => api.patchSettings(p), ok);
    saving = false;
  }
  function saveName() {
    const name = nameDraft.trim();
    if (!name || name === data?.general.name) return;
    void patch({ name }, "名称已更新");
  }
</script>

<PageHeader subtitle="外观、节点名称、发现与传输策略" />

<div class="list-title">外观</div>
<div class="list">
  <div class="row">
    <div class="main"><div class="name">主题</div><div class="sub">跟随系统，或固定为浅色 / 深色。</div></div>
    <ThemeToggle />
  </div>
</div>

<div class="list-title">常规</div>
<div class="list">
  {#if !data}
    <div class="empty">等待设置数据…</div>
  {:else}
    <form class="row" onsubmit={(e) => { e.preventDefault(); saveName(); }}>
      <div class="main"><label class="name" for="node-name">节点名称</label><div class="sub">其他设备在配对与布局中看到的名字。</div></div>
      <input id="node-name" class="input w-44" bind:value={nameDraft} maxlength={64} disabled={saving} placeholder={data.general.name} />
      <button type="submit" class="btn btn-sm btn-primary" disabled={saving || !nameDraft.trim() || nameDraft.trim() === data.general.name}>保存</button>
    </form>
    <div class="row">
      <div class="main"><div class="name">mDNS 广播</div><div class="sub">在局域网内公布本机，供其他设备发现与配对。</div></div>
      <Switch checked={data.general.advertise} aria-busy={saving} label="mDNS 广播" onchange={(v) => patch({ advertise: v })} />
    </div>
    <div class="row">
      <div class="main"><div class="name">自动重连</div><div class="sub">链路中断后自动尝试恢复到已配对设备。</div></div>
      <Switch checked={data.general.reconnect} aria-busy={saving} label="自动重连" onchange={(v) => patch({ reconnect: v })} />
    </div>
    <div class="row">
      <div class="main"><div class="name">仅本机回环</div><div class="sub">由 daemon 启动参数决定，此处只读。</div></div>
      <span class="badge">{data.general.loopback_only ? "开启" : "关闭"}</span>
    </div>
    <div class="row">
      <div class="main"><div class="name">Overlay 组网</div><div class="sub">跨网段中继能力，由 daemon 配置决定。</div></div>
      <span class="badge">{data.general.overlay_enabled ? "开启" : "关闭"}</span>
    </div>
    <div class="row">
      <div class="main"><div class="name">托盘归属</div><div class="sub">当前由 {data.general.tray_owner === "client" ? "桌面客户端" : "daemon"} 持有托盘图标。</div></div>
      <span class="badge mono">{data.general.tray_owner}</span>
    </div>
  {/if}
</div>

<div class="list-title">传输</div>
<div class="list">
  {#if !data}
    <div class="empty">等待设置数据…</div>
  {:else}
    <div class="row">
      <div class="main"><div class="name">自动接收文件</div><div class="sub">来自已配对设备的文件直接写入收件箱，无需逐个确认。</div></div>
      <Switch checked={data.transfer.auto_accept} aria-busy={saving} label="自动接收文件" onchange={(v) => patch({ auto_accept: v })} />
    </div>
  {/if}
</div>

<div class="list-title">手势</div>
<div class="list">
  {#if !gestureMap}
    <div class="empty">等待手势配置…</div>
  {:else}
    <div class="row">
      <div class="main"><div class="name">四指滑动连续切换</div><div class="sub">开启后一次滑动按住 Alt，连续滑动沿同方向循环切换窗口；关闭则每次滑动直接 Alt+Tab 切一次。仅 Windows 宿主生效。</div></div>
      <Switch checked={gestureMap.app_switch_mode === "sticky_chord"} label="四指滑动连续切换" onchange={(v) => patchGestureMode(v)} />
    </div>
  {/if}
</div>
