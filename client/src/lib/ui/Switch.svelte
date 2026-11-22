<script lang="ts">
  // Magpie-style iOS toggle (role=switch) used by settings/policy rows.
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
  class="toggle {checked ? 'on' : ''} {className}"
  onclick={() => onchange?.(!checked)}
  {...rest}
></button>
