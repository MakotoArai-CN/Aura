<script lang="ts">
  /**
   * iOS 26 风格液态玻璃表面。
   *
   * 与旧实现（liquid-glass-js）的区别：旧版把背景 DOM `cloneNode(true)` 出来再套
   * `filter: url()`，等于把整个主视图复制一份，而且克隆是静态快照——滚动或换歌后
   * 玻璃里是过期像素。这里改成 Chromium 原生的 `backdrop-filter: url(#id)`：
   * 采样的是实时 backdrop，没有克隆、没有 rAF 轮询，位移贴图只在尺寸变化时重算。
   *
   * 光学模型沿用 shuding/liquid-glass 的思路（圆角矩形 SDF + 边缘折射带），
   * 但位移只集中在边缘 bevel 内、中心保持平整，再叠加 blur 作为“中心磨砂”——
   * 边缘折射 + 中心磨砂两者同时存在才是 iOS 26 的观感，只有其中之一都不像。
   */
  import { deviceTier } from "../../lib/stores/device";

  let {
    target = null,
    enabled = false,
  }: {
    target?: HTMLElement | null;
    enabled?: boolean;
  } = $props();

  type Preset = {
    /** 边缘最大位移（CSS px），即 feDisplacementMap 的 scale */
    displace: number;
    /** 折射带宽度（CSS px），带内渐变、带外完全平整 */
    bevel: number;
    /** 折射衰减指数，越大越贴边 */
    curvature: number;
    blur: number;
    saturate: number;
    brightness: number;
    tint: number;
    sheen: number;
  };

  // 参数在 Chromium 里实测调过：位移必须排在 blur 之前，且 blur 不能太大——
  // 先位移后模糊会把折射带的细节抹掉，blur 10px 以上边缘就基本看不出折射了。
  const BASE_PRESET: Preset = {
    displace: 26,
    bevel: 16,
    curvature: 2.4,
    blur: 8,
    saturate: 180,
    brightness: 1.06,
    tint: 0.1,
    sheen: 0.18,
  };

  /**
   * ultra 档：折射带更宽、位移更大、高光更明显。开销全在 backdrop-filter 的采样面积
   * 上，只有用户手动选到极致才会用；自动策略最高到 high，用的仍是 BASE_PRESET。
   */
  const ULTRA_PRESET: Preset = {
    displace: 34,
    bevel: 22,
    curvature: 2.1,
    blur: 10,
    saturate: 200,
    brightness: 1.09,
    tint: 0.13,
    sheen: 0.24,
  };

  let preset = $derived($deviceTier === "ultra" ? ULTRA_PRESET : BASE_PRESET);

  // 贴图像素上限：footer 这种 1400x100 用不满，超大面积自动降采样后由 feImage 拉伸。
  const MAX_MAP_PIXELS = 240_000;

  /**
   * 位移贴图按几何缓存（模块级，跨挂载存活）。
   *
   * 底部播放器展开 ⇄ 收起会反复回到同一组尺寸，命中缓存就能在同一帧同步贴上正确
   * 的贴图，不用等「算图 + 解码」——那一帧的空档正是收起最后一步闪一下的成因。
   * 尺寸量化后再做 key，避免窗口被拖动 1px 就换一张图。
   */
  const MAP_QUANT_PX = 4;
  /** 收起/展开两组尺寸 × 两档预设，再留点余量。 */
  const MAP_CACHE_LIMIT = 6;
  /** 尺寸稳定多久才真正重算。动画期间每帧都算会掉帧。 */
  const MAP_SETTLE_MS = 140;
  const mapCache = new Map<string, string>();

  function quantizeSize(value: number) {
    return Math.max(MAP_QUANT_PX, Math.round(value / MAP_QUANT_PX) * MAP_QUANT_PX);
  }

  function mapCacheKey(w: number, h: number, r: number, p: Preset) {
    // 档位不同 → 折射带几何不同 → 必须是不同的 key，否则从 high 切到 ultra 会
    // 命中上一档算好的贴图，滑条动了但边缘纹理没变。只有这三个参数进贴图，
    // 其余（blur/saturate/tint/sheen）走 CSS 变量，与贴图无关。
    return `${quantizeSize(w)}x${quantizeSize(h)}x${Math.round(r)}@${p.displace}-${p.bevel}-${p.curvature}`;
  }

  /** 写入并把最近使用的挪到末尾，超限时淘汰最旧的一张。 */
  function rememberMap(key: string, url: string) {
    mapCache.delete(key);
    mapCache.set(key, url);
    while (mapCache.size > MAP_CACHE_LIMIT) {
      const oldest = mapCache.keys().next().value;
      if (oldest === undefined) break;
      mapCache.delete(oldest);
    }
  }

  function mostRecentMap(): string | undefined {
    let last: string | undefined;
    for (const url of mapCache.values()) last = url;
    return last;
  }

  const filterId = `lqg-${Math.random().toString(36).slice(2, 9)}`;

  let width = $state(0);
  let height = $state(0);
  let radius = $state(0);
  let mapUrl = $state("");
  /**
   * 是否已经有贴图在用。刻意不是 $state：贴图效果自己要读它来决定是否借用一张，
   * 如果它是响应式的，这次读 + 这次写就会让效果多跑一轮、把去抖计时器重置掉。
   */
  let hasMap = false;

  let active = $derived(enabled && width >= 8 && height >= 8);

  $effect(() => {
    const el = target;
    if (!el || !enabled) {
      width = 0;
      height = 0;
      mapUrl = "";
      hasMap = false;
      return;
    }

    const read = () => {
      // clientWidth/Height 是 padding box，和绝对定位子元素 inset:0 的参照系一致
      width = el.clientWidth;
      height = el.clientHeight;
      const style = getComputedStyle(el);
      const outer = Number.parseFloat(style.borderTopLeftRadius) || 0;
      const border = Number.parseFloat(style.borderLeftWidth) || 0;
      radius = Math.max(0, outer - border);
    };

    read();
    const observer = new ResizeObserver(read);
    observer.observe(el);
    return () => observer.disconnect();
  });

  $effect(() => {
    if (!active) return;
    const w = width;
    const h = height;
    const r = radius;
    // 捕成局部量：下面的 setTimeout 要用和 key 完全同一套参数算图，
    // 直接在回调里读 preset 会在「等待期间用户又动了滑条」时算出对不上 key 的图。
    const p = preset;
    const key = mapCacheKey(w, h, r, p);

    // 精确命中：同一帧同步换上。收起动画的最后一帧因此不会出现「没有贴图」的空档，
    // 也不会触发 .ready 的 backdrop-filter 切换——那一下切换就是肉眼看到的闪动。
    const exact = mapCache.get(key);
    if (exact !== undefined) {
      mapUrl = exact;
      hasMap = true;
      return;
    }

    // 没有精确匹配、且当前还一张都没有（刚挂载）：先借用最近算过的一张。
    // feImage 是 preserveAspectRatio="none"，会拉伸铺满，折射带宽度略有偏差但形体
    // 完整。借一张的意义在于 .ready 从第一帧就成立，backdrop-filter 不必等算完再换。
    if (!hasMap) {
      const fallback = mostRecentMap();
      if (fallback !== undefined) {
        mapUrl = fallback;
        hasMap = true;
      }
    }

    // 真正重算含嵌套像素循环 + toDataURL，动画期间每帧都算必然掉帧，
    // 所以等尺寸稳定下来只算一次；这期间沿用上面那张（拉伸，不会消失）。
    let cancelled = false;
    const timer = setTimeout(() => {
      const url = buildDisplacementMap(w, h, r, p);
      if (!url) return;
      rememberMap(key, url);
      // 先解码再换 href：直接换的话 feImage 要异步取这张 data URI，
      // 这期间 feDisplacementMap 的 in2 是空的，画面同样会闪一下。
      const image = new Image();
      const apply = () => {
        if (cancelled) return;
        mapUrl = url;
        hasMap = true;
      };
      image.onload = apply;
      image.onerror = apply;
      image.src = url;
      if (image.complete) apply();
    }, MAP_SETTLE_MS);

    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  });

  /** 圆角矩形有符号距离场，中心在原点，内部为负 */
  function roundedRectSDF(px: number, py: number, hw: number, hh: number, r: number) {
    const qx = Math.abs(px) - hw + r;
    const qy = Math.abs(py) - hh + r;
    const ax = Math.max(qx, 0);
    const ay = Math.max(qy, 0);
    return Math.hypot(ax, ay) + Math.min(Math.max(qx, qy), 0) - r;
  }

  function toByte(value: number) {
    return value < 0 ? 0 : value > 255 ? 255 : value | 0;
  }

  /**
   * 生成位移贴图：R/G 编码每个像素的采样偏移（128 为中性）。
   * feDisplacementMap 按 scale*(C-0.5) 取偏移，所以这里存归一化值，量级交给 scale。
   */
  function buildDisplacementMap(w: number, h: number, r: number, p: Preset) {
    const quality = Math.min(1, Math.sqrt(MAX_MAP_PIXELS / Math.max(1, w * h)));
    const cw = Math.max(2, Math.round(w * quality));
    const ch = Math.max(2, Math.round(h * quality));

    const canvas = document.createElement("canvas");
    canvas.width = cw;
    canvas.height = ch;
    const ctx = canvas.getContext("2d");
    if (!ctx) return "";

    const image = ctx.createImageData(cw, ch);
    const data = image.data;
    const hw = cw / 2;
    const hh = ch / 2;
    const rr = Math.min(r * quality, Math.min(hw, hh));
    const bevel = Math.max(1, Math.min(p.bevel * quality, Math.min(hw, hh) - 0.5));
    const epsilon = Math.max(0.5, quality);

    for (let y = 0; y < ch; y++) {
      const py = y + 0.5 - hh;
      for (let x = 0; x < cw; x++) {
        const px = x + 0.5 - hw;
        const distance = roundedRectSDF(px, py, hw, hh, rr);
        // distance ∈ [-bevel, 0] 映射到 [0, 1]，带外为 0
        const t = distance <= -bevel ? 0 : distance >= 0 ? 1 : 1 + distance / bevel;
        const index = (y * cw + x) * 4;

        if (t <= 0) {
          data[index] = 128;
          data[index + 1] = 128;
          data[index + 2] = 128;
          data[index + 3] = 255;
          continue;
        }

        const magnitude = Math.pow(t, p.curvature);
        // SDF 梯度即外法线；折射把背景往内侧压，所以取负号
        const gx =
          roundedRectSDF(px + epsilon, py, hw, hh, rr) - roundedRectSDF(px - epsilon, py, hw, hh, rr);
        const gy =
          roundedRectSDF(px, py + epsilon, hw, hh, rr) - roundedRectSDF(px, py - epsilon, hw, hh, rr);
        const length = Math.hypot(gx, gy) || 1;
        const ox = (-gx / length) * magnitude;
        const oy = (-gy / length) * magnitude;

        data[index] = toByte(128 + ox * 127);
        data[index + 1] = toByte(128 + oy * 127);
        data[index + 2] = 128;
        data[index + 3] = 255;
      }
    }

    ctx.putImageData(image, 0, 0);
    return canvas.toDataURL();
  }
