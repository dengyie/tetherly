<script lang="ts">
  // Ported from awesome-ui/vanilla/StatusIndicator.js (same class structure).
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
      return {
        dot: "bg-amber-500",
        ping: "bg-amber-400",
        text: "text-amber-700 dark:text-amber-400",
        badge: "bg-amber-50 dark:bg-amber-950/40 border-amber-200/80 dark:border-amber-800/60",
        label: status === "busy" ? "Busy" : "Connecting",
      };
    }
    if (status === "error" || status === "offline") {
      return {
        dot: "bg-rose-500",
        ping: "bg-rose-400",
        text: "text-rose-700 dark:text-rose-400",
        badge: "bg-rose-50 dark:bg-rose-950/40 border-rose-200/80 dark:border-rose-800/60",
        label: status === "error" ? "Error" : "Offline",
      };
    }
    return {
      dot: "bg-emerald-500",
      ping: "bg-emerald-400",
      text: "text-emerald-700 dark:text-emerald-400",
      badge: "bg-emerald-50 dark:bg-emerald-950/40 border-emerald-200/80 dark:border-emerald-800/60",
      label: "Online",
    };
  });
</script>

<div
  class="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-full border text-xs font-medium {tone.badge} {tone.text} {className}"
  data-status={status}
>
  {#if showDot}
    <span class="relative flex h-2 w-2">
      {#if status === "online"}
        <span class="animate-ping absolute inline-flex h-full w-full rounded-full opacity-75 {tone.ping}"></span>
      {/if}
      <span class="relative inline-flex rounded-full h-2 w-2 {tone.dot}"></span>
    </span>
  {/if}
  <span class="font-semibold">{label || tone.label}</span>
  {#if pingMs !== null && pingMs !== undefined}
    <span class="font-mono text-[10px] opacity-75 ml-0.5">({pingMs}ms)</span>
  {/if}
</div>
