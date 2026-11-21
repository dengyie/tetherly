<script lang="ts">
  // Ported from awesome-ui/vanilla/CommandPalette.js: Esc / arrows / Enter,
  // grouped items, fuzzy-ish substring filter over label/hint/group.
  import Icon from "./Icon.svelte";
  import { tick } from "svelte";

  export type PaletteItem = { id: string; label: string; hint?: string; group?: string; icon?: string; shortcut?: string };

  let {
    open = $bindable(false),
    items = [],
    placeholder = "输入命令或搜索…",
    onselect,
    onclose,
  }: {
    open?: boolean;
    items?: PaletteItem[];
    placeholder?: string;
    onselect?: (item: PaletteItem) => void;
    onclose?: () => void;
  } = $props();

  let query = $state("");
  let active = $state(0);
  let input: HTMLInputElement | undefined = $state();

  const filtered = $derived.by(() => {
    const q = query.trim().toLowerCase();
    if (!q) return items;
    return items.filter(
      (i) =>
        i.label.toLowerCase().includes(q) ||
        (i.hint ?? "").toLowerCase().includes(q) ||
        (i.group ?? "").toLowerCase().includes(q),
    );
  });

  $effect(() => {
    if (open) {
      query = "";
      active = 0;
      void tick().then(() => input?.focus());
    }
  });

  function close() {
    open = false;
    onclose?.();
  }
  function select(item: PaletteItem) {
    onselect?.(item);
    close();
  }
  function onKey(e: KeyboardEvent) {
    if (!open) return;
    if (e.key === "Escape") {
      e.preventDefault();
      close();
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      active = Math.min(filtered.length - 1, active + 1);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      active = Math.max(0, active - 1);
    } else if (e.key === "Enter") {
      e.preventDefault();
      const item = filtered[active];
      if (item) select(item);
    }
  }
</script>

<svelte:document onkeydown={onKey} />

{#if open}
  <div
    class="fixed inset-0 z-50 flex items-start justify-center pt-[15vh] bg-black/40 backdrop-blur-sm"
    role="dialog"
    aria-modal="true"
    aria-label="命令面板"
    data-testid="command-palette"
    tabindex="-1"
    onclick={(e) => {
      if (e.target === e.currentTarget) close();
    }}
    onkeydown={(e) => {
      if (e.key === "Escape") close();
    }}
  >
    <div
      class="w-full max-w-lg mx-4 rounded-2xl border border-zinc-200 dark:border-zinc-700 bg-white dark:bg-zinc-900 shadow-2xl overflow-hidden"
    >
      <div class="flex items-center gap-2.5 px-4 py-3 border-b border-zinc-100 dark:border-zinc-800">
        <Icon name="search" size={16} class="text-zinc-400" />
        <input
          bind:this={input}
          bind:value={query}
          oninput={() => (active = 0)}
          type="text"
          class="w-full bg-transparent border-none text-sm text-zinc-800 dark:text-zinc-200 placeholder-zinc-400 focus:outline-none"
          {placeholder}
          aria-label={placeholder}
        />
        <kbd class="hidden sm:inline-block kbd">ESC</kbd>
      </div>
      <div class="max-h-72 overflow-y-auto p-1.5" role="listbox">
        {#if filtered.length === 0}
          <div class="p-4 text-center text-sm text-zinc-400">没有匹配结果</div>
        {:else}
          {#each filtered as item, idx (item.id)}
            {#if item.group && (idx === 0 || filtered[idx - 1].group !== item.group)}
              <div class="px-2.5 pt-2.5 pb-1 text-[10px] uppercase font-semibold tracking-wider text-zinc-400">{item.group}</div>
            {/if}
            <button
              type="button"
              role="option"
              aria-selected={idx === active}
              class="w-full flex items-center gap-3 px-2.5 py-2 rounded-lg text-left text-sm transition-colors cursor-pointer {idx === active
                ? 'bg-zinc-100 dark:bg-zinc-800 text-zinc-900 dark:text-zinc-100'
                : 'text-zinc-700 dark:text-zinc-300'}"
              onmouseenter={() => (active = idx)}
              onclick={() => select(item)}
            >
              {#if item.icon}
                <span class="p-1 rounded-md bg-zinc-200/60 dark:bg-zinc-700/60 text-zinc-500 dark:text-zinc-400 shrink-0">
                  <Icon name={item.icon} size={14} />
                </span>
              {/if}
              <span class="flex-1 min-w-0">
                <span class="block truncate font-medium">{item.label}</span>
                {#if item.hint}<span class="block truncate text-[11px] text-zinc-400">{item.hint}</span>{/if}
              </span>
              {#if item.shortcut}<kbd class="kbd shrink-0">{item.shortcut}</kbd>{/if}
            </button>
          {/each}
        {/if}
      </div>
    </div>
  </div>
{/if}
