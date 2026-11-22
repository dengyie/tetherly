<script lang="ts">
  // Magpie layout (§224): the page title lives in the shell's breadcrumb
  // (`main h1`), so this row only carries the one-line description and the
  // page's actions. Renders nothing when it has neither.
  import type { Snippet } from "svelte";
  // `title` / `icon` stay in the prop contract for callers but are not read here.
  let { subtitle = "", children }: { title?: string; subtitle?: string; icon?: string; children?: Snippet } = $props();
</script>

{#if subtitle || children}
  <div class="page-intro">
    <p>{subtitle}</p>
    {#if children}
      <div class="flex items-center gap-2">{@render children()}</div>
    {/if}
  </div>
{/if}
