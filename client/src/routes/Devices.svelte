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

<PageHeader title="设备" subtitle="已配对设备的连接、屏蔽与移除" icon="monitor" />

<section class="card">
  <h2 class="card-title"><Icon name="shield" size={15} class="text-zinc-400" />已配对（{trusted.length}）</h2>
  {#if trusted.length === 0}
    <EmptyState icon="link" title="还没有已配对设备" description="在另一台设备上拨号即可配对。" />
  {:else}
    <ul class="m-0 list-none p-0">
      {#each trusted as d (d.id)}
        <li class="row flex-wrap">
          <span class="inline-flex h-9 w-9 shrink-0 items-center justify-center rounded-xl bg-zinc-100 dark:bg-zinc-800 text-zinc-500">
            <Icon name={platformIcon(d.platform)} size={17} />
          </span>
          <div class="min-w-0 flex-1">
            <div class="flex items-center gap-2">
              <span class="font-medium">{d.name}</span>
              <StatusIndicator status={d.online ? "online" : "offline"} label={d.online ? "在线" : "离线"} pingMs={d.online ? d.rtt_ms : null} />
            </div>
            <div class="mt-0.5 flex flex-wrap items-center gap-1.5 text-xs dim">
              <span>{d.platform ?? "?"}</span>
              {#if d.online && d.path}<span>· {d.path}</span>{/if}
              {#if d.group}<span class="badge">{d.group}</span>{/if}
              {#each d.caps as c (c)}<span class="badge">{c}</span>{/each}
            </div>
          </div>
          <div class="flex items-center gap-1.5">
            {#if !d.online}
              <button type="button" class="btn btn-primary btn-sm" disabled={busyId === d.id} onclick={() => connect(d)}>连接</button>
            {:else}
              <button type="button" class="btn btn-sm" disabled={busyId === d.id} onclick={() => disconnect(d)}>断开</button>
            {/if}
            <button type="button" class="btn btn-sm" disabled={busyId === d.id} onclick={() => block(d)}>屏蔽</button>
            <button type="button" class="btn btn-danger btn-sm" disabled={busyId === d.id} onclick={() => forget(d)}>忘记</button>
          </div>
        </li>
      {/each}
    </ul>
  {/if}
</section>

{#if blocked.length > 0}
  <section class="card">
    <h2 class="card-title"><Icon name="shield-off" size={15} class="text-zinc-400" />已屏蔽（{blocked.length}）</h2>
    <ul class="m-0 list-none p-0">
      {#each blocked as d (d.id)}
        <li class="row">
          <span class="inline-flex h-2 w-2 rounded-full bg-zinc-400"></span>
          <span class="font-medium">{d.name}</span>
          {#if d.block_reason}<span class="text-xs dim">{d.block_reason}</span>{/if}
          <button type="button" class="btn btn-sm ml-auto" disabled={busyId === d.id} onclick={() => unblock(d)}>解除屏蔽</button>
        </li>
      {/each}
    </ul>
  </section>
{/if}
