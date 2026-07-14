<script lang="ts">
  import { onMount } from "svelte";
  import { closeFloatWindow as closeNativeFloatWindow, emitTo, getFloatingLyricPayload, getFloatingLyricSettings, getFloatWindowPosition, hideFloatWindow, listen, moveFloatWindow, setFloatWindowHeight } from "./lib/tauri";
  import { type AppSettings } from "./lib/stores/settings";
  import { getNextLyricVariantMode, isLyricVariantModeActive, lyricVariantButtonLabel as getLyricVariantButtonLabel, lyricVariantButtonTitle as getLyricVariantButtonTitle, normalizeLyricVariantMode, type LyricVariantMode } from "./lib/lyrics";

  type VariantKind = "translation" | "phonetic";

  // 浮窗为被动视图：歌词 + 设置由主窗推送。
  // 锁定不使用 set_ignore_cursor_events（Tauri 无 forward，一旦穿透就无法点解锁）；
  // 改用透明像素穿透 + 锁定时常显工具栏。
  let lyric = $state("等待歌词");
  let tlyric = $state("");
  let variantKind = $state<VariantKind>("translation");
  let nextLyric = $state("");
  let hasTranslation = $state(false);
  let hasPhonetic = $state(false);
  let showToolbar = $state(false);
  let isDragging = $state(false);
  let lyricWindow = $state<AppSettings["lyricWindow"]>({
    fontSize: 24,
    color: "#ffffff",
    colorScheme: "classic",
    gradientFrom: "#7cf7c8",
    gradientTo: "#f0a6ff",
    backgroundAlpha: 0.8,
    lines: 1,
    offsetMs: 0,
    staggeredLayout: false,
    locked: false,
    rememberLockState: true,
    variantMode: "translation",
  });
  let enableFloatWindow = $state(true);
  let hideWhenMainVisible = $state(true);
  let mouseStartX = 0;
  let mouseStartY = 0;
  let windowStartX = 0;
  let windowStartY = 0;
  let lastFloatHeight = 0;
  let lastAppliedSeq = 0;

  let variantAvailability = $derived({ hasTranslation, hasPhonetic });
  let variantMode = $derived<LyricVariantMode>(
    normalizeLyricVariantMode(lyricWindow.variantMode, variantAvailability)
  );
  let locked = $derived(Boolean(lyricWindow.locked));

  function applyLyricPayload(payload: {
    lyric: string;
    tlyric: string;
    variantKind?: VariantKind;
    nextLyric?: string;
    hasTranslation?: boolean;
    hasPhonetic?: boolean;
    seq?: number;
  }) {
    // 丢弃迟到的过期推送：seq 更小说明是旧帧，忽略以免把已显示的新句覆盖回旧句。
    if (typeof payload.seq === "number") {
      if (payload.seq <= lastAppliedSeq) return;
      lastAppliedSeq = payload.seq;
    }
    lyric = payload.lyric;
    tlyric = payload.tlyric;
    variantKind = payload.variantKind ?? "translation";
    nextLyric = payload.nextLyric ?? "";
    hasTranslation = typeof payload.hasTranslation === "boolean"
      ? payload.hasTranslation
      : Boolean(payload.tlyric) && (payload.variantKind ?? "translation") === "translation";
    hasPhonetic = typeof payload.hasPhonetic === "boolean"
      ? payload.hasPhonetic
      : Boolean(payload.tlyric) && payload.variantKind === "phonetic";
  }

  function parseLyricPayload(payload: unknown): {
    lyric: string;
    tlyric: string;
    variantKind?: VariantKind;
    nextLyric?: string;
    hasTranslation?: boolean;
    hasPhonetic?: boolean;
    seq?: number;
  } | null {
    try {
      const value = typeof payload === "string" ? JSON.parse(payload) : payload;
      if (!value || typeof value !== "object") return null;
      const data = value as { lyric?: unknown; tlyric?: unknown };
      if (typeof data.lyric !== "string" || data.lyric === "Aura") return null;
      return {
        ...(value as object),
        lyric: data.lyric,
        tlyric: typeof data.tlyric === "string" ? data.tlyric : "",
      } as ReturnType<typeof parseLyricPayload>;
    } catch {
      return null;
    }
  }

  function applySettingsPayload(payload: {
    lyricWindow?: Partial<AppSettings["lyricWindow"]>;
    enableLyricFloatingWindow?: boolean;
    hideLyricFloatingWindowWhenMainVisible?: boolean;
  }) {
    if (payload.lyricWindow) {
      lyricWindow = { ...lyricWindow, ...payload.lyricWindow };
    }
    if (typeof payload.enableLyricFloatingWindow === "boolean") {
      enableFloatWindow = payload.enableLyricFloatingWindow;
    }
    if (typeof payload.hideLyricFloatingWindowWhenMainVisible === "boolean") {
      hideWhenMainVisible = payload.hideLyricFloatingWindowWhenMainVisible;
    }
  }

  function parseSettingsPayload(payload: unknown): {
    lyricWindow?: Partial<AppSettings["lyricWindow"]>;
    enableLyricFloatingWindow?: boolean;
    hideLyricFloatingWindowWhenMainVisible?: boolean;
  } | null {
    try {
      const value = typeof payload === "string" ? JSON.parse(payload) : payload;
      if (!value || typeof value !== "object") return null;
      const data = value as {
        lyricWindow?: Partial<AppSettings["lyricWindow"]>;
        enableLyricFloatingWindow?: boolean;
        hideLyricFloatingWindowWhenMainVisible?: boolean;
      };
      return {
        lyricWindow: data.lyricWindow,
        enableLyricFloatingWindow: data.enableLyricFloatingWindow,
        hideLyricFloatingWindowWhenMainVisible: data.hideLyricFloatingWindowWhenMainVisible,
      };
    } catch {
      return null;
    }
  }

  onMount(() => {
    let unlistenSettings: (() => void) | null = null;
    let onNativeLyricUpdate: ((event: Event) => void) | null = null;
    let disposed = false;

    void (async () => {
      // 冷启动 seed：从 Rust 缓存拉初值（localStorage 通道已废弃）。
      try {
        const nativePayload = await getFloatingLyricPayload();
        const parsed = parseLyricPayload(nativePayload);
        if (parsed) applyLyricPayload(parsed);
      } catch {}

      try {
        const nativeSettings = await getFloatingLyricSettings();
        const parsed = parseSettingsPayload(nativeSettings);
        if (parsed) applySettingsPayload(parsed);
      } catch {}

      // 唯一歌词入口：Rust eval 注入的 CustomEvent。
      onNativeLyricUpdate = (event: Event) => {
        const parsed = parseLyricPayload((event as CustomEvent).detail);
        if (parsed) applyLyricPayload(parsed);
      };
      window.addEventListener("listen1-native-lyric-update", onNativeLyricUpdate);

      // 唯一设置入口：Tauri emit。
      unlistenSettings = await listen("lyric-settings-update", (e) => {
        const parsed = parseSettingsPayload(e.payload);
        if (parsed) applySettingsPayload(parsed);
      });

      await emitTo("main", "float-lyric-ready");
      if (disposed) {
        unlistenSettings();
      }
    })();

    return () => {
      disposed = true;
      unlistenSettings?.();
      if (onNativeLyricUpdate) window.removeEventListener("listen1-native-lyric-update", onNativeLyricUpdate);
    };
  });

  function onMouseEnter() {
    showToolbar = true;
  }

  function onMouseLeave() {
    // 锁定时工具栏常显（用于解锁），不因 leave 隐藏
    if (!locked) showToolbar = false;
  }

  function toggleLock() {
    // 乐观更新本地 locked，主窗 patch 后经 settings 回流校正。
    const nextLocked = !locked;
    lyricWindow = { ...lyricWindow, locked: nextLocked };
    showToolbar = true;
    void emitTo("main", "float-lyric-lock-changed", { locked: nextLocked }).catch(() => undefined);
  }

  async function closeFloatWindow() {
    void emitTo("main", "float-lyric-closed", {}).catch(() => undefined);
    await closeNativeFloatWindow()
      .catch(() => hideFloatWindow())
      .catch((error) => {
        console.error("[FloatingLyric] hide float window failed", error);
      });
  }

  function nextVariantMode(): LyricVariantMode {
    return getNextLyricVariantMode(variantMode, variantAvailability);
  }

  function variantButtonLabel() {
    return getLyricVariantButtonLabel(variantMode, variantAvailability, true);
  }

  function variantButtonTitle() {
    return getLyricVariantButtonTitle(variantMode, variantAvailability);
  }

  function toggleVariantMode() {
    if (!hasTranslation && !hasPhonetic) return;
    // 只发请求给主窗；主窗 patch settings.variantMode 后经设置通道回流。
    const nextMode = nextVariantMode();
    void emitTo("main", "float-lyric-variant-next", { mode: nextMode }).catch(() => undefined);
  }

  function patchLyricWindow(partial: Partial<AppSettings["lyricWindow"]>) {
    // 工具栏 A±/◐± 等调整：只发请求给主窗，主窗写 settings 后回流。
    void emitTo("main", "float-lyric-settings-changed", { lyricWindow: partial }).catch(() => undefined);
  }

  function toolbarTextColor(scheme: string, color: string) {
    if (scheme === "gold") return "#3b2600";
    if (scheme === "cyan") return "#042f3a";
    if (scheme === "rose") return "#3c0618";
    if (scheme === "aurora") return "#062f2b";
    if (scheme === "sunset") return "#351100";
    if (scheme === "classic" || color.toLowerCase() === "#ffffff") return "#0b1f34";
    return readableTextColor(color);
  }

  function toolbarButtonBackground(scheme: string, color: string) {
    if (scheme === "gold") return "#ffe6a3";
    if (scheme === "cyan") return "#8be9fd";
    if (scheme === "rose") return "#ffc1d6";
    if (scheme === "aurora") return "#9defff";
    if (scheme === "sunset") return "#ffd166";
    if (scheme === "classic" || color.toLowerCase() === "#ffffff") return "#8fd3ff";
    return color;
  }

  function readableTextColor(color: string) {
    const hex = color.trim().replace("#", "");
    const full = hex.length === 3
      ? hex.split("").map((part) => `${part}${part}`).join("")
      : hex;
    if (!/^[0-9a-f]{6}$/i.test(full)) return "#0b1f34";
    const r = Number.parseInt(full.slice(0, 2), 16);
    const g = Number.parseInt(full.slice(2, 4), 16);
    const b = Number.parseInt(full.slice(4, 6), 16);
    const luminance = (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255;
    return luminance > 0.58 ? "#111827" : "#f8fafc";
  }

  async function startDrag(e: MouseEvent) {
    if (locked || (e.target instanceof HTMLElement && e.target.closest(".toolbar"))) return;
    e.preventDefault();
    mouseStartX = e.screenX;
    mouseStartY = e.screenY;
    try {
      const [x, y] = await getFloatWindowPosition();
      windowStartX = x;
      windowStartY = y;
    } catch (error) {
      console.error("[FloatingLyric] get float window position failed", error);
      windowStartX = Math.round(e.screenX - e.clientX);
      windowStartY = Math.round(e.screenY - e.clientY);
    }
    isDragging = true;

    const onMove = async (e: MouseEvent) => {
      await moveFloatWindow(
        Math.round(windowStartX + e.screenX - mouseStartX),
        Math.round(windowStartY + e.screenY - mouseStartY)
      );
    };
    const onUp = () => {
      isDragging = false;
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  }

  let lyricStyleClass = $derived(`scheme-${lyricWindow.colorScheme}`);
  let variantActive = $derived(isLyricVariantModeActive(variantMode, variantAvailability));
  let hasVariant = $derived(Boolean(tlyric && variantMode !== "off"));
  let showTwoLines = $derived(Boolean(lyricWindow.lines === 2 && nextLyric));
  let staggeredLayout = $derived(Boolean(lyricWindow.staggeredLayout && showTwoLines));
  let lyricPayloadKey = $derived(`${lyric}:${tlyric}:${variantKind}:${variantMode}:${nextLyric}:${showTwoLines}:${staggeredLayout}`);
  let floatHeight = $derived(Math.max(72, Math.min(220, Math.round(
    showTwoLines
      ? lyricWindow.fontSize * 2.85 + 44
      : hasVariant
        ? lyricWindow.fontSize * 1.95 + 30
        : lyricWindow.fontSize * 1.45 + 34
  ))));
  let toolbarColor = $derived(toolbarTextColor(lyricWindow.colorScheme, lyricWindow.color));
  let toolbarButtonBg = $derived(toolbarButtonBackground(lyricWindow.colorScheme, lyricWindow.color));
  let toolbarBackgroundAlpha = $derived(Math.min(0.72, Math.max(0.26, lyricWindow.backgroundAlpha + 0.16)));
  // 锁定时工具栏常显，保证解锁按钮始终可点
  let toolbarVisible = $derived(locked || showToolbar);

  $effect(() => {
    if (Math.abs(floatHeight - lastFloatHeight) < 2) return;
    lastFloatHeight = floatHeight;
    void setFloatWindowHeight(floatHeight).catch(() => undefined);
  });
</script>

<div
  class={`float-window ${lyricStyleClass}`}
  class:locked
  class:two-lines={showTwoLines}
  class:staggered={staggeredLayout}
  style="font-size: {lyricWindow.fontSize}px; min-height: {floatHeight}px; --lyric-custom-color: {lyricWindow.color}; --lyric-gradient-from: {lyricWindow.gradientFrom ?? '#7cf7c8'}; --lyric-gradient-to: {lyricWindow.gradientTo ?? '#f0a6ff'}; --float-bg-alpha: {locked ? 0 : lyricWindow.backgroundAlpha}; --float-toolbar-color: {toolbarColor}; --float-toolbar-button-bg: {toolbarButtonBg}; --float-toolbar-bg: rgba(18,18,18,{toolbarBackgroundAlpha}); color: {lyricWindow.color};"
  onmouseenter={onMouseEnter}
  onmouseleave={onMouseLeave}
  role="region"
  aria-label="浮动歌词"
>
  <div class="toolbar safe-toolbar" class:visible={toolbarVisible} class:locked-toolbar={locked}>
    {#if !locked}
      <button class="tb-btn variant-btn" class:active={variantActive} disabled={!hasTranslation && !hasPhonetic} onclick={toggleVariantMode} title={variantButtonTitle()}>{variantButtonLabel()}</button>
      <button class="tb-btn" onclick={() => patchLyricWindow({ fontSize: Math.max(14, lyricWindow.fontSize - 2) })} title="缩小字体">A-</button>
      <button class="tb-btn" onclick={() => patchLyricWindow({ fontSize: Math.min(60, lyricWindow.fontSize + 2) })} title="放大字体">A+</button>
      <button class="tb-btn" onclick={() => patchLyricWindow({ backgroundAlpha: Math.max(0, lyricWindow.backgroundAlpha - 0.1) })} title="降低背景">◐-</button>
      <button class="tb-btn" onclick={() => patchLyricWindow({ backgroundAlpha: Math.min(1, lyricWindow.backgroundAlpha + 0.1) })} title="提高背景">◐+</button>
    {/if}
    <button class="tb-btn" type="button" onclick={toggleLock} title={locked ? "解锁" : "锁定"}>
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
        {#if locked}
          <rect x="3" y="11" width="18" height="11" rx="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/>
        {:else}
          <rect x="3" y="11" width="18" height="11" rx="2"/><path d="M7 11V7a5 5 0 0 1 9.9-1"/>
        {/if}
      </svg>
    </button>
    <button class="tb-btn" type="button" onclick={closeFloatWindow} title="关闭桌面歌词">
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
        <path d="M18 6 6 18M6 6l12 12"/>
      </svg>
    </button>
  </div>

  <div
    class="lyric-content"
    onmousedown={startDrag}
    role="none"
  >
    {#key lyricPayloadKey}
      <div class="lyric-stack">
        <span class="lyric-text">{lyric}</span>
        {#if hasVariant}
          <span class="lyric-trans" class:phonetic={variantKind === "phonetic"}>{tlyric}</span>
        {/if}
        {#if showTwoLines}
          <span class="lyric-next">{nextLyric}</span>
        {/if}
      </div>
    {/key}
  </div>
</div>

<style>
  :global(html, body, #app) {
    margin: 0;
    padding: 0;
    overflow: hidden;
    background: transparent !important;
    width: 100%;
    height: 100%;
    border: 0;
    outline: 0;
    box-shadow: none;
    filter: none;
  }

  .float-window {
    width: 100vw;
    min-height: 70px;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    border-radius: 0;
    overflow: visible;
    position: relative;
    user-select: none;
    border: 0;
    outline: 0;
    box-shadow: none;
    filter: none;
    background: transparent !important;
    transition: min-height 0.18s ease;
    pointer-events: none;
  }

  /* 未锁定时窗口本体可接收 hover；锁定时仅工具栏可点，其余透明穿透 */
  .float-window:not(.locked) {
    pointer-events: auto;
  }

  .float-window.locked {
    background: transparent !important;
  }

  .float-window.locked .lyric-content {
    cursor: default;
    pointer-events: none !important;
    background: transparent !important;
  }

  .safe-toolbar {
    opacity: 0;
    pointer-events: none;
  }

  .safe-toolbar.visible {
    opacity: 1;
    pointer-events: auto;
  }

  .safe-toolbar.locked-toolbar {
    opacity: 1;
    pointer-events: auto;
  }

  .lyric-content {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 24px 20px 10px;
    cursor: move;
    width: 100%;
    text-align: center;
    background: rgba(0, 0, 0, var(--float-bg-alpha));
    border: 0;
    outline: 0;
    box-shadow: none;
    filter: none;
    pointer-events: auto;
  }

  .lyric-stack {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    width: min(100%, 920px);
    animation: lyricSwap 260ms cubic-bezier(0.2, 0.72, 0.18, 1) both;
    will-change: transform, opacity;
  }

  @keyframes lyricSwap {
    from {
      opacity: 0;
      transform: translateY(18px);
      filter: blur(1.5px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
      filter: blur(0);
    }
  }

  .float-window.staggered .lyric-stack {
    align-items: stretch;
    animation-name: lyricSwapStaggered;
  }

  @keyframes lyricSwapStaggered {
    from {
      opacity: 0;
      transform: translate(24px, 18px);
      filter: blur(1.5px);
    }
    to {
      opacity: 1;
      transform: translate(0, 0);
      filter: blur(0);
    }
  }

  .lyric-text {
    font-weight: 600;
    text-shadow: none;
    line-height: 1.3;
    color: var(--lyric-custom-color);
  }

  .float-window.staggered .lyric-text {
    align-self: flex-start;
    max-width: 88%;
    text-align: left;
    transform: translateX(4vw);
  }

  .scheme-gold .lyric-text,
  .scheme-gold .lyric-next {
    color: #ffe6a3;
  }

  .scheme-cyan .lyric-text,
  .scheme-cyan .lyric-next {
    color: #8be9fd;
  }

  .scheme-rose .lyric-text,
  .scheme-rose .lyric-next {
    color: #ffc1d6;
  }

  .scheme-aurora .lyric-text,
  .scheme-aurora .lyric-next,
  .scheme-sunset .lyric-text,
  .scheme-sunset .lyric-next,
  .scheme-custom-gradient .lyric-text,
  .scheme-custom-gradient .lyric-next {
    color: transparent;
    -webkit-background-clip: text;
    background-clip: text;
  }

  .scheme-aurora .lyric-text,
  .scheme-aurora .lyric-next {
    background-image: linear-gradient(90deg, #7cf7c8, #74a7ff 52%, #f0a6ff);
  }

  .scheme-sunset .lyric-text,
  .scheme-sunset .lyric-next {
    background-image: linear-gradient(90deg, #ffd166, #ff6b6b 52%, #845ec2);
  }

  .scheme-custom-gradient .lyric-text,
  .scheme-custom-gradient .lyric-next {
    background-image: linear-gradient(90deg, var(--lyric-gradient-from), var(--lyric-gradient-to));
  }

  .lyric-trans {
    font-size: 0.68em;
    opacity: 0.78;
    margin-top: 0;
    line-height: 1.15;
    text-shadow: none;
  }

  .lyric-trans.phonetic,
  .lyric-next-trans.phonetic {
    letter-spacing: 0;
    opacity: 0.7;
  }

  .lyric-next {
    margin-top: 4px;
    font-size: 0.78em;
    font-weight: 600;
    opacity: 0.72;
    line-height: 1.25;
    text-shadow: none;
  }

  .float-window.staggered .lyric-next {
    align-self: flex-end;
    max-width: 88%;
    text-align: right;
    transform: translateX(-4vw);
  }

  .lyric-next-trans {
    margin-top: 1px;
    font-size: 0.58em;
    opacity: 0.58;
    line-height: 1.12;
    text-shadow: none;
  }

  .toolbar {
    position: absolute;
    top: 4px;
    right: 8px;
    display: flex;
    gap: 4px;
    background: color-mix(in srgb, var(--float-toolbar-button-bg) 16%, rgba(18,18,18,0.72));
    border: 0;
    border-radius: 6px;
    padding: 3px;
    z-index: 5;
    box-shadow: none;
  }

  .tb-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 3px 6px;
    border-radius: 4px;
    color: var(--float-toolbar-color);
    background: color-mix(in srgb, var(--float-toolbar-button-bg) 82%, rgba(0,0,0,0.16));
    font-size: 11px;
    border: 0;
    box-shadow: none;
    transition: background 0.15s, color 0.15s, transform 0.15s;
  }

  .tb-btn:hover {
    background: color-mix(in srgb, var(--float-toolbar-button-bg) 94%, #ffffff 6%);
    color: var(--float-toolbar-color);
    transform: translateY(-1px);
  }

  .tb-btn.active {
    color: var(--float-toolbar-color);
  }

  .variant-btn:not(.active) {
    background: rgba(255, 255, 255, 0.12);
    color: color-mix(in srgb, var(--float-toolbar-color) 72%, #ffffff 28%);
  }

  .variant-btn.active {
    background: color-mix(in srgb, var(--float-toolbar-button-bg) 92%, rgba(255,255,255,0.08));
  }
</style>
