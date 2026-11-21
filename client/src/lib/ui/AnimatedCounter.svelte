<script lang="ts">
  // Ported from awesome-ui/vanilla/AnimatedCounter.js: eases from the last
  // shown value to `target` whenever target changes (store-driven stats).
  let {
    target,
    duration = 700,
    prefix = "",
    suffix = "",
    class: className = "",
  }: { target: number; duration?: number; prefix?: string; suffix?: string; class?: string } = $props();

  const easeOutCubic = (t: number) => 1 - Math.pow(1 - t, 3);
  let shown = $state(0);
  let raf = 0;

  $effect(() => {
    const to = Number.isFinite(target) ? target : 0;
    const from = shown;
    if (typeof requestAnimationFrame !== "function" || duration <= 0 || from === to) {
      shown = to;
      return;
    }
    const t0 = performance.now();
    const tick = (now: number) => {
      const p = Math.min(1, (now - t0) / duration);
      shown = Math.round(from + easeOutCubic(p) * (to - from));
      if (p < 1) raf = requestAnimationFrame(tick);
      else shown = to;
    };
    cancelAnimationFrame(raf);
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  });
</script>

<span class="font-mono tabular-nums select-none {className}">{prefix}{shown}{suffix}</span>
