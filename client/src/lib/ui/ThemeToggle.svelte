<script lang="ts">
  // Magpie's 外觀 segmented control (跟隨系統 / 淺色 / 深色); icons via UiIcon.
  import Icon from "./Icon.svelte";
  import { applyTheme, themeMode, type ThemeMode } from "./theme";

  let { compact = false, class: className = "" }: { compact?: boolean; class?: string } = $props();

  const modes: { mode: ThemeMode; icon: string; label: string; title: string }[] = [
    { mode: "auto", icon: "monitor", label: "跟随系统", title: "跟随系统" },
    { mode: "light", icon: "sun", label: "浅色", title: "浅色模式" },
    { mode: "dark", icon: "moon", label: "深色", title: "深色模式" },
  ];
</script>

<div class="seg {compact ? 'sm' : ''} {className}" role="group" aria-label="主题切换">
  {#each modes as m (m.mode)}
    <button
      type="button"
      data-mode={m.mode}
      title={m.title}
      aria-pressed={$themeMode === m.mode}
      class={$themeMode === m.mode ? "on" : ""}
      onclick={() => applyTheme(m.mode)}
    >
      {#if compact}<Icon name={m.icon} size={13} />{:else}{m.label}{/if}
    </button>
  {/each}
</div>
