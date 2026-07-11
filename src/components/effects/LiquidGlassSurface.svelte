<script lang="ts">
  import { onMount } from "svelte";
  import type { LiquidGlassOptions, LiquidGlassParams } from "liquid-glass-js";

  type LiquidGlassInstance = {
    set: (partial: Partial<LiquidGlassParams>) => LiquidGlassInstance;
    moveTo: (x: number, y: number) => LiquidGlassInstance;
    setBackground: (el: Element | null) => LiquidGlassInstance;
    refresh: () => LiquidGlassInstance;
    destroy: () => void;
    glassEl?: HTMLElement;
    lensEl?: HTMLElement;
  };

  type LiquidGlassConstructor = new (options?: LiquidGlassOptions) => LiquidGlassInstance;
  type GlassVariant = "liquid" | "immersive";

  let {
    target = null,
    backgroundSelector = "#listen1-glass-scene",
    enabled = false,
    variant = "liquid",
    zIndex = 126,
  }: {
    target?: HTMLElement | null;
    backgroundSelector?: string;
    enabled?: boolean;
    variant?: GlassVariant;
    zIndex?: number;
  } = $props();

  let mounted = false;
  let ctor: LiquidGlassConstructor | null = null;
  let glass: LiquidGlassInstance | null = null;
  let resizeObserver: ResizeObserver | null = null;
  let observedTarget: HTMLElement | null = null;
  let raf = 0;
  let lastVariant: GlassVariant | null = null;
  let lastBackground: Element | null = null;
  let lastRect = { x: -1, y: -1, width: -1, height: -1, radius: -1 };

  const VARIANT_PARAMS: Record<GlassVariant, Partial<LiquidGlassParams>> = {
    liquid: {
      scale: 28,
      depth: 30,
      curvature: 3.8,
      convexity: 1,
      chroma: 0.06,
      blur: 0,
      glow: 0.08,
      edge: 0.48,
      specAngle: 132,
      tint: 0.06,
      tintColor: "#ffffff",
    },
    immersive: {
      scale: 42,
      depth: 36,
      curvature: 3.2,
      convexity: 1,
      chroma: 0.14,
      blur: 0,
      glow: 0.16,
      edge: 0.62,
      specAngle: 136,
      tint: 0.04,
      tintColor: "#00f5d4",
    },
  };

  onMount(() => {
    mounted = true;
    scheduleSync();
    return cleanup;
  });

  $effect(() => {
    if (!mounted) return;
    enabled;
    target;
    variant;
    backgroundSelector;
    scheduleSync();
  });

  function radiusFor(rect: DOMRect) {
    if (variant === "immersive") return Math.min(50, rect.height / 2);
    return Math.min(18, rect.height / 2);
  }

  function scheduleSync() {
    cancelAnimationFrame(raf);
    raf = requestAnimationFrame(() => {
      raf = 0;
      void syncGlass();
    });
  }

  async function syncGlass() {
    if (!enabled || !target) {
      cleanupGlass();
      return;
    }

    const rect = target.getBoundingClientRect();
    if (rect.width < 8 || rect.height < 8) {
      cleanupGlass();
      return;
    }

    if (!ctor) {
      const module = await import("liquid-glass-js");
      ctor = module.default as LiquidGlassConstructor;
      if (!enabled || !target) return;
    }

    const background = document.querySelector(backgroundSelector);
    const radius = radiusFor(rect);
    const params = VARIANT_PARAMS[variant];

    if (!glass) {
      glass = new ctor({
        background,
        draggable: false,
        zIndex,
        x: rect.left,
        y: rect.top,
        width: rect.width,
        height: rect.height,
        radius,
        ...params,
      });
      glass.glassEl?.classList.add("listen1-liquid-glass");
      glass.lensEl?.classList.add("listen1-liquid-glass-lens");
      lastVariant = variant;
      lastBackground = background;
      lastRect = { x: rect.left, y: rect.top, width: rect.width, height: rect.height, radius };
    } else {
      if (background !== lastBackground) {
        glass.setBackground(background);
        lastBackground = background;
      }
      if (variant !== lastVariant) {
        glass.set({ ...params, radius });
        lastVariant = variant;
      }
      updateGeometry(rect, radius);
    }

    observeTarget();
  }

  function updateGeometry(rect: DOMRect, radius = radiusFor(rect)) {
    if (!glass) return;

    const next = {
      x: rect.left,
      y: rect.top,
      width: Math.round(rect.width),
      height: Math.round(rect.height),
      radius,
    };
    const needsMap =
      Math.abs(next.width - lastRect.width) > 1 ||
      Math.abs(next.height - lastRect.height) > 1 ||
      Math.abs(next.radius - lastRect.radius) > 1;

    if (needsMap) {
      glass.set({ width: next.width, height: next.height, radius: next.radius });
    }

    if (Math.abs(next.x - lastRect.x) > 0.5 || Math.abs(next.y - lastRect.y) > 0.5) {
      glass.moveTo(next.x, next.y);
    }

    lastRect = next;
  }

  function observeTarget() {
    if (observedTarget === target) return;
    resizeObserver?.disconnect();
    observedTarget = target;
    if (!target) return;
    resizeObserver = new ResizeObserver(() => scheduleSync());
    resizeObserver.observe(target);
  }

  function cleanupGlass() {
    resizeObserver?.disconnect();
    resizeObserver = null;
    observedTarget = null;
    lastBackground = null;
    lastVariant = null;
    lastRect = { x: -1, y: -1, width: -1, height: -1, radius: -1 };
    if (glass) {
      glass.destroy();
      glass = null;
    }
  }

  function cleanup() {
    mounted = false;
    cancelAnimationFrame(raf);
    cleanupGlass();
  }
</script>
