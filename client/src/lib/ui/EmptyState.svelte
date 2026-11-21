<script lang="ts">
  // Ported from awesome-ui/vanilla/EmptyState.js.
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

<div class="flex flex-col items-center justify-center text-center {compact ? 'px-4 py-6' : 'px-6 py-12'} {className}">
  <div class="p-3 rounded-2xl bg-zinc-100 dark:bg-zinc-800 text-zinc-400 dark:text-zinc-500 {compact ? 'mb-2' : 'mb-4'}">
    <Icon name={icon} size={compact ? 18 : 24} />
  </div>
  <div class="text-sm font-semibold text-zinc-800 dark:text-zinc-200">{title}</div>
  {#if description}
    <div class="mt-1 text-xs text-zinc-400 dark:text-zinc-500 max-w-xs">{description}</div>
  {/if}
  {#if actionLabel}
    <button
      type="button"
      class="mt-4 px-3.5 py-1.5 rounded-lg bg-indigo-600 hover:bg-indigo-500 text-white text-xs font-medium transition-colors cursor-pointer"
      onclick={() => onaction?.()}
    >
      {actionLabel}
    </button>
  {/if}
</div>
