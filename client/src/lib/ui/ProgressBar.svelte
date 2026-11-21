<script lang="ts">
  // Ported from awesome-ui/vanilla/StreamingProgressBar.js.
  let {
    value = null,
    label = "",
    status = "streaming",
    showPercent = true,
    class: className = "",
  }: {
    value?: number | null;
    label?: string;
    status?: "streaming" | "done" | "error";
    showPercent?: boolean;
    class?: string;
  } = $props();

  const BAR: Record<string, string> = { streaming: "bg-brand-500", done: "bg-emerald-500", error: "bg-rose-500" };
  const indeterminate = $derived(value === null || value === undefined || Number.isNaN(value));
  const clamped = $derived(indeterminate ? 0 : Math.min(100, Math.max(0, value as number)));
  const bar = $derived(BAR[status] ?? BAR.streaming);
</script>

<div class="block w-full {className}">
  {#if label || (!indeterminate && showPercent)}
    <div class="flex items-center justify-between mb-1 text-[11px] font-mono text-zinc-400">
      <span class="truncate">{label}</span>
      {#if !indeterminate && showPercent}<span class="tabular-nums shrink-0">{Math.round(clamped)}%</span>{/if}
    </div>
  {/if}
  <div
    role="progressbar"
    aria-valuemin="0"
    aria-valuemax="100"
    aria-valuenow={indeterminate ? undefined : Math.round(clamped)}
    aria-label={label || "Progress"}
    class="h-1 w-full rounded-full bg-zinc-200 dark:bg-zinc-800 overflow-hidden"
  >
    {#if indeterminate}
      <div class="h-full w-1/3 rounded-full {bar} animate-pulse"></div>
    {:else}
      <div class="h-full rounded-full transition-all duration-300 {bar}" style="width: {clamped}%"></div>
    {/if}
  </div>
</div>
