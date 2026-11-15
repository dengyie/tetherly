<script lang="ts">
  import type { Settings } from "../lib/api/gen/api";

  let { data }: { data: Settings | null } = $props();
</script>

<h1>设置</h1>
<section class="card">
  <h2>常规</h2>
  {#if !data}
    <p class="dim">等待设置数据…</p>
  {:else}
    <ul>
      <li><span class="k">节点名称</span><strong>{data.general.name}</strong></li>
      <li><span class="k">mDNS 广播</span><span>{data.general.advertise ? "开" : "关"}</span></li>
      <li><span class="k">自动重连</span><span>{data.general.reconnect ? "开" : "关"}</span></li>
      <li><span class="k">仅本机回环</span><span>{data.general.loopback_only ? "开" : "关"}</span></li>
      <li><span class="k">Overlay 组网</span><span>{data.general.overlay_enabled ? "开" : "关"}</span></li>
      <li><span class="k">自动接收文件</span><span>{data.transfer.auto_accept ? "开" : "关"}</span></li>
    </ul>
    <p class="dim">只读视图；修改走 CLI（tetherly config）。</p>
  {/if}
</section>

<style>
  .card { background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius); padding: var(--pad); margin-bottom: 12px; }
  h2 { margin: 0 0 8px; font-size: 15px; }
  .dim { color: var(--text-dim); font-size: 13px; }
  ul { list-style: none; margin: 0; padding: 0; }
  li { padding: 6px 0; display: flex; gap: 16px; align-items: baseline; }
  .k { color: var(--text-dim); width: 120px; flex: none; }
</style>
