<script lang="ts">
  // Ported from awesome-ui/vanilla/ThemeToggle.js; emoji replaced by UiIcon
  // (kit rule: icons only through uiIcon).
  import Icon from "./Icon.svelte";
  import { applyTheme, themeMode, type ThemeMode } from "./theme";

  let { compact = false, class: className = "" }: { compact?: boolean; class?: string } = $props();

  const modes: { mode: ThemeMode; icon: string; label: string; title: string }[] = [
    { mode: "auto", icon: "monitor", label: "自动", title: "跟随系统" },
    { mode: "light", icon: "sun", label: "浅色", title: "浅色模式" },
    { mode: "dark", icon: "moon", label: "深色", title: "深色模式" },
  ];
</script>

<div
  class="inline-flex items-center p-1 rounded-xl bg-zinc-100 dark:bg-zinc-800/80 border border-zinc-200/80 dark:border-zinc-700/80 text-xs font-medium text-zinc-500 dark:text-zinc-400 select-none {className}"
  role="group"
  aria-label="主题切换"
>
  {#each modes as m (m.mode)}
    <button
      type="button"
      data-mode={m.mode}
      title={m.title}
      aria-pressed={$themeMode === m.mode}
      class="flex items-center gap-1.5 px-2.5 py-1 rounded-lg transition-all cursor-pointer {$themeMode === m.mode
        ? 'bg-white dark:bg-zinc-900 text-zinc-900 dark:text-zinc-100 shadow-xs font-semibold'
        : 'hover:text-zinc-900 dark:hover:text-zinc-100'}"
      onclick={() => applyTheme(m.mode)}
    >
      <Icon name={m.icon} size={14} />
      {#if !compact}<span>{m.label}</span>{/if}
    </button>
  {/each}
</div>
