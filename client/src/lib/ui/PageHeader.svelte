<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon from "./Icon.svelte";
  let {
    title,
    subtitle = "",
    icon = "",
    children,
  }: { title: string; subtitle?: string; icon?: string; children?: Snippet } = $props();
</script>

<div class="flex items-start gap-3 pb-1">
  {#if icon}
    <span class="mt-0.5 inline-flex h-9 w-9 shrink-0 items-center justify-center rounded-xl bg-brand-500/10 text-brand-600 dark:text-brand-400">
      <Icon name={icon} size={18} />
    </span>
  {/if}
  <div class="min-w-0 flex-1">
    <h1 class="m-0 text-lg font-semibold tracking-tight leading-7">{title}</h1>
    {#if subtitle}<p class="m-0 text-xs text-zinc-500 dark:text-zinc-400">{subtitle}</p>{/if}
  </div>
  {#if children}
    <div class="flex items-center gap-2">{@render children()}</div>
  {/if}
</div>
