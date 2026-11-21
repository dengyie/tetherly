<script lang="ts">
  import type { Settings } from "../lib/api/gen/api";
  import type { Api } from "../lib/api/client";
  import { withToast } from "../lib/ui/toast";
  import Icon from "../lib/ui/Icon.svelte";
  import PageHeader from "../lib/ui/PageHeader.svelte";
  import Switch from "../lib/ui/Switch.svelte";

  let { data, api }: { data: Settings | null; api: Api } = $props();

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

<PageHeader title="设置" subtitle="节点名称、发现与传输策略" icon="settings" />

<section class="card">
  <h2 class="card-title"><Icon name="sliders" size={15} class="text-zinc-400" />常规</h2>
  {#if !data}
    <p class="m-0 dim">等待设置数据…</p>
  {:else}
    <form class="flex flex-wrap items-center gap-2" onsubmit={(e) => { e.preventDefault(); saveName(); }}>
      <label class="text-xs dim w-24" for="node-name">节点名称</label>
      <input id="node-name" class="input flex-1 min-w-40 max-w-xs" bind:value={nameDraft} maxlength={64} disabled={saving} placeholder={data.general.name} />
      <button type="submit" class="btn btn-primary" disabled={saving || !nameDraft.trim() || nameDraft.trim() === data.general.name}>保存</button>
    </form>
    <ul class="m-0 mt-3 list-none p-0">
      <li class="row">
        <div class="flex-1"><div class="text-[13px]">mDNS 广播</div><div class="text-xs dim">在局域网内公布本机，供其他设备发现与配对。</div></div>
        <Switch checked={data.general.advertise} aria-busy={saving} label="mDNS 广播" onchange={(v) => patch({ advertise: v })} />
      </li>
      <li class="row">
        <div class="flex-1"><div class="text-[13px]">自动重连</div><div class="text-xs dim">链路中断后自动尝试恢复到已配对设备。</div></div>
        <Switch checked={data.general.reconnect} aria-busy={saving} label="自动重连" onchange={(v) => patch({ reconnect: v })} />
      </li>
      <li class="row">
        <div class="flex-1"><div class="text-[13px]">仅本机回环</div><div class="text-xs dim">由 daemon 启动参数决定，此处只读。</div></div>
        <span class="badge">{data.general.loopback_only ? "开" : "关"}</span>
      </li>
      <li class="row">
        <div class="flex-1"><div class="text-[13px]">Overlay 组网</div><div class="text-xs dim">跨网段中继能力，由 daemon 配置决定。</div></div>
        <span class="badge">{data.general.overlay_enabled ? "开" : "关"}</span>
      </li>
      <li class="row">
        <div class="flex-1"><div class="text-[13px]">托盘归属</div><div class="text-xs dim">当前由 {data.general.tray_owner === "client" ? "桌面客户端" : "daemon"} 持有托盘图标。</div></div>
        <span class="badge mono">{data.general.tray_owner}</span>
      </li>
    </ul>
  {/if}
</section>

<section class="card">
  <h2 class="card-title"><Icon name="download" size={15} class="text-zinc-400" />传输</h2>
  {#if !data}
    <p class="m-0 dim">等待设置数据…</p>
  {:else}
    <ul class="m-0 list-none p-0">
      <li class="row">
        <div class="flex-1"><div class="text-[13px]">自动接收文件</div><div class="text-xs dim">来自已配对设备的文件直接写入收件箱，无需逐个确认。</div></div>
        <Switch checked={data.transfer.auto_accept} aria-busy={saving} label="自动接收文件" onchange={(v) => patch({ auto_accept: v })} />
      </li>
    </ul>
  {/if}
</section>