</script>

{#if active}
  <svg class="lqg-defs" width="0" height="0" aria-hidden="true" focusable="false">
    <filter
      id={filterId}
      filterUnits="userSpaceOnUse"
      primitiveUnits="userSpaceOnUse"
      x="0"
      y="0"
      width={width}
      height={height}
      color-interpolation-filters="sRGB"
    >
      <feImage
        href={mapUrl}
        x="0"
        y="0"
        width={width}
        height={height}
        preserveAspectRatio="none"
        result="map"
      />
      <feDisplacementMap
        in="SourceGraphic"
        in2="map"
        scale={preset.displace}
        xChannelSelector="R"
        yChannelSelector="G"
      />
    </filter>
  </svg>

  <div
    class="lqg-surface"
    class:ready={mapUrl !== ""}
    style="--lqg-radius:{radius}px; --lqg-blur:{preset.blur}px; --lqg-sat:{preset.saturate}%; --lqg-bright:{preset.brightness}; --lqg-tint:{preset.tint}; --lqg-sheen:{preset.sheen}; --lqg-filter:url(#{filterId});"
  ></div>
{/if}

<style>
  .lqg-defs {
    position: absolute;
    width: 0;
    height: 0;
    overflow: hidden;
    pointer-events: none;
  }

  .lqg-surface {
    position: absolute;
    inset: 0;
    /* 负层级：排在 .footer-main 背景之后、::before 与内容之前，
       backdrop-filter 因此能采到页面而不是 footer 自己 */
    z-index: -1;
    pointer-events: none;
    border-radius: var(--lqg-radius);
    /* 玻璃本体几乎无色，形体靠折射和高光，不靠底色堆不透明度 */
    background: linear-gradient(
      180deg,
      rgba(255, 255, 255, var(--lqg-tint)),
      rgba(255, 255, 255, calc(var(--lqg-tint) * 0.3)) 46%,
      rgba(255, 255, 255, calc(var(--lqg-tint) * 0.55))
    );
    box-shadow:
      inset 0 1px 0 rgba(255, 255, 255, 0.5),
      inset 0 0 0 1px rgba(255, 255, 255, 0.14),
      inset 0 -1px 0 rgba(255, 255, 255, 0.1);
    /* 不带 url() 的降级：万一 backdrop-filter 不接受滤镜引用，至少还有磨砂 */
    -webkit-backdrop-filter: blur(var(--lqg-blur)) saturate(var(--lqg-sat));
    backdrop-filter: blur(var(--lqg-blur)) saturate(var(--lqg-sat));
  }

  /* url() 必须排在最前：先位移实时 backdrop，再模糊，否则折射细节会被模糊吃掉 */
  .lqg-surface.ready {
    backdrop-filter: var(--lqg-filter) blur(var(--lqg-blur)) saturate(var(--lqg-sat))
      brightness(var(--lqg-bright));
  }

  /* 斜向高光带：纯合成层，无滤镜开销 */
  .lqg-surface::after {
    content: "";
    position: absolute;
    inset: 0;
    border-radius: inherit;
    background: linear-gradient(
      112deg,
      rgba(255, 255, 255, 0) 32%,
      rgba(255, 255, 255, var(--lqg-sheen)) 47%,
      rgba(255, 255, 255, 0) 60%
    );
  }
</style>
