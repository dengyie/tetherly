<script lang="ts">
  // Svelte wrapper over the ported awesome-ui `uiIcon()` string renderer.
  import { uiIcon } from "./icons";
  let {
    name,
    size = 18,
    strokeWidth = 1.8,
    label = "",
    class: className = "",
  }: { name: string; size?: number; strokeWidth?: number; label?: string; class?: string } = $props();
  const html = $derived(uiIcon(name, { size, strokeWidth, label, className: `shrink-0 ${className}`.trim() }));
</script>

{@html html}
