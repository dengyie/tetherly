<script lang="ts">
  // Ported from awesome-ui/vanilla/ToastStack.js (fixed bottom-right, w-80).
  import Icon from "./Icon.svelte";
  import { dismissToast, toasts, type ToastType } from "./toast";

  const META: Record<ToastType, { icon: string; color: string }> = {
    success: { icon: "check-circle", color: "text-emerald-500" },
    error: { icon: "circle-x", color: "text-rose-500" },
    warning: { icon: "alert-triangle", color: "text-amber-500" },
    info: { icon: "sparkles", color: "text-sky-500" },
  };
</script>

{#if $toasts.length > 0}
  <div class="fixed bottom-4 right-4 z-50 flex flex-col gap-2 w-80 max-w-[calc(100vw-2rem)]" aria-live="polite" data-testid="toast-stack">
    {#each $toasts as t (t.id)}
      <div
        class="flex items-center gap-2.5 px-3.5 py-2.5 rounded-xl border border-zinc-200 dark:border-zinc-700 bg-white dark:bg-zinc-900 shadow-lg text-sm text-zinc-800 dark:text-zinc-200"
        role="alert"
        data-type={t.type}
      >
        <Icon name={META[t.type].icon} size={16} class={META[t.type].color} />
        <span class="flex-1 min-w-0 break-words">{t.message}</span>
        <button
          type="button"
          class="text-zinc-400 hover:text-zinc-600 dark:hover:text-zinc-200 transition-colors shrink-0 cursor-pointer"
          aria-label="关闭提示"
          onclick={() => dismissToast(t.id)}
        >
          <Icon name="x" size={14} />
        </button>
      </div>
    {/each}
  </div>
{/if}
