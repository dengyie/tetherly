<script lang="ts">
  import type { Layout } from "../lib/api/gen/api";

  let { data }: { data: Layout | null } = $props();

  const edgeLabel: Record<string, string> = {
    left: "左",
    right: "右",
    top: "上",
    bottom: "下",
  };
</script>

<h1>布局</h1>
<section class="card">
  <h2>本机屏幕</h2>
  {#if !data}
    <p class="dim">等待布局数据…</p>
  {:else}
    <p>{data.display.width} × {data.display.height}（缩放 {Math.round(data.display.scale_factor * 100)}%）</p>
    <p class="dim">
      策略：{data.policy.enabled ? "启用" : "停用"} ·
      入外设控制{data.policy.allow_inbound_control ? "允许" : "禁止"} ·
      自动连接已配对{data.policy.auto_link_paired_peers ? "开" : "关"}
    </p>
    <ul>
      {#each data.edges as e (e.edge)}
        <li>
          <strong>{edgeLabel[e.edge] ?? e.edge}边缘</strong>
          <span class="dim">{e.peer ? `对侧：${e.peer}` : "未配置邻居"}</span>
        </li>
      {/each}
    </ul>
    <h2>跨屏参数</h2>
    <p class="dim">
      推入阻力 {data.config.resistance_px} px · 驻留 {data.config.delay_ms} ms ·
      角落锁定 {data.config.corner_lock_size_px} px ·
      环绕{data.config.wrap_around ? "开" : "关"} ·
      远端速度 {data.config.remote_speed_percent}%
    </p>
  {/if}
</section>

<style>
  .card { background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius); padding: var(--pad); margin-bottom: 12px; }
  h2 { margin: 12px 0 8px; font-size: 15px; }
  h2:first-of-type { margin-top: 0; }
  .dim { color: var(--text-dim); }
  ul { list-style: none; margin: 0; padding: 0; }
  li { padding: 6px 0; display: flex; gap: 8px; align-items: baseline; }
</style>
