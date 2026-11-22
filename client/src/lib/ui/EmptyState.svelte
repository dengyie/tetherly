<script lang="ts">
  // Ported from awesome-ui/vanilla/EmptyState.js; Magpie-flat styling.
  import Icon from "./Icon.svelte";
  let {
    icon = "sparkles",
    title,
    description = "",
    actionLabel = "",
    onaction,
    compact = false,
    class: className = "",
  }: {
    icon?: string;
    title: string;
    description?: string;
    actionLabel?: string;
    onaction?: () => void;
    compact?: boolean;
    class?: string;
  } = $props();
</script>

<div class="flex flex-col items-center justify-center text-center {compact ? 'px-4 py-5' : 'px-6 py-10'} {className}">
  <div class="inline-flex h-8 w-8 items-center justify-center rounded-lg bg-[var(--pill)] text-[var(--muted)] {compact ? 'mb-2' : 'mb-3'}">
    <Icon name={icon} size={compact ? 15 : 17} />
  </div>
  <div class="text-[12.5px] font-medium text-[var(--fg-2)]">{title}</div>
  {#if description}
    <div class="mt-0.5 max-w-xs text-[11.5px] text-[var(--muted)]">{description}</div>
  {/if}
  {#if actionLabel}
    <button type="button" class="btn btn-primary mt-3" onclick={() => onaction?.()}>{actionLabel}</button>
  {/if}
</div>
