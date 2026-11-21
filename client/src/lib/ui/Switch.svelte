<script lang="ts">
  // Small Tailwind toggle switch (role=switch) used by settings/policy rows.
  let {
    checked = false,
    disabled = false,
    label = "",
    onchange,
    class: className = "",
    ...rest
  }: {
    checked?: boolean;
    disabled?: boolean;
    label?: string;
    onchange?: (next: boolean) => void;
    class?: string;
    [key: string]: unknown;
  } = $props();
</script>

<button
  type="button"
  role="switch"
  aria-checked={checked}
  aria-label={label || undefined}
  {disabled}
  class="relative inline-flex h-5 w-9 shrink-0 items-center rounded-full border transition-colors cursor-pointer disabled:opacity-50 disabled:cursor-default focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-indigo-500/50 {checked
    ? 'bg-indigo-600 border-indigo-600'
    : 'bg-zinc-200 dark:bg-zinc-700 border-zinc-200 dark:border-zinc-700'} {className}"
  onclick={() => onchange?.(!checked)}
  {...rest}
>
  <span
    class="inline-block h-4 w-4 rounded-full bg-white shadow transition-transform {checked ? 'translate-x-4' : 'translate-x-0.5'}"
  ></span>
</button>
