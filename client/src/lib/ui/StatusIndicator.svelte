<script lang="ts">
  // Ported from awesome-ui/vanilla/StatusIndicator.js, restyled to Magpie's
  // dot + text status line (the tone class names keep the kit's contract).
  export type Status = "online" | "connecting" | "busy" | "error" | "offline";
  let {
    status = "online",
    label = "",
    pingMs = null,
    showDot = true,
    class: className = "",
  }: { status?: Status; label?: string; pingMs?: number | null; showDot?: boolean; class?: string } = $props();

  const tone = $derived.by(() => {
    if (status === "connecting" || status === "busy") {
      return { dot: "bg-amber-500", text: "text-amber-700 dark:text-amber-400", label: status === "busy" ? "Busy" : "Connecting" };
    }
    if (status === "error" || status === "offline") {
      return { dot: "bg-rose-500", text: "text-rose-600 dark:text-rose-400", label: status === "error" ? "Error" : "Offline" };
    }
    return { dot: "bg-emerald-500", text: "text-emerald-700 dark:text-emerald-400", label: "Online" };
  });
</script>

<span class="inline-flex items-center gap-1.5 text-[12px] font-medium {tone.text} {className}" data-status={status}>
  {#if showDot}
    <span class="inline-flex h-[7px] w-[7px] rounded-full {tone.dot}"></span>
  {/if}
  <span>{label || tone.label}</span>
  {#if pingMs !== null && pingMs !== undefined}
    <span class="mono text-[10.5px] opacity-70">({pingMs}ms)</span>
  {/if}
</span>
