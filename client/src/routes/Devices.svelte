<script lang="ts">
  import type { Device } from "../lib/api/gen/api";
  import type { Api } from "../lib/api/client";
  import { withToast } from "../lib/ui/toast";
  import Icon from "../lib/ui/Icon.svelte";
  import PageHeader from "../lib/ui/PageHeader.svelte";
  import EmptyState from "../lib/ui/EmptyState.svelte";
  import StatusIndicator from "../lib/ui/StatusIndicator.svelte";

  let { list, api }: { list: Device[]; api: Api } = $props();
  const trusted = $derived(list.filter((d) => d.trusted && !d.blocked));
  const blocked = $derived(list.filter((d) => d.blocked));

  let busyId: string | null = $state(null);
  async function run(id: string, op: () => Promise<unknown>, ok: string) {
    busyId = id;
    await withToast(op, ok);
    busyId = null;
  }
  const connect = (d: Device) => run(d.id, () => api.connectDevice(d.id), `正在连接 ${d.name}…`);
  const disconnect = (d: Device) => run(d.id, () => api.disconnectDevice(d.id), `已断开 ${d.name}`);
  const block = (d: Device) => run(d.id, () => api.blockDevice(d.id), `已屏蔽 ${d.name}`);
  const unblock = (d: Device) => run(d.id, () => api.unblockDevice(d.id), `已解除屏蔽 ${d.name}`);
  const forget = (d: Device) => run(d.id, () => api.forgetDevice(d.id), `已忘记 ${d.name}`);

  const platformIcon = (p: string | null) => (p === "macos" ? "monitor" : p === "windows" ? "monitor" : p === "android" || p === "ios" ? "mouse" : "cpu");
</script>

<PageHeader subtitle="已配对设备的连接、屏蔽与移除" />

<div class="list-title">已配对 <span class="acts dim">{trusted.length}</span></div>
<div class="list">
  {#if trusted.length === 0}
    <EmptyState icon="link" title="还没有已配对设备" description="在另一台设备上拨号即可配对。" />
  {:else}
    {#each trusted as d (d.id)}
      <div class="row hov flex-wrap">
        <span class="ic lg"><Icon name={platformIcon(d.platform)} size={16} /></span>
        <div class="main">
          <div class="flex items-center gap-2.5">
            <span class="name">{d.name}</span>
            <StatusIndicator status={d.online ? "online" : "offline"} label={d.online ? "在线" : "离线"} pingMs={d.online ? d.rtt_ms : null} />
          </div>
          <div class="sub flex flex-wrap items-center gap-1.5">
            <span>{d.platform ?? "?"}</span>
            {#if d.online && d.path}<span>· {d.path}</span>{/if}
            {#if d.group}<span class="badge">{d.group}</span>{/if}
            {#each d.caps as c (c)}<span class="badge">{c}</span>{/each}
          </div>
        </div>
        <div class="acts">
          {#if !d.online}
            <button type="button" class="btn btn-sm btn-primary" disabled={busyId === d.id} onclick={() => connect(d)}>连接</button>
          {:else}
            <button type="button" class="btn btn-sm" disabled={busyId === d.id} onclick={() => disconnect(d)}>断开</button>
          {/if}
          <button type="button" class="btn btn-sm" disabled={busyId === d.id} onclick={() => block(d)}>屏蔽</button>
          <button type="button" class="btn btn-sm btn-text btn-danger" disabled={busyId === d.id} onclick={() => forget(d)}>忘记</button>
        </div>
      </div>
    {/each}
  {/if}
</div>

{#if blocked.length > 0}
  <div class="list-title">已屏蔽 <span class="acts dim">{blocked.length}</span></div>
  <div class="list">
    {#each blocked as d (d.id)}
      <div class="row hov">
        <span class="ic lg"><Icon name="shield-off" size={15} /></span>
        <div class="main">
          <div class="name">{d.name}</div>
          {#if d.block_reason}<div class="sub">{d.block_reason}</div>{/if}
        </div>
        <button type="button" class="btn btn-sm" disabled={busyId === d.id} onclick={() => unblock(d)}>解除屏蔽</button>
      </div>
    {/each}
  </div>
{/if}
