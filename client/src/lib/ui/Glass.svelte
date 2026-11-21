<script lang="ts">
  // Ported from awesome-ui/vanilla/LiquidGlass.js: Apple-style liquid glass
  // pane with backdrop blur, specular rim and pointer glare.
  import type { Snippet } from "svelte";
  let {
    variant = "card",
    intensity = "regular",
    interactive = true,
    enableTilt = false,
    maxTilt = 4,
    glareOpacity = 0.35,
    class: className = "",
    children,
    ...rest
  }: {
    variant?: "capsule" | "card" | "sidebar";
    intensity?: "light" | "regular" | "thick";
    interactive?: boolean;
    enableTilt?: boolean;
    maxTilt?: number;
    glareOpacity?: number;
    class?: string;
    children?: Snippet;
    [key: string]: unknown;
  } = $props();

  const base =
    "liquid-glass-pane group relative overflow-hidden text-[#1d1d1f] dark:text-[#f5f5f7] transition-all duration-300 " +
    "bg-white/65 dark:bg-[#16181d]/65 contrast-more:bg-white contrast-more:dark:bg-[#16181d] contrast-more:backdrop-blur-none " +
    "motion-reduce:transform-none motion-reduce:transition-none border border-white/50 dark:border-white/10 " +
    "shadow-[inset_0_1px_1px_0_rgba(255,255,255,0.7),inset_0_-1px_1px_0_rgba(0,0,0,0.06),0_8px_32px_rgba(0,0,0,0.06)] " +
    "dark:shadow-[inset_0_1px_1px_0_rgba(255,255,255,0.18),inset_0_-1px_1px_0_rgba(0,0,0,0.4),0_12px_40px_rgba(0,0,0,0.35)]";
  const variantCls = $derived(
    variant === "capsule"
      ? "rounded-full px-5 py-2.5 inline-flex items-center"
      : variant === "sidebar"
        ? "rounded-none border-r h-full block"
        : "rounded-2xl sm:rounded-3xl p-6 block",
  );
  const intensityCls = $derived(
    intensity === "light"
      ? "backdrop-blur-md backdrop-saturate-150"
      : intensity === "thick"
        ? "backdrop-blur-2xl backdrop-saturate-[200%]"
        : "backdrop-blur-xl backdrop-saturate-[180%]",
  );

  let glare: HTMLDivElement | undefined = $state();
  let pane: HTMLDivElement | undefined = $state();

  function onPointerMove(e: PointerEvent) {
    if (!interactive && !enableTilt) return;
    if (!pane) return;
    const rect = pane.getBoundingClientRect();
    if (!rect.width || !rect.height) return;
    const x = e.clientX - rect.left;
    const y = e.clientY - rect.top;
    const gx = ((x / rect.width) * 100).toFixed(1);
    const gy = ((y / rect.height) * 100).toFixed(1);
    if (glare && interactive) {
      glare.style.opacity = String(glareOpacity);
      glare.style.background = `radial-gradient(circle at ${gx}% ${gy}%, rgba(255,255,255,0.8) 0%, rgba(255,255,255,0.2) 35%, transparent 65%)`;
    }
    if (enableTilt && variant !== "sidebar") {
      const cx = rect.width / 2;
      const cy = rect.height / 2;
      const ry = ((x - cx) / cx) * maxTilt;
      const rx = -((y - cy) / cy) * maxTilt;
      pane.style.transform = `perspective(1000px) rotateX(${rx.toFixed(2)}deg) rotateY(${ry.toFixed(2)}deg) translateY(-1px)`;
      pane.style.transition = "transform 0.1s ease-out";
    }
  }
  function onPointerLeave() {
    if (glare) glare.style.opacity = "0";
    if (pane) {
      pane.style.transform = "";
      pane.style.transition = "transform 0.4s cubic-bezier(0.22, 1, 0.36, 1)";
    }
  }
</script>

<div
  bind:this={pane}
  class="{base} {variantCls} {intensityCls} {className}"
  onpointermove={onPointerMove}
  onpointerleave={onPointerLeave}
  {...rest}
>
  <div
    class="liquid-glass-ring pointer-events-none absolute inset-0 z-10 rounded-[inherit] ring-1 ring-inset ring-white/30 dark:ring-white/10"
    aria-hidden="true"
  ></div>
  <div
    bind:this={glare}
    class="liquid-glass-glare pointer-events-none absolute inset-0 z-10 transition-opacity duration-300"
    style="opacity:0;mix-blend-mode:overlay"
    aria-hidden="true"
  ></div>
  <div class="relative z-20 h-full">
    {@render children?.()}
  </div>
</div>
