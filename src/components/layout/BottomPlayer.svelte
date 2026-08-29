<script lang="ts">
  import { playerState, progressPercent, positionFormatted, durationFormatted } from "../../lib/stores/player";
  import { player } from "../../lib/player";
  import { settings, resolvedTheme } from "../../lib/stores/settings";
  import { deviceTier } from "../../lib/stores/device";
  import { MediaService, myplaylistLib } from "../../lib/providers/index";
  import { sizedImageUrl, proxyResourceUrl } from "../../lib/resourceUrl";
  import { runOnActionKey } from "../../lib/keyboard";
  import { toast } from "../../lib/stores/toast";
  import { openExternalUrl } from "../../lib/tauri";
  import LiquidGlassSurface from "../effects/LiquidGlassSurface.svelte";
  import NowPlayingView from "../views/NowPlayingView.svelte";
  import WindowControls from "./WindowControls.svelte";
  import { fade, fly } from "svelte/transition";
  import { getLyricsVariantAvailability, getNextLyricVariantMode, isLyricVariantModeActive, lyricVariantButtonLabel as getLyricVariantButtonLabel, lyricVariantButtonTitle as getLyricVariantButtonTitle, normalizeLyricVariantMode, parseLyric, type LyricLine, type LyricVariantMode } from "../../lib/lyrics";

  let {
    navigate,
    activeView,
    nowPlayingOpen = false,
    onCloseNowPlaying = () => {},
  }: {
    navigate: (v: unknown) => void;
    activeView: { type: string };
    nowPlayingOpen?: boolean;
    onCloseNowPlaying?: () => void;
  } = $props();

  let isNowPlaying = $derived(activeView.type === "nowplaying");
  let hasPlayerContent = $derived(Boolean($playerState.currentTrack) || $playerState.playlist.length > 0);
  let isDragging = $state(false);
  let dragPercent = $state(0);
  let showQueue = $state(false);
  let showAddMenu = $state(false);
  let addMenuCreateMode = $state(false);
  let addMenuCreateTitle = $state("");
  let myPlaylists = $state<Array<{ id: string; title: string }>>([]);
  let queueListEl = $state<HTMLElement | null>(null);
  let shouldScrollQueueOnOpen = $state(false);
  let adaptiveAccent = $state<{ r: number; g: number; b: number } | null>(null);
  let coverSwitchDirection = $state<"next" | "prev" | "">("");
  // 底栏译文按钮的 availability：本组件自主解析当前歌词（不再依赖跨窗口通道），
  // 与 LyricSync 独立但结果一致（同一首歌、同一 lyrics.ts 算法）。
  let bottomLyricLines = $state<LyricLine[]>([]);
  let bottomLyricTrackId = $state("");
  let currentLyricAvailability = $derived(getLyricsVariantAvailability(bottomLyricLines));
  let currentLyricHasTranslation = $derived(currentLyricAvailability.hasTranslation);
  let currentLyricHasPhonetic = $derived(currentLyricAvailability.hasPhonetic);
  let currentLyricVariantMode = $derived<LyricVariantMode>(
    normalizeLyricVariantMode($settings.lyricWindow.variantMode, currentLyricAvailability)
  );
  let currentLyricVariantActive = $derived(isLyricVariantModeActive(currentLyricVariantMode, currentLyricAvailability));
  let lastCoverIndex = -1;
  let lastCoverTrackId = "";
  let currentCoverUrl = $derived(sizedImageUrl($playerState.currentTrack?.img_url, 200));
  let coverAccentStyle = $derived.by(() => {
    if (!$settings.enableCoverAdaptiveTheme || !adaptiveAccent) return "";
    const { r, g, b } = adaptiveAccent;
    return `--cover-accent-rgb:${r},${g},${b};--cover-accent:rgb(${r} ${g} ${b});`;
  });
  let footerMainEl = $state<HTMLElement | null>(null);
  /**
   * 展开/收起动画进行中。
   *
   * `.footer-main` 的高度要在一瞬间从 100px 变到 100vh，而它身上挂着
   * `backdrop-filter: var(--visual-backdrop)`（high 档是 saturate+blur20）。
   * 尺寸每帧都变 → 合成器每帧都得重新为整块离屏纹理做一遍模糊，面积一路涨到全屏，
   * present 直接落后：状态早就切完了，画面还停在原处，然后一口气追上——
   * 就是「点了没反应，然后突然快起来」。液态玻璃那层更糟，除了模糊还要跑
   * feDisplacementMap，收起落定那一下还有一次 toDataURL 卡主线程。
   *
   * 所以动画期间把这些常驻滤镜全部摘掉，落定后再装回去：静止的两个姿态观感不变。
   */
  let footerResizing = $state(false);
  $effect(() => {
    // 读一下就够，isNowPlaying 一变就重跑
    void isNowPlaying;
    footerResizing = true;
    // 兜底清除：transitionend 不一定发得出来（高度没实际变化、或元素被移出文档），
    // 光靠事件会把滤镜永久关掉。要比 --player-expand-dur(300ms) 略长一点。
    const timer = setTimeout(() => (footerResizing = false), 380);
    return () => clearTimeout(timer);
  });
  // 玻璃表面只在 high / ultra 档 + 液态玻璃主题下启用。现在走原生 backdrop-filter，
  // 没有 DOM 克隆也没有逐帧同步，因此播放中也可以常开（旧实现必须暂停时才敢开）。
  let glassSurfaceEnabled = $derived(
    ($deviceTier === "high" || $deviceTier === "ultra") &&
    $resolvedTheme === "liquidGlass" &&
    !isNowPlaying &&
    !footerResizing &&
    Boolean($playerState.currentTrack)
  );
  // 相邻封面常驻显示；只有最低档才省掉这两张解码位图。
  let showAdjacentCovers = $derived($deviceTier !== "low");
  // 播放页首次打开前不挂载（隐藏的完整歌词 DOM 常驻内存无意义），打开过一次后保持挂载以保留滑出动画。
  let nowPlayingEverOpened = $state(false);
  $effect(() => {
    if (nowPlayingOpen) nowPlayingEverOpened = true;
  });
  // 三个窗口按钮翻下来时会盖住歌词区右上角的进度微调/翻译按钮，翻出期间让那一排
  // 整体下移躲开。用 state 而不是纯 CSS：那两个按钮在 NowPlayingView 里，而
  // NowPlayingView 是 .wc-flyout 的前置兄弟节点的子节点，选择器够不到。
  let wcRevealed = $state(false);
  // 实际那三个按钮的 DOM。让位与否要按真实横向区间判断，不能按 220px 的命中带算。
  let wcInnerEl = $state<HTMLElement | null>(null);
  // 收起播放页时 .wc-flyout 直接从 DOM 移除，pointerleave 不一定发得出来，
  // 所以这里再兜一层，避免下次展开时那一排还停在下移位置。
  let wcButtonsRevealed = $derived(isNowPlaying && wcRevealed);
  let prevCoverUrl = $derived.by(() => {
    if (!showAdjacentCovers) return "";
    const list = $playerState.playlist;
    const index = $playerState.currentIndex;
    if (!list.length || index < 0) return "";
    return sizedImageUrl(list[(index - 1 + list.length) % list.length]?.img_url, 200);
  });
  let nextCoverUrl = $derived.by(() => {
    if (!showAdjacentCovers) return "";
    const list = $playerState.playlist;
    const index = $playerState.currentIndex;
    if (!list.length || index < 0) return "";
    return sizedImageUrl(list[(index + 1) % list.length]?.img_url, 200);
  });
  let displayedProgress = $derived(isDragging ? dragPercent : $progressPercent);

  const LOOP_ICONS = [
    `<path d="M17 1l4 4-4 4"/><path d="M3 11V9a4 4 0 0 1 4-4h14"/><path d="M7 23l-4-4 4-4"/><path d="M21 13v2a4 4 0 0 1-4 4H3"/>`,
    `<polyline points="16 3 21 3 21 8"/><line x1="4" y1="20" x2="21" y2="3"/><polyline points="21 16 21 21 16 21"/><line x1="15" y1="15" x2="21" y2="21"/>`,
    `<path d="M17 1l4 4-4 4"/><path d="M3 11V9a4 4 0 0 1 4-4h14"/><path d="M7 23l-4-4 4-4"/><path d="M21 13v2a4 4 0 0 1-4 4H3"/><line x1="12" y1="8" x2="12" y2="16"/>`,
  ];

  function clamp(value: number, min: number, max: number) {
    return Math.max(min, Math.min(max, value));
  }

  function rgbToHsl(r: number, g: number, b: number): [number, number, number] {
    r /= 255; g /= 255; b /= 255;
    const max = Math.max(r, g, b);
    const min = Math.min(r, g, b);
    let h = 0;
    let s = 0;
    const l = (max + min) / 2;
    if (max !== min) {
      const d = max - min;
      s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
      if (max === r) h = (g - b) / d + (g < b ? 6 : 0);
      else if (max === g) h = (b - r) / d + 2;
      else h = (r - g) / d + 4;
      h /= 6;
    }
    return [h, s, l];
  }

  function hslToRgb(h: number, s: number, l: number): { r: number; g: number; b: number } {
    const hue2rgb = (p: number, q: number, t: number) => {
      if (t < 0) t += 1;
      if (t > 1) t -= 1;
      if (t < 1 / 6) return p + (q - p) * 6 * t;
      if (t < 1 / 2) return q;
      if (t < 2 / 3) return p + (q - p) * (2 / 3 - t) * 6;
      return p;
    };
    if (s === 0) {
      const v = Math.round(l * 255);
      return { r: v, g: v, b: v };
    }
    const q = l < 0.5 ? l * (1 + s) : l + s - l * s;
    const p = 2 * l - q;
    return {
      r: Math.round(hue2rgb(p, q, h + 1 / 3) * 255),
      g: Math.round(hue2rgb(p, q, h) * 255),
      b: Math.round(hue2rgb(p, q, h - 1 / 3) * 255),
    };
  }

  async function extractAccentFromCover(url: string): Promise<{ r: number; g: number; b: number } | null> {
    if (!url) return null;
    const image = new Image();
    image.crossOrigin = "anonymous";
    image.decoding = "async";
    const loaded = new Promise<void>((resolve, reject) => {
      image.onload = () => resolve();
      image.onerror = () => reject(new Error("cover image load failed"));
    });
    image.src = url;
    await loaded;

    const canvas = document.createElement("canvas");
    const size = 32;
    canvas.width = size;
    canvas.height = size;
    const ctx = canvas.getContext("2d", { willReadFrequently: true });
    if (!ctx) return null;
    ctx.drawImage(image, 0, 0, size, size);
    const data = ctx.getImageData(0, 0, size, size).data;

    let total = 0;
    let r = 0;
    let g = 0;
    let b = 0;
    for (let i = 0; i < data.length; i += 4) {
      const alpha = data[i + 3];
      if (alpha < 180) continue;
      const pr = data[i];
      const pg = data[i + 1];
      const pb = data[i + 2];
      const [, s, l] = rgbToHsl(pr, pg, pb);
      if (l < 0.08 || l > 0.94) continue;
      const weight = 0.2 + s * 1.8 + (0.5 - Math.abs(l - 0.52)) * 0.7;
      total += weight;
      r += pr * weight;
      g += pg * weight;
      b += pb * weight;
    }
    if (total <= 0) return null;
    const [h, s, l] = rgbToHsl(r / total, g / total, b / total);
    return hslToRgb(h, clamp(Math.max(s, 0.46), 0.36, 0.78), clamp(l, 0.38, 0.58));
  }

  function seekFromElement(el: HTMLElement, clientX: number) {
    const rect = el.getBoundingClientRect();
    return Math.max(0, Math.min(100, ((clientX - rect.left) / rect.width) * 100));
  }

  function setVolumePercent(value: number) {
    player.setVolume(Math.max(0, Math.min(100, value)), false);
    if ($playerState.muted) player.unmute();
  }

  function handleProgressDown(e: MouseEvent) {
    const target = e.currentTarget as HTMLElement;
    const pct = seekFromElement(target, e.clientX);
    isDragging = true;
    dragPercent = pct;
    e.preventDefault();

    const onMove = (event: MouseEvent) => {
      dragPercent = seekFromElement(target, event.clientX);
    };
    const onUp = () => {
      player.seek(dragPercent);
      isDragging = false;
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  }

  function handleProgressKeydown(e: KeyboardEvent) {
    if (e.key !== "ArrowLeft" && e.key !== "ArrowRight" && e.key !== "Home" && e.key !== "End") return;
    e.preventDefault();
    const current = $progressPercent;
    const next = e.key === "Home"
      ? 0
      : e.key === "End"
        ? 100
        : Math.max(0, Math.min(100, current + (e.key === "ArrowRight" ? 2 : -2)));
    player.seek(next);
  }

  function handleVolumeDown(e: MouseEvent) {
    const target = e.currentTarget as HTMLElement;
    const update = (event: MouseEvent) => setVolumePercent(seekFromElement(target, event.clientX));
    e.preventDefault();
    update(e);

    const onMove = (event: MouseEvent) => update(event);
    const onUp = () => {
      player.commitVolume();
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  }

  function handleVolumeWheel(e: WheelEvent) {
    e.preventDefault();
    player.adjustVolume(e.deltaY < 0);
    if ($playerState.muted) player.unmute();
  }

  function handleVolumeKeydown(e: KeyboardEvent) {
    if (e.key !== "ArrowLeft" && e.key !== "ArrowRight" && e.key !== "ArrowDown" && e.key !== "ArrowUp" && e.key !== "Home" && e.key !== "End") return;
    e.preventDefault();
    const next = e.key === "Home"
      ? 0
      : e.key === "End"
        ? 100
        : $playerState.volume + (e.key === "ArrowRight" || e.key === "ArrowUp" ? 5 : -5);
    setVolumePercent(next);
  }

  function playQueueIndex(index: number) {
    player.loadByIndex(index);
    closeQueue();
  }

  function removeQueueIndex(index: number, e: MouseEvent) {
    e.stopPropagation();
    player.removeTrack(index);
  }

  function refreshMyPlaylists() {
    myPlaylists = myplaylistLib.show("my").map((p) => ({ id: p.info.id, title: p.info.title }));
  }

  function closeAddMenu() {
    showAddMenu = false;
    addMenuCreateMode = false;
    addMenuCreateTitle = "";
  }

  function addCurrentToQueue() {
    const track = $playerState.currentTrack;
    if (!track) return;
    player.insertTrack(track);
    toast.success("已加入播放队列");
  }

  function openAddMenu() {
    refreshMyPlaylists();
    showAddMenu = true;
    closeQueue();
  }

  function handleAddCurrent() {
    if ($settings.bottomPlayerAddAction === "playlist") openAddMenu();
    else addCurrentToQueue();
  }

  function addCurrentToPlaylist(id: string) {
    const track = $playerState.currentTrack;
    if (!track) return;
    MediaService.addTrackToMyPlaylist(id, track);
    toast.success("已添加到歌单");
    closeAddMenu();
  }

  function createPlaylistAndAddCurrent() {
    const track = $playerState.currentTrack;
    const title = addMenuCreateTitle.trim();
    if (!track || !title) return;
    MediaService.createMyPlaylist(title, track);
    toast.success("已创建歌单并添加歌曲");
    closeAddMenu();
  }

  function nextLyricVariantMode(): LyricVariantMode {
    return getNextLyricVariantMode(currentLyricVariantMode, currentLyricAvailability);
  }

  function lyricVariantButtonLabel() {
    return getLyricVariantButtonLabel(currentLyricVariantMode, currentLyricAvailability, true);
  }

  function lyricVariantButtonTitle() {
    return getLyricVariantButtonTitle(currentLyricVariantMode, currentLyricAvailability);
  }

  function toggleLyricVariant() {
    if (!currentLyricHasTranslation && !currentLyricHasPhonetic) return;
    const nextMode = nextLyricVariantMode();
    settings.patch({
      lyricWindow: { ...$settings.lyricWindow, variantMode: nextMode },
    });
  }

  function toggleQueue() {
    const next = !showQueue;
    showQueue = next;
    shouldScrollQueueOnOpen = next;
    if (next) closeAddMenu();
  }

  function closeQueue() {
    showQueue = false;
    shouldScrollQueueOnOpen = false;
  }

  function toggleNowPlaying() {
    if (isNowPlaying) onCloseNowPlaying();
    else navigate({ type: "nowplaying" });
  }

  /**
   * 把"播放器已展开"这件事发布到 body 上。
   *
   * 展开后 `.footer` 是 z-index:320 的全屏层，但仍有 position: fixed 的浮动按钮
   * （歌单页右下角的定位按钮 z-index:900）压在它上面。那些按钮散落在各个视图里，
   * 而展开状态只有这个组件知道 —— App 那边把上一个视图继续挂着当背景，压根不会
   * 告诉它们。用一个 body 属性当广播口，谁需要让位谁自己写规则，不用一层层传 prop。
   */
  $effect(() => {
    const body = document.body;
    if (isNowPlaying) body.setAttribute("data-player-expanded", "true");
    else body.removeAttribute("data-player-expanded");
    return () => body.removeAttribute("data-player-expanded");
  });

  function toggleFloatingLyric() {
    // 仅切换开关，浮窗显隐由 LyricSync 统一决策（显式开启会立即显示以给出反馈）。
    settings.patch({ enableLyricFloatingWindow: !$settings.enableLyricFloatingWindow });
  }

  // 自主解析当前歌曲歌词以得出译文/音标 availability（底栏译文按钮的可用态数据源）。
  $effect(() => {
    const track = $playerState.currentTrack;
    if (!track) {
      bottomLyricLines = [];
      bottomLyricTrackId = "";
      return;
    }
    if (track.id === bottomLyricTrackId) return;
    bottomLyricTrackId = track.id;
    const inlineLyric = track.lyric ?? "";
    bottomLyricLines = inlineLyric.trim() ? parseLyric(inlineLyric) : [];
    MediaService.getLyric(track.id, track.album_id ?? "", track.lyric_url, track.tlyric_url)
      .then((result) => {
        if (track.id !== $playerState.currentTrack?.id) return;
        const lyric = result.lyric || inlineLyric;
        bottomLyricLines = lyric ? parseLyric(lyric, result.tlyric) : [];
      })
      .catch(() => undefined);
  });

  $effect(() => {
    const t = $playerState.currentTrack;
    if (t) {
      document.title = $playerState.playing
        ? `▶ ${t.title} - ${t.artist}`
        : `${t.title} - ${t.artist}`;
    }
  });

  $effect(() => {
    activeView.type;
    closeQueue();
    closeAddMenu();
  });

  $effect(() => {
    if (!showQueue || !shouldScrollQueueOnOpen) return;
    requestAnimationFrame(() => {
      const current = queueListEl?.querySelector<HTMLElement>('[data-current="true"]');
      current?.scrollIntoView({ block: "center" });
      shouldScrollQueueOnOpen = false;
    });
  });

  $effect(() => {
    const url = currentCoverUrl;
    const enabled = $settings.enableCoverAdaptiveTheme;
    let cancelled = false;
    if (!enabled || !url) {
      adaptiveAccent = null;
      return;
    }
    extractAccentFromCover(url)
      .then((accent) => {
        if (!cancelled) adaptiveAccent = accent;
      })
      .catch(() => {
        if (!cancelled) adaptiveAccent = null;
      });
    return () => {
      cancelled = true;
    };
  });

  $effect(() => {
    const index = $playerState.currentIndex;
    const length = $playerState.playlist.length;
    const trackId = $playerState.currentTrack?.id ?? "";
    if (index < 0 || !trackId) {
      lastCoverIndex = -1;
      lastCoverTrackId = "";
      coverSwitchDirection = "";
      return;
    }
    if (lastCoverIndex < 0) {
      lastCoverIndex = index;
      lastCoverTrackId = trackId;
      return;
    }
    if (index === lastCoverIndex && trackId === lastCoverTrackId) return;
    const previousIndex = lastCoverIndex;
    lastCoverIndex = index;
    lastCoverTrackId = trackId;
    const direction = (() => {
      if (length <= 1) return "next";
      const expectedPrev = (previousIndex - 1 + length) % length;
      return index === expectedPrev ? "prev" : "next";
    })();
    coverSwitchDirection = "";
    const frame = window.requestAnimationFrame(() => {
      coverSwitchDirection = direction;
    });
    const timer = window.setTimeout(() => {
      coverSwitchDirection = "";
    }, 240);
    return () => {
      window.cancelAnimationFrame(frame);
      window.clearTimeout(timer);
    };
  });
</script>

<div
  class="footer"
  class:footerdef={!hasPlayerContent}
  class:expanded={isNowPlaying}
  class:adaptive={$settings.enableCoverAdaptiveTheme && adaptiveAccent}
  style={coverAccentStyle}
>
  <div
    class="footer-main"
    class:slidedown={isNowPlaying}
    class:resizing={footerResizing}
    class:glass-surface={glassSurfaceEnabled}
    bind:this={footerMainEl}
    ontransitionend={(e) => {
      // 只认自己那条 height，子元素冒泡上来的一律不算
      if (e.target === footerMainEl && e.propertyName === "height") footerResizing = false;
    }}
  >
    <LiquidGlassSurface target={footerMainEl} enabled={glassSurfaceEnabled} />
    {#if nowPlayingEverOpened}
      <NowPlayingView visible={nowPlayingOpen} onClose={onCloseNowPlaying} windowControlsRevealed={wcButtonsRevealed} windowControlsEl={wcInnerEl} />
    {/if}

    {#if isNowPlaying}
      <!-- 展开后整块盖住标题栏，窗口控件在这里补一份，否则最小化/关闭都点不到 -->
      <div
        class="wc-flyout"
        role="group"
        aria-label="窗口控件"
        onpointerenter={() => (wcRevealed = true)}
        onpointerleave={() => (wcRevealed = false)}
        onfocusin={() => (wcRevealed = true)}
        onfocusout={() => (wcRevealed = false)}
      >
        <div class="wc-flyout-inner" bind:this={wcInnerEl}>
          <WindowControls />
        </div>
      </div>
    {/if}

    <div class="footerwrap" class:switch-next={coverSwitchDirection === "next"} class:switch-prev={coverSwitchDirection === "prev"}>
      <div class="left-control" class:slidedown={isNowPlaying}>
        <div class="playlist-toggle">
          <span
            class="icon"
            class:playlistactive={showQueue}
            onclick={toggleQueue}
            role="button"
            tabindex="0"
            onkeydown={(e) => runOnActionKey(e, toggleQueue)}
          >
            <svg viewBox="0 0 512 512" fill="currentColor" stroke="none">
              <path d="M16 256h256a16 16 0 0 0 16-16v-32a16 16 0 0 0-16-16H16a16 16 0 0 0-16 16v32a16 16 0 0 0 16 16zm0-128h256a16 16 0 0 0 16-16V80a16 16 0 0 0-16-16H16A16 16 0 0 0 0 80v32a16 16 0 0 0 16 16zm128 192H16a16 16 0 0 0-16 16v32a16 16 0 0 0 16 16h128a16 16 0 0 0 16-16v-32a16 16 0 0 0-16-16zM470.94 1.33l-96.53 28.51A32 32 0 0 0 352 60.34V360a148.76 148.76 0 0 0-48-8c-61.86 0-112 35.82-112 80s50.14 80 112 80 112-35.82 112-80V148.15l73-21.39a32 32 0 0 0 23-30.71V32a32 32 0 0 0-41.06-30.67z"/>
            </svg>
          </span>
        </div>

        <div class="splitter"></div>

        {#if $playerState.currentTrack}
          <div class="detail">
            <div class="title">
              {$playerState.currentTrack.title}
            </div>
            <div class="more-info">
              <div class="singer truncate">
                {$playerState.currentTrack.artist}
                {#if $playerState.currentTrack.album}
                  - {$playerState.currentTrack.album}
                {/if}
              </div>
            </div>
          </div>
        {/if}
      </div>

      <div class="main-info">
        {#if $playerState.currentTrack}
          <div class="cover" class:cover-shift-next={coverSwitchDirection === "next"} class:cover-shift-prev={coverSwitchDirection === "prev"}>
            <div class="cover-stage" aria-hidden="true">
              {#if prevCoverUrl}
                <span class="stage a">
                  <img src={prevCoverUrl} alt="" />
                </span>
              {/if}
              <span class="stage b">
                {#if currentCoverUrl}
                  <img
                    src={currentCoverUrl}
                    alt=""
                    class:liplay={$playerState.playing}
                    class:lipause={!$playerState.playing}
                  />
                {:else}
                  <span class="cover-placeholder"></span>
                {/if}
              </span>
              {#if nextCoverUrl}
                <span class="stage c">
                  <img src={nextCoverUrl} alt="" />
                </span>
              {/if}
            </div>
            <div class="cover-list">
              <span
                class="a"
                onclick={() => player.skip("prev")}
                role="button"
                tabindex="0"
                aria-label="上一首"
                title="上一首"
                onkeydown={(e) => runOnActionKey(e, () => player.skip("prev"))}
              >
                <svg viewBox="0 0 448 512" fill="currentColor" stroke="none">
                  <path d="M64 468V44c0-6.6 5.4-12 12-12h48c6.6 0 12 5.4 12 12v176.4l195.5-181C352.1 22.3 384 36.6 384 64v384c0 27.4-31.9 41.7-52.5 24.6L136 292.7V468c0 6.6-5.4 12-12 12H76c-6.6 0-12-5.4-12-12z"/>
                </svg>
              </span>
              <span
                class="b"
                onclick={() => player.togglePlayPause()}
                role="button"
                tabindex="0"
                aria-label={$playerState.playing ? "暂停" : "播放"}
                title={$playerState.playing ? "暂停" : "播放"}
                onkeydown={(e) => runOnActionKey(e, () => player.togglePlayPause())}
              >
                {#if $playerState.loading}
                  <div class="spinner"></div>
                {:else if $playerState.playing}
                  <svg class="pause-glyph" viewBox="0 0 448 512" fill="currentColor" stroke="none">
                    <path d="M144 479H48c-26.5 0-48-21.5-48-48V79c0-26.5 21.5-48 48-48h96c26.5 0 48 21.5 48 48v352c0 26.5-21.5 48-48 48zm304-48V79c0-26.5-21.5-48-48-48h-96c-26.5 0-48 21.5-48 48v352c0 26.5 21.5 48 48 48h96c26.5 0 48-21.5 48-48z"/>
                  </svg>
                {:else}
                  <svg viewBox="0 0 448 512" fill="currentColor" stroke="none">
                    <path d="M424.4 214.7 72.4 6.6C43.8-10.3 0 6.1 0 47.9V464c0 37.5 40.7 60.1 72.4 41.3l352-208c31.4-18.5 31.5-64.1 0-82.6z"/>
                  </svg>
                {/if}
              </span>
              <span
                class="c"
                onclick={() => player.skip("next")}
                role="button"
                tabindex="0"
                aria-label="下一首"
                title="下一首"
                onkeydown={(e) => runOnActionKey(e, () => player.skip("next"))}
              >
                <svg viewBox="0 0 448 512" fill="currentColor" stroke="none">
                  <path d="M384 44v424c0 6.6-5.4 12-12 12h-48c-6.6 0-12-5.4-12-12V291.6l-195.5 181C95.9 489.7 64 475.4 64 448V64c0-27.4 31.9-41.7 52.5-24.6L312 219.3V44c0-6.6 5.4-12 12-12h48c6.6 0 12 5.4 12 12z"/>
                </svg>
              </span>
            </div>
            <div class="circlemark" aria-hidden="true">
              <div class="circle" style="transform: rotate(-{displayedProgress / 100 * 180}deg)">
                <div class="topmark">
                  <div class="top"></div>
                </div>
                <div class="bottom">
                  <div class="bottomcircle"></div>
                </div>
              </div>
            </div>
          </div>
        {:else}
          <div class="logo-banner">
            <svg class="logo" viewBox="0 0 24 24">
              <polygon points="7 4 7 19 16 19 16 16 10 16 10 4"></polygon>
              <polygon points="13 4 13 13 16 13 16 4"></polygon>
            </svg>
          </div>
        {/if}

        <div class="footertime">
          <div class="bottomprogressbar">
            <div class="playbar">
              <span class="icon">
                <svg viewBox="0 0 24 24" fill="currentColor" stroke="none">
                  <path d="M12 2a8 8 0 0 0-8 8c0 5.5 8 12 8 12s8-6.5 8-12a8 8 0 0 0-8-8zm0 11.2A3.2 3.2 0 1 1 12 6.8a3.2 3.2 0 0 1 0 6.4z"/>
                </svg>
              </span>
              <div
                class="playbar-clickable"
                onmousedown={handleProgressDown}
                onkeydown={handleProgressKeydown}
                role="slider"
                tabindex="0"
                aria-valuenow={$progressPercent}
                aria-valuemin="0"
                aria-valuemax="100"
              >
                <div class="barbg">
                  <div class="cur" style="clip-path: inset(0 {100 - displayedProgress}% 0 0)"></div>
                  <span class="btn" style="left:{displayedProgress}%"><i></i></span>
                </div>
              </div>
            </div>
            <div class="volume-ctrl" onwheel={handleVolumeWheel}>
              <span
                class="icon"
                onclick={() => player.toggleMute()}
                role="button"
                tabindex="0"
                aria-label={$playerState.muted ? "取消静音" : "静音"}
                title={$playerState.muted ? "取消静音" : "静音"}
                onkeydown={(e) => runOnActionKey(e, () => player.toggleMute())}
              >
                {#if $playerState.muted}
                  <svg viewBox="0 0 24 24"><polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"/><line x1="23" y1="9" x2="17" y2="15"/><line x1="17" y1="9" x2="23" y2="15"/></svg>
                {:else}
                  <svg viewBox="0 0 24 24"><polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"/><path d="M19.07 4.93a10 10 0 0 1 0 14.14M15.54 8.46a5 5 0 0 1 0 7.07"/></svg>
                {/if}
              </span>
              <div
                class="m-pbar volume"
                onmousedown={handleVolumeDown}
                onkeydown={handleVolumeKeydown}
                role="slider"
                tabindex="0"
                aria-label="音量"
                aria-valuemin="0"
                aria-valuemax="100"
                aria-valuenow={$playerState.muted ? 0 : $playerState.volume}
              >
                <div class="barbg">
                  <div class="cur" style="clip-path: inset(0 {100 - $playerState.volume}% 0 0)"></div>
                  <span class="btn" style="left:{$playerState.volume}%"><i></i></span>
                </div>
              </div>
            </div>
          </div>
          <div class="timeswitch">
            <span class="current">{$positionFormatted}</span>
            <span style="font-weight:700"> / </span>
            <span class="total">{$durationFormatted}</span>
          </div>
        </div>
      </div>

      <div class="right-control">
        <div class="ctrl">
          <a href="/" onclick={(e) => { e.preventDefault(); handleAddCurrent(); }} title={$settings.bottomPlayerAddAction === "playlist" ? "添加到歌单" : "添加到播放队列"}>
            <span class="icon">
              <svg viewBox="0 0 24 24"><path d="M12 5v14M5 12h14"/></svg>
            </span>
          </a>
          <a
            href="/"
            title="播放模式"
            onclick={(e) => { e.preventDefault(); player.loopMode = (($playerState.loopMode + 1) % 3) as 0 | 1 | 2; }}
          >
            <span class="icon">
              <svg viewBox="0 0 24 24">
                {@html LOOP_ICONS[$playerState.loopMode]}
              </svg>
            </span>
          </a>
        </div>

        <div
          class="lyric-toggle"
          class:selected={$settings.enableLyricFloatingWindow}
          title="桌面歌词"
          role="button"
          tabindex="0"
          onclick={toggleFloatingLyric}
          onkeydown={(e) => runOnActionKey(e, toggleFloatingLyric)}
        >
          <svg viewBox="0 0 24 24">
            <rect x="4" y="4" width="16" height="16" rx="2"/>
            <path d="M8 9h8M8 13h5"/>
          </svg>
        </div>

        <div class="variant-switches" aria-label="歌词译文和音标">
          <button
            type="button"
            class="translate-switch"
            class:selected={currentLyricVariantActive}
            class:available={currentLyricHasTranslation || currentLyricHasPhonetic}
            disabled={!currentLyricHasTranslation && !currentLyricHasPhonetic}
            title={lyricVariantButtonTitle()}
            onclick={toggleLyricVariant}
          >
            <span>{lyricVariantButtonLabel()}</span>
          </button>
        </div>

        <div
          class="mask"
          class:slidedown={isNowPlaying}
          onclick={toggleNowPlaying}
          role="button"
          tabindex="0"
          onkeydown={(e) => runOnActionKey(e, toggleNowPlaying)}
          title="展开播放页"
        >
          <svg viewBox="0 0 32 32" fill="currentColor" stroke="none">
            <path d="M18.221 7.206 27.806 16.791c.879.879.879 2.317 0 3.195l-.8.801c-.877.878-2.316.878-3.194 0l-7.315-7.315-7.315 7.315c-.878.878-2.317.878-3.194 0l-.8-.801c-.879-.878-.879-2.316 0-3.195l9.587-9.585c.471-.472 1.103-.682 1.723-.647.617-.035 1.25.175 1.723.647z"/>
          </svg>
        </div>
      </div>
    </div>
  </div>

  {#if showQueue}
    <div class="menu-modal slideup" transition:fade={{ duration: 150 }} onclick={closeQueue} role="none"></div>
    <div class="menu slideup" transition:fly={{ y: 18, duration: 180 }} onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.stopPropagation()} role="dialog" tabindex="-1">
      <div class="menu-header">
        <span class="menu-title">共 {$playerState.playlist.length} 首</span>
        <button type="button" class="remove-all" onclick={() => player.clearPlaylist()}>
          <svg class="icon" viewBox="0 0 24 24" fill="currentColor" stroke="none">
            <path d="M9 3h6l1 2h5v2H3V5h5l1-2Zm1 7h2v8h-2v-8Zm4 0h2v8h-2v-8ZM6 8h12l-1 13H7L6 8Z"/>
          </svg>
          <span>清空</span>
        </button>
        <button type="button" class="close" onclick={closeQueue} aria-label="关闭队列">
          <svg viewBox="0 0 352 512" fill="currentColor" stroke="none">
            <path d="M242.72 256 342.79 155.93c12.28-12.28 12.28-32.19 0-44.48l-22.24-22.24c-12.28-12.28-32.19-12.28-44.48 0L176 189.28 75.93 89.21c-12.28-12.28-32.19-12.28-44.48 0L9.21 111.45c-12.28 12.28-12.28 32.19 0 44.48L109.28 256 9.21 356.07c-12.28 12.28-12.28 32.19 0 44.48l22.24 22.24c12.28 12.28 32.2 12.28 44.48 0L176 322.72l100.07 100.07c12.28 12.28 32.2 12.28 44.48 0l22.24-22.24c12.28-12.28 12.28-32.19 0-44.48L242.72 256z"/>
          </svg>
        </button>
      </div>
      <ul class="menu-list" bind:this={queueListEl}>
        {#if $playerState.playlist.length === 0}
          <li class="queue-empty">队列为空</li>
        {:else}
          {#each $playerState.playlist as track, i (`${track.id}-${i}`)}
            <!-- svelte-ignore a11y_no_noninteractive_element_to_interactive_role -->
            <li
              class:playing={i === $playerState.currentIndex}
              data-current={i === $playerState.currentIndex}
              onclick={() => playQueueIndex(i)}
              role="button"
              tabindex="0"
              onkeydown={(e) => runOnActionKey(e, () => playQueueIndex(i))}
            >
              <div class="song-status-icon">
                {#if i === $playerState.currentIndex}
                  <svg viewBox="0 0 448 512" fill="currentColor" stroke="none">
                    <path d="M424.4 214.7 72.4 6.6C43.8-10.3 0 6.1 0 47.9V464c0 37.5 40.7 60.1 72.4 41.3l352-208c31.4-18.5 31.5-64.1 0-82.6z"/>
                  </svg>
                {/if}
              </div>
              <div class="song-title truncate" class:disabled={track.disabled || track.url === ""}>{track.title}</div>
              <div class="song-singer truncate">{track.artist}</div>
              <div class="tools">
                <button type="button" class="icon" onclick={(e) => removeQueueIndex(i, e)} aria-label="移除">
                  <svg viewBox="0 0 24 24"><path d="M18 6 6 18M6 6l12 12"/></svg>
                </button>
                {#if track.source_url}
                  <button type="button" class="icon" onclick={(e) => { e.stopPropagation(); void openExternalUrl(track.source_url!); }} aria-label="打开来源">
                    <svg viewBox="0 0 24 24"><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/></svg>
                  </button>
                {/if}
              </div>
            </li>
          {/each}
        {/if}
      </ul>
    </div>
  {/if}

  {#if showAddMenu}
    <div class="menu-modal slideup add-modal" onclick={closeAddMenu} role="none"></div>
    <div class="add-menu" onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.stopPropagation()} role="dialog" tabindex="-1">
      <div class="add-menu-header">
        <strong>添加到歌单</strong>
        <button type="button" onclick={closeAddMenu} aria-label="关闭">
          <svg viewBox="0 0 24 24"><path d="M18 6 6 18M6 6l12 12"/></svg>
        </button>
      </div>
      {#if myPlaylists.length > 0}
        <div class="add-menu-list">
          {#each myPlaylists as pl}
            <button type="button" onclick={() => addCurrentToPlaylist(pl.id)}>{pl.title}</button>
          {/each}
        </div>
      {:else}
        <div class="add-menu-empty">还没有本地歌单</div>
      {/if}
      {#if addMenuCreateMode}
        <div class="add-menu-create">
          <input
            bind:value={addMenuCreateTitle}
            placeholder="歌单名称"
            onkeydown={(e) => {
              if (e.key === "Enter") createPlaylistAndAddCurrent();
              if (e.key === "Escape") addMenuCreateMode = false;
            }}
          />
          <button type="button" onclick={createPlaylistAndAddCurrent}>创建</button>
        </div>
      {:else}
        <button type="button" class="add-menu-new" onclick={() => addMenuCreateMode = true}>新建歌单</button>
      {/if}
    </div>
  {/if}
</div>

<style>
  .footer {
    --cover-accent-rgb: 1, 122, 254;
    --cover-accent: rgb(var(--cover-accent-rgb));
    /* 展开/收起的节奏统一在这里定义，`.footer-main` 的高度动画、`.songdetail-wrapper`
       的入场延迟（靠变量继承读到）和脚本里的兜底定时器都引用它。0.5s 对一次全屏展开
       来说太长，落定前的那段就是用户说的「慢一拍」；换成 300ms + 快出慢入。 */
    --player-expand-dur: 300ms;
    --player-expand-ease: cubic-bezier(0.22, 0.9, 0.24, 1);
    height: 100px;
    display: flex;
    align-items: flex-end;
    z-index: 130;
    margin: 1vh 1vw;
    border-radius: 10px;
    position: fixed;
    bottom: 0;
    /* 显式给 left，让两个状态用同一套定位方式：收起态原来是 left:auto（静态位置，
       解析结果同样是 0），展开态是 left:0 —— auto→0 不可插值，属于离散切换。
       写死 0 之后横向几何完全由 width + margin 决定，两者都能平滑过渡。 */
    left: 0;
    width: 98vw;
    /* width(98vw→100vw) 和 margin(1vh 1vw→0) 是布局属性，之前被摘出过渡就是因为
       它们每帧都会把整条播放器连同里面那页歌词重新布局，而歌词页底下垫着全屏
       48~72px 模糊的封面，跟着重算一遍 —— 那才是真正的开销。
       现在 `.songdetail-wrapper` 的盒子已经改成视口尺寸、跟父级尺寸解耦（见
       NowPlayingView），每帧重排的只剩播放条自己那几个 flex 项；常驻模糊也已经由
       `.footer-main.resizing` 在动画期间摘掉。所以这两个属性可以重新参与过渡：
       几何平滑，又不会把整页歌词拖进逐帧布局。 */
    transition:
      opacity var(--player-expand-dur) var(--player-expand-ease),
      bottom var(--player-expand-dur) var(--player-expand-ease),
      width var(--player-expand-dur) var(--player-expand-ease),
      margin var(--player-expand-dur) var(--player-expand-ease);
    color: var(--text-default-color);
  }

  .footer.adaptive {
    --theme-color: var(--cover-accent);
    --theme-color-ope: rgba(var(--cover-accent-rgb), 0.32);
    --theme-color-hover: rgba(var(--cover-accent-rgb), 0.18);
    --important-color: var(--cover-accent);
    --footer-player-bar-cur-background-color: var(--cover-accent);
    --footer-player-bar-cur-button-color: var(--cover-accent);
  }

  .footer svg[fill="currentColor"] {
    fill: currentColor;
  }

  .footer svg[stroke="none"] {
    stroke: none;
  }

  .footer svg[fill="currentColor"] path {
    fill: currentColor;
  }

  .footer.footerdef {
    opacity: 0;
    bottom: -140px;
    pointer-events: none;
  }

  .footer-main {
    position: relative;
    z-index: 140;
    height: 100px;
    border-radius: 10px;
    display: flex;
    flex: 1;
    /* 只让高度参与过渡。原来是 `0.5s` 简写（等于 all），而 backdrop-filter 也是可动画
       属性 —— 动画期间摘掉模糊、落定后装回去这两步都会被插值成 0.5s 的模糊半径动画，
       等于把想省掉的逐帧模糊又原样加了回来。这块只有 height 需要动。 */
    transition: height var(--player-expand-dur) var(--player-expand-ease);
    /* 注意：这里**不能**加 overflow: hidden。收起态的封面是 `top: -30px`，故意探出
       播放条上沿 30px；一裁就把封面切了。歌词页收起时靠自己的 opacity 退场，
       不依赖父级裁剪（见 NowPlayingView 的 .songdetail-wrapper）。 */
    /* 视觉档位变量（app.css 按 data-visual 定义）：
       high=毛玻璃恢复最佳效果；mid/low=半透明底色省掉常驻模糊合成 */
    -webkit-backdrop-filter: var(--visual-backdrop);
    backdrop-filter: var(--visual-backdrop);
    background-color: color-mix(in srgb, var(--nav-background-color) var(--footer-surface-alpha), transparent);
    border: 1px solid rgba(255, 255, 255, 0.08);
    box-shadow: 0 0 16px rgb(0 0 0 / 10%);
    border-top: solid 1px var(--line-default-color);
  }

  /* 液态玻璃表面接管时，footer 自己必须停掉 backdrop-filter：
     带 backdrop-filter 的元素会成为 backdrop root，子层就只能采到 footer 自身，
     采不到页面；底色也一并让给玻璃层，否则等于模糊两遍。 */
  .footer-main.glass-surface {
    -webkit-backdrop-filter: none;
    backdrop-filter: none;
    background-color: transparent;
  }

  /* 展开/收起动画期间摘掉所有常驻模糊（含展开态那层 footerwrap 的）。
     理由见 script 里 footerResizing 的注释：这块的高度每帧都在变，模糊面积一路涨到
     全屏，合成器跟不上，画面就落在状态后面。
     !important 是因为 .footer.expanded .footerwrap 跟这条同特异性但写在后面。 */
  .footer-main.resizing,
  .footer-main.resizing .footerwrap {
    -webkit-backdrop-filter: none !important;
    backdrop-filter: none !important;
  }

  .footer-main::before {
    content: "";
    position: absolute;
    inset: 0;
    pointer-events: none;
    opacity: 0;
    border-radius: inherit;
    background:
      radial-gradient(circle at 50% 100%, rgba(var(--cover-accent-rgb), 0.22), transparent 34%),
      linear-gradient(180deg, rgba(var(--cover-accent-rgb), 0.08), transparent 48%);
    transition: opacity 300ms ease;
  }

  .footer.adaptive .footer-main::before {
    opacity: 1;
  }

  .footer-main.slidedown {
    height: 100vh;
    border-radius: 10px;
  }

  .footer.adaptive .footer-main.slidedown {
    background:
      radial-gradient(circle at 28% 20%, rgba(var(--cover-accent-rgb), 0.22), transparent 34%),
      radial-gradient(circle at 74% 82%, rgba(var(--cover-accent-rgb), 0.14), transparent 32%),
      var(--nav-background-color);
    border-color: rgba(var(--cover-accent-rgb), 0.26);
    box-shadow:
      0 24px 56px rgba(0, 0, 0, 0.22),
      inset 0 1px 0 rgba(255, 255, 255, 0.12);
  }

  .footer.expanded {
    left: 0;
    right: 0;
    bottom: 0;
    width: 100vw;
    margin: 0;
    transform: none;
    z-index: 320;
  }

  /* ── 展开态的悬浮窗口控件 ──────────────────────────────
     展开后 .footer 是 z-index:320 的全屏层，把 .navigation（z-index:100）连同三个
     窗口按钮一起盖住、点都点不到，所以在播放器内部再挂一份。
     收起姿态：以顶边为铰链、朝屏幕里侧向上翻起 90°，等于贴着窗口顶边收平。
     命中区按"三个按钮实际占的区域"的 120% 算：原来是一条 220×10 的窄带，10px 高
     根本瞄不准，这就是用户说的"触发区域太小"。指针一划入就把带子长到能容纳翻下来的
     按钮，这样停在按钮上时 :hover 不会断掉又把它翻回去。
     副作用是好的：歌词区右上角那排微调/翻译按钮本来就会在翻出时整体下移让位
     （见 NowPlayingView 的 .variant-toggles.pushed-down），现在让位来得更早。 */
  .wc-flyout {
    /* 按钮区尺寸：单个按钮 = 18px 图标 + 6px padding ×2 = 30px；三个 = 90px，
       再加 margin-left 4px ×3 和 gap 2px ×2 = 106px 宽、30px 高。
       .wc-flyout-inner 还有 right:10px / padding-top:5px，所以相对右上角是 116×35。 */
    --wc-hit-w: 116px;
    --wc-hit-h: 35px;
    --wc-hit-scale: 1.2;
    position: absolute;
    top: 0;
    right: 0;
    width: calc(var(--wc-hit-w) * var(--wc-hit-scale));
    height: calc(var(--wc-hit-h) * var(--wc-hit-scale));
    /* 必须压过同级的 .songdetail-wrapper（z-index:100，定义在 NowPlayingView.svelte）。
       低于它时整条命中带都被歌曲详情盖住，hover 进不来，按钮量出来是 36×0、点击超时，
       而 TitleBar 那份又被 .footer(320) 盖着 —— 展开态就彻底没有可用的窗口控件了。 */
    z-index: 120;
    /* 透视必须给在命中区（父级）上，否则 rotateX 只是把按钮垂直压扁，没有铰链感 */
    perspective: 620px;
    perspective-origin: top center;
  }

  /* 窄窗按钮会收紧（见 WindowControls 的 media query），命中区跟着一起算小 */
  @media (max-width: 720px) {
    .wc-flyout {
      /* 单个按钮 = 16px 图标 + 5px padding ×2 = 26px，无 margin-left，gap 2px ×2 */
      --wc-hit-w: 92px;
      --wc-hit-h: 31px;
    }
  }

  .wc-flyout:hover {
    height: 54px;
  }

  .wc-flyout-inner {
    position: absolute;
    top: 0;
    right: 10px;
    display: flex;
    justify-content: flex-end;
    padding-top: 5px;
    transform-origin: top center;
    transform: rotateX(-90deg);
    opacity: 0;
    pointer-events: none;
    /* 收起时先转、快到 90° 才淡出，避免翻到一半就凭空消失 */
    transition:
      transform 320ms cubic-bezier(0.32, 0.72, 0.24, 1),
      opacity 110ms linear 210ms;
  }

  .wc-flyout:hover .wc-flyout-inner {
    transform: rotateX(0deg);
    opacity: 1;
    pointer-events: auto;
    /* 翻出时反过来：立刻可见，再补完剩下的转动 */
    transition:
      transform 340ms cubic-bezier(0.22, 0.9, 0.24, 1),
      opacity 90ms linear;
  }

  /* 最低档不做三维翻转，退化成纯透明度切换 */
  :global(.wrap[data-visual="low"]) .wc-flyout {
    perspective: none;
  }

  :global(.wrap[data-visual="low"]) .wc-flyout-inner,
  :global(.wrap[data-visual="low"]) .wc-flyout:hover .wc-flyout-inner {
    transform: none;
    transition: opacity 120ms linear;
  }

  @media (prefers-reduced-motion: reduce) {
    .wc-flyout-inner,
    .wc-flyout:hover .wc-flyout-inner {
      transform: none;
      transition: opacity 100ms linear;
    }
  }

  :global(.wrap[data-theme="liquid-glass"]) .footer-main:not(.slidedown) {
    background: rgba(246, 251, 255, 0.24);
    border-color: rgba(255, 255, 255, 0.38);
    box-shadow: 0 18px 44px rgba(58, 82, 99, 0.12), inset 0 1px 0 rgba(255,255,255,0.56);
  }

  .footerwrap {
    width: 100%;
    display: flex;
    height: 100px;
    position: absolute;
    bottom: 0;
    z-index: 2;
    border-radius: 0 0 10px 10px;
  }

  .footerwrap.switch-next .left-control .detail,
  .footerwrap.switch-next .main-info {
    animation: player-meta-next 220ms cubic-bezier(0.2, 0.72, 0.18, 1) both;
  }

  .footerwrap.switch-prev .left-control .detail,
  .footerwrap.switch-prev .main-info {
    animation: player-meta-prev 220ms cubic-bezier(0.2, 0.72, 0.18, 1) both;
  }

  .footer.expanded .footerwrap {
    background:
      linear-gradient(180deg, transparent, rgba(0, 0, 0, 0.10) 22%, rgba(0, 0, 0, 0.16)),
      var(--nav-background-color);
    border-top: 1px solid var(--line-default-color);
    -webkit-backdrop-filter: var(--visual-backdrop);
    backdrop-filter: var(--visual-backdrop);
  }

  .footer.expanded.adaptive .footerwrap {
    background:
      linear-gradient(180deg, rgba(var(--cover-accent-rgb), 0.03), rgba(var(--cover-accent-rgb), 0.14)),
      var(--nav-background-color);
    border-top-color: rgba(var(--cover-accent-rgb), 0.24);
  }

  .left-control {
    flex: 0 0 36%;
    display: flex;
    align-items: center;
    overflow: hidden;
    transition: 0.5s;
    opacity: 1;
  }

  .left-control.slidedown {
    flex: 0 0 0;
    opacity: 0;
    transform: scaleX(0);
  }

  .left-control .icon {
    display: flex;
    font-size: 22px;
    border-radius: 10px;
    padding: 7px;
    margin: 37px;
    transition: all 0.3s;
    background: transparent;
    cursor: pointer;
    color: var(--player-icon-color);
  }

  .left-control .icon:hover {
    background-color: var(--songlist-hover-background-color);
  }

  .left-control .icon.playlistactive {
    background-color: var(--theme-color-hover);
    color: var(--theme-color);
  }

  .left-control .splitter {
    height: 20px;
    width: 1px;
    display: inline-block;
    background: #a9a9a9;
  }

  .left-control .detail {
    max-width: 356px;
    margin-left: 37px;
    position: relative;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    justify-content: center;
  }

  .left-control .detail .title {
    color: var(--text-default-color);
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    margin: 5px 0;
    font-size: 18px;
    font-weight: 600;
  }

  .left-control .detail .more-info {
    margin: 5px 0;
    display: flex;
    color: var(--text-subtitle-color);
  }

  .left-control .detail .more-info .singer {
    flex: 1;
    font-size: 12px;
    min-width: 0;
  }

  .main-info {
    flex: 1;
    display: flex;
    justify-content: center;
    align-items: center;
    flex-direction: column;
    z-index: 110;
    min-width: 240px;
  }

  .logo-banner {
    text-align: center;
    flex: 1;
    display: flex;
    align-items: center;
  }

  .logo-banner svg.logo {
    height: 48px;
    width: 48px;
    fill: #666666;
    stroke: #666666;
    margin: 0 auto;
  }

  @keyframes rotatecircl {
    0% { transform: rotate(0deg); }
    100% { transform: rotate(360deg); }
  }

  .liplay {
    animation: rotatecircl 16s 0.5s infinite forwards linear;
    /* 显式提升成独立合成层。不提升的话这张 200px 源图每帧都要重新缩放到 90px
       再旋转，而且它外面套着 border-radius:50% + overflow:hidden 的圆形裁剪，
       每帧还要多一次带遮罩的合成——就是"封面旋转卡卡的"。
       提升之后纹理只光栅化一次，之后每帧只是改一个变换矩阵。 */
    will-change: transform;
    backface-visibility: hidden;
  }

  .lipause {
    animation-play-state: paused;
  }

  .cover {
    height: 90px;
    width: 90px;
    flex: 0 0 90px;
    object-fit: cover;
    position: relative;
    color: transparent;
    top: -30px;
    display: flex;
    justify-content: center;
  }

  .cover .cover-list {
    width: 220px;
    height: 90px;
    position: absolute;
  }

  .cover .cover-list span {
    bottom: 0;
    cursor: pointer;
    transition: 0.3s;
    color: var(--white--black);
    display: flex;
    justify-content: center;
    align-items: center;
    opacity: 0;
  }

  .cover .cover-list span > *,
  .cover .cover-list span svg,
  .cover .cover-list span path,
  .cover .cover-list span .spinner {
    pointer-events: none;
  }

  .cover .cover-list span:hover,
  .cover .cover-list span:focus-visible {
    opacity: 1;
    background-color: var(--white--black-background);
    outline: 1px solid var(--theme-color);
  }

  .cover:hover .cover-list span:not(.cover-image) {
    opacity: 1;
  }

  .cover .cover-list .b .pause-glyph {
    opacity: 0;
    transition: opacity 120ms ease;
  }

  .cover .cover-list .b:hover .pause-glyph,
  .cover .cover-list .b:focus-visible .pause-glyph {
    opacity: 1;
  }

  .cover .cover-list .a {
    height: 45px;
    width: 32px;
    left: 0;
    position: absolute;
    overflow: hidden;
    border-radius: 16px;
    opacity: 1;
    z-index: 100;
    display: flex;
  }

  .cover .cover-list .b {
    height: 90px;
    width: 90px;
    left: 65px;
    position: absolute;
    overflow: hidden;
    border-radius: 50%;
    opacity: 1;
    z-index: 101;
    display: flex;
  }

  .cover .cover-list .c {
    height: 45px;
    width: 32px;
    left: 190px;
    position: absolute;
    overflow: hidden;
    border-radius: 16px;
    opacity: 1;
    z-index: 99;
    display: flex;
  }

  .cover-list .a svg,
  .cover-list .c svg {
    width: 20px;
    height: 20px;
  }

  .cover-list .b svg {
    width: 30px;
    height: 30px;
  }

  .cover-stage {
    width: 220px;
    height: 90px;
    position: absolute;
    pointer-events: none;
  }

  .cover-stage .stage {
    position: absolute;
    bottom: 0;
    overflow: hidden;
    transition: opacity 0.18s, transform 0.18s;
    display: block;
  }

  .cover.cover-shift-next .cover-stage .stage.b {
    animation: cover-slide-next 220ms cubic-bezier(0.2, 0.72, 0.18, 1) both;
  }

  .cover.cover-shift-prev .cover-stage .stage.b {
    animation: cover-slide-prev 220ms cubic-bezier(0.2, 0.72, 0.18, 1) both;
  }

  .cover.cover-shift-next .cover-stage .stage.a,
  .cover.cover-shift-next .cover-stage .stage.c,
  .cover.cover-shift-prev .cover-stage .stage.a,
  .cover.cover-shift-prev .cover-stage .stage.c {
    animation: cover-side-pulse 220ms cubic-bezier(0.2, 0.72, 0.18, 1) both;
  }

  @keyframes cover-slide-next {
    0% { opacity: 0.46; transform: translateX(20px) scale(0.94); }
    100% { opacity: 1; transform: translateX(0) scale(1); }
  }

  @keyframes cover-slide-prev {
    0% { opacity: 0.46; transform: translateX(-20px) scale(0.94); }
    100% { opacity: 1; transform: translateX(0) scale(1); }
  }

  @keyframes cover-side-pulse {
    0% { opacity: 0.35; transform: translateY(4px) scale(0.88); }
    100% { opacity: 1; transform: translateY(0) scale(1); }
  }

  @keyframes player-meta-next {
    0% { opacity: 0.45; transform: translateX(14px); }
    100% { opacity: 1; transform: translateX(0); }
  }

  @keyframes player-meta-prev {
    0% { opacity: 0.45; transform: translateX(-14px); }
    100% { opacity: 1; transform: translateX(0); }
  }

  .cover-stage .stage.a {
    left: 0;
    height: 45px;
    width: 32px;
    border-radius: 16px;
  }

  .cover-stage .stage.b {
    left: 65px;
    height: 90px;
    width: 90px;
    border-radius: 50%;
  }

  .cover-stage .stage.c {
    left: 190px;
    height: 45px;
    width: 32px;
    border-radius: 16px;
  }

  .cover img,
  .cover-placeholder {
    height: 100%;
    width: 100%;
    object-fit: cover;
    box-sizing: border-box;
  }

  .cover-placeholder {
    display: block;
    border-radius: 50%;
    background: var(--button-background-color);
  }

  .circlemark {
    display: flex;
    justify-content: center;
    width: 100px;
    height: 50px;
    position: absolute;
    left: 50%;
    transform: translateX(-50%);
    top: 45px;
    z-index: -1;
    overflow: hidden;
    transform-origin: top center;
  }

  .circlemark .circle {
    width: 100px;
    height: 100px;
    position: relative;
    top: -50px;
    z-index: -1;
    overflow: hidden;
    transition: transform 0.1s linear;
  }

  .circlemark .topmark {
    width: 100px;
    height: 50px;
    z-index: -1;
    overflow: hidden;
  }

  .circlemark .top {
    width: 96px;
    height: 96px;
    z-index: -1;
    border-radius: 50%;
    border: 2px solid var(--text-default-color);
  }

  .circlemark .bottom {
    width: 100px;
    height: 50px;
    overflow: hidden;
  }

  .circlemark .bottomcircle {
    width: 96px;
    height: 96px;
    transform: translateY(-50px);
    z-index: -1;
    border-radius: 50%;
    border: 2px solid var(--footer-player-bar-background-color);
  }

  .footertime {
    position: relative;
    height: 49px;
    padding: 0;
    font-size: 12px;
    flex: 0 0 49px;
    cursor: default;
    font-weight: 500;
    display: flex;
    justify-content: center;
    align-items: center;
    flex-direction: column;
    width: 100%;
    max-width: 30vw;
    transition: opacity 0.18s ease;
  }

  .timeswitch {
    position: absolute;
    left: 0;
    right: 0;
    top: 0;
    height: 34px;
    display: flex;
    align-items: center;
    justify-content: center;
    opacity: 1;
    pointer-events: none;
    transform: translateY(0);
    /* 时间文本和进度条/音量占同一个盒子，纯 opacity 对切会被看成"内容一下换掉了"。
       给一段位移、让退场比入场快一点，读起来才是一次交接。 */
    transition: opacity 140ms ease-in, transform 220ms cubic-bezier(0.22, 0.9, 0.24, 1);
  }

  .bottomprogressbar {
    position: absolute;
    left: 0;
    right: 0;
    top: 0;
    height: 34px;
    justify-content: center;
    align-items: center;
    flex-wrap: nowrap;
    width: 100%;
    display: flex;
    opacity: 0;
    pointer-events: none;
    transform: translateY(8px);
    transition: opacity 220ms ease-out 40ms, transform 220ms cubic-bezier(0.22, 0.9, 0.24, 1) 40ms;
  }

  .footertime:hover .bottomprogressbar,
  .footertime:focus-within .bottomprogressbar {
    opacity: 1;
    pointer-events: auto;
    transform: translateY(0);
  }

  .footertime:hover .timeswitch,
  .footertime:focus-within .timeswitch {
    opacity: 0;
    transform: translateY(-8px);
  }

  @media (prefers-reduced-motion: reduce) {
    .timeswitch,
    .bottomprogressbar {
      transform: none !important;
      transition: opacity 120ms linear;
    }
  }

  .playbar {
    display: flex;
    justify-content: center;
    align-items: center;
    width: 50%;
  }

  .playbar .playbar-clickable {
    margin: 5px 10px 5px 0;
    padding: 5px 0;
    flex: 1;
    cursor: pointer;
  }

  .barbg {
    height: 3px;
    background: var(--footer-player-bar-background-color);
    position: relative;
  }

  .barbg .cur {
    height: 100%;
    width: 100%;
    background: var(--footer-player-bar-cur-background-color);
    /* clip-path 替代 width：进度更新不再触发布局，仅重绘该层。
       图层提升只在 high 档开启（mid/low 下常驻提升的显存不值这点收益） */
    will-change: var(--visual-will-change);
  }

  .barbg .btn {
    position: absolute;
    top: -5px;
    transform: translateX(-50%);
  }

  .playbar .playbar-clickable:hover .cur,
  .playbar .playbar-clickable:focus-visible .cur,
  .m-pbar:focus-visible .barbg .cur,
  .m-pbar:hover .barbg .cur {
    background: var(--theme-color);
  }

  .barbg .btn {
    background: var(--footer-player-bar-cur-button-color);
    height: 8px;
    width: 2px;
    transition: 0.3s;
  }

  .playbar .playbar-clickable:hover .barbg .btn,
  .playbar .playbar-clickable:focus-visible .barbg .btn,
  .m-pbar:focus-visible .barbg .btn,
  .m-pbar:hover .barbg .btn {
    width: 10px;
    height: 10px;
    border-radius: 5px;
    top: -3px;
  }

  .volume-ctrl {
    width: 50%;
    display: flex;
    justify-content: center;
    align-items: center;
  }

  .bottomprogressbar .icon {
    flex: 0 0 24px;
    color: var(--text-default-color);
    cursor: default;
    padding: 7px;
    display: flex;
  }

  .bottomprogressbar .icon svg {
    width: 18px;
    height: 18px;
  }

  .volume-ctrl .m-pbar {
    flex: 1;
    margin: 5px 0;
    padding: 5px 0;
    cursor: pointer;
    position: relative;
  }

  .right-control {
    flex: 0 0 36%;
    display: flex;
    align-items: center;
    justify-content: flex-end;
    min-width: 0;
  }

  .footer.expanded .right-control {
    flex: 0 0 28%;
  }

  .right-control .ctrl {
    display: flex;
    justify-content: center;
    align-items: center;
  }

  .right-control .ctrl a {
    margin-right: 32px;
    padding: 7px;
    display: flex;
    transition: 0.3s;
    border-radius: 10px;
    color: var(--player-right-icon-color);
  }

  .right-control .ctrl a:hover {
    text-decoration: none;
    background-color: var(--songlist-hover-background-color);
    color: var(--player-right-icon-hover-color);
  }

  .right-control .variant-switches {
    margin-right: 32px;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 4px;
    -webkit-app-region: no-drag;
  }

  .right-control .translate-switch {
    padding: 7px 4px;
    border-radius: 8px;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    -webkit-app-region: no-drag;
    height: 35px;
    box-sizing: border-box;
    width: 30px;
    transition: 0.3s;
    overflow: hidden;
    color: var(--player-right-icon-color);
    background: transparent;
    border: 0;
  }

  .right-control .translate-switch:hover {
    background-color: var(--songlist-hover-background-color);
  }

  .right-control .translate-switch.selected {
    color: var(--theme-color);
  }

  .right-control .translate-switch:not(.available) {
    opacity: 0.64;
    cursor: default;
  }

  .right-control .translate-switch span {
    font-size: 12px;
    font-weight: 900;
    line-height: 1;
    white-space: nowrap;
  }

  .right-control .mask {
    margin-right: 32px;
    padding: 7px;
    display: flex;
    transition: 0.3s;
    border-radius: 50%;
    color: var(--player-right-icon-color);
    cursor: pointer;
  }

  .right-control .mask.slidedown {
    transform: rotate(180deg);
  }

  .right-control .mask:hover {
    background-color: var(--songlist-hover-background-color);
  }

  .right-control .mask:focus-visible,
  .right-control .lyric-toggle:focus-visible,
  .right-control .translate-switch:focus-visible,
  .left-control .icon:focus-visible,
  .cover .cover-list span:focus-visible {
    outline: 1px solid var(--theme-color);
    outline-offset: 2px;
  }

  .right-control .lyric-toggle {
    margin-right: 32px;
    padding: 7px;
    display: flex;
    cursor: pointer;
    transition: 0.3s;
    border-radius: 10px;
    color: var(--player-right-icon-color);
  }

  .right-control .lyric-toggle:hover {
    background-color: var(--songlist-hover-background-color);
  }

  .right-control .lyric-toggle.selected {
    background-color: var(--theme-color-hover);
    color: var(--theme-color);
  }

  .right-control svg,
  .left-control svg {
    width: 18px;
    height: 18px;
  }

  .spinner {
    width: 20px;
    height: 20px;
    border: 2px solid rgba(127, 127, 127, 0.2);
    border-top-color: currentColor;
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .menu-modal {
    border-radius: 10px;
    transition: 0.3s;
    left: 0;
    right: 0;
    top: 0;
    position: fixed;
    opacity: 0;
    background: var(--shadow-mask);
  }

  .menu-modal.slideup {
    bottom: 0;
    opacity: 1;
    transition: 0.3s;
  }

  .menu {
    border-radius: 12px;
    position: absolute;
    z-index: 120;
    bottom: 120px;
    height: 0;
    opacity: 0;
    box-sizing: border-box;
    border: 1px solid var(--line-default-color);
    left: 0;
    -webkit-app-region: no-drag;
    transition: all 0.3s;
    overflow: hidden;
    width: 530px;
    -webkit-backdrop-filter: var(--visual-backdrop);
    backdrop-filter: var(--visual-backdrop);
    background:
      linear-gradient(180deg, rgba(255,255,255,0.04), transparent 42%),
      var(--nav-background-color);
    box-shadow: 0 24px 54px rgba(0, 0, 0, 0.22);
    padding-bottom: 14px;
  }

  .menu.slideup {
    bottom: 125px;
    height: 500px;
    opacity: 1;
    box-sizing: border-box;
    border: 1px solid var(--line-default-color);
  }

  .menu .menu-header {
    height: 58px;
    display: flex;
    align-items: center;
    color: var(--text-subtitle-color);
    padding: 14px 20px 10px;
    user-select: none;
  }

  .menu .menu-header .menu-title {
    flex: 1;
    padding: 0;
    font-size: 18px;
    font-weight: 800;
    color: var(--text-default-color);
  }

  .menu .menu-header .remove-all {
    all: unset;
    margin-left: 10px;
    flex: 0 0 auto;
    display: flex;
    align-items: center;
    font-size: 13px;
    cursor: pointer;
  }

  .menu .menu-header .remove-all:hover,
  .menu .menu-header .remove-all:hover span {
    text-decoration: none;
    color: var(--theme-color);
  }

  .menu .menu-header .remove-all .icon {
    margin-right: 7px;
    width: 18px;
    height: 18px;
  }

  .menu .menu-header .close {
    all: unset;
    margin-left: 15px;
    flex: 0 0 25px;
    align-items: center;
    cursor: pointer;
    color: var(--icon-default-color);
  }

  .menu .menu-header .close:hover {
    color: var(--theme-color);
  }

  .menu .menu-header .close svg {
    margin-right: 3px;
    width: 20px;
    height: 20px;
  }

  .menu ul.menu-list {
    overflow-y: scroll;
    height: 370px;
    padding: 0 14px 10px;
    font-size: 14px;
  }

  .menu ul.menu-list li {
    border-radius: 9px;
    display: flex;
    align-items: center;
    min-height: 44px;
    position: relative;
    margin-bottom: 4px;
    padding: 0 12px 0 4px;
    transition: background 0.16s, color 0.16s, transform 0.16s;
    cursor: pointer;
  }

  .menu ul.menu-list li:hover {
    background: var(--songlist-hover-background-color);
  }

  .menu ul.menu-list li.playing {
    color: var(--important-color);
    background: color-mix(in srgb, var(--theme-color) 14%, transparent);
    box-shadow: inset 3px 0 0 var(--theme-color);
  }

  .menu ul.menu-list li .song-status-icon {
    flex: 0 0 28px;
    width: 20px;
    height: 44px;
    text-align: center;
    display: flex;
    align-items: center;
  }

  .menu ul.menu-list li .song-status-icon svg {
    width: 10px;
    height: 10px;
    fill: var(--important-color);
    stroke: var(--important-color);
    flex: 1;
  }

  .menu ul.menu-list li .song-title {
    flex: 2;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    font-size: 15px;
    font-weight: 700;
    padding-right: 10px;
  }

  .menu ul.menu-list li .song-title.disabled {
    color: #777777;
  }

  .menu ul.menu-list li .song-singer {
    flex: 1;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    cursor: pointer;
    padding: 0 10px;
    font-weight: 500;
    color: var(--text-subtitle-color);
  }

  .menu ul.menu-list li .tools {
    flex: 0 0 42px;
    width: 42px;
    display: flex;
  }

  .menu ul.menu-list li .tools .icon {
    all: unset;
    cursor: pointer;
    opacity: 0.55;
    width: 16px;
    height: 16px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }

  .menu ul.menu-list li .tools .icon:first-of-type {
    margin-right: 5px;
  }

  .menu ul.menu-list li .tools .icon:hover {
    opacity: 1;
  }

  .menu ul.menu-list li .tools svg {
    width: 14px;
    height: 14px;
  }

  .queue-empty {
    justify-content: center;
    color: var(--text-subtitle-color);
  }

  .add-modal {
    z-index: 180;
  }

  .add-menu {
    position: absolute;
    right: 44px;
    bottom: 125px;
    z-index: 190;
    width: 300px;
    max-height: min(420px, calc(100dvh - 160px));
    overflow: hidden;
    border: 1px solid var(--line-default-color);
    border-radius: 12px;
    background:
      linear-gradient(180deg, rgba(255,255,255,0.05), transparent 42%),
      var(--nav-background-color);
    -webkit-backdrop-filter: var(--visual-backdrop);
    backdrop-filter: var(--visual-backdrop);
    box-shadow: 0 24px 54px rgba(0, 0, 0, 0.22);
    padding: 10px;
    color: var(--text-default-color);
  }

  .add-menu-header {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 4px 4px 10px;
  }

  .add-menu-header strong {
    flex: 1;
    font-size: 15px;
  }

  .add-menu-header button,
  .add-menu-list button,
  .add-menu-new,
  .add-menu-create button {
    border: 0;
    color: inherit;
    background: transparent;
    cursor: pointer;
  }

  .add-menu-header button {
    width: 28px;
    height: 28px;
    border-radius: 8px;
    display: grid;
    place-items: center;
  }

  .add-menu-header button:hover,
  .add-menu-list button:hover,
  .add-menu-new:hover {
    background: var(--songlist-hover-background-color);
  }

  .add-menu-header svg {
    width: 16px;
    height: 16px;
  }

  .add-menu-list {
    max-height: 250px;
    overflow: auto;
    padding: 2px 0;
  }

  .add-menu-list button,
  .add-menu-new {
    width: 100%;
    min-height: 36px;
    border-radius: 8px;
    padding: 0 10px;
    text-align: left;
    font-size: 13px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .add-menu-empty {
    padding: 14px 10px;
    color: var(--text-subtitle-color);
    font-size: 13px;
  }

  .add-menu-create {
    display: flex;
    gap: 8px;
    padding-top: 8px;
  }

  .add-menu-create input {
    min-width: 0;
    flex: 1;
    height: 34px;
    border: 1px solid var(--line-default-color);
    border-radius: 8px;
    padding: 0 10px;
    background: var(--button-background-color);
    color: var(--text-default-color);
  }

  .add-menu-create button {
    height: 34px;
    padding: 0 12px;
    border-radius: 8px;
    background: var(--theme-color);
    color: #fff;
    font-size: 13px;
    font-weight: 700;
  }

  @media (max-width: 980px) and (min-width: 721px) {
    .left-control {
      flex: 1 1 30%;
      min-width: 0;
    }

    .left-control .icon {
      margin: 32px 18px;
      flex: 0 0 auto;
    }

    .left-control .detail {
      margin-left: 18px;
      max-width: none;
      min-width: 0;
    }

    .main-info {
      flex: 0 0 clamp(190px, 26vw, 240px);
      min-width: 190px;
    }

    .footertime {
      max-width: 34vw;
    }

    .right-control {
      flex: 1 1 30%;
      min-width: 0;
    }

    .footer.expanded .right-control {
      flex: 1 1 24%;
    }

    .right-control .ctrl a,
    .right-control .lyric-toggle,
    .right-control .variant-switches,
    .right-control .mask {
      margin-right: 14px;
    }
  }

  @media (max-width: 720px) {
    .footer {
      height: 88px;
      width: calc(100vw - 16px);
      margin: 0 8px calc(var(--safe-bottom) + 8px);
      border-radius: 12px;
    }

    .footer-main {
      height: 88px;
      border-radius: 12px;
    }

    .footer-main.slidedown {
      height: calc(100dvh - var(--safe-top) - var(--safe-bottom) - 16px);
    }

    .footerwrap {
      height: 88px;
      display: grid;
      grid-template-columns: minmax(0, 1fr) 148px 42px;
      align-items: center;
      border-radius: 0 0 12px 12px;
    }

    .left-control {
      min-width: 0;
      overflow: hidden;
    }

    .left-control.slidedown {
      display: none;
    }

    .left-control .icon {
      margin: 0 10px 0 12px;
      padding: 6px;
      flex: 0 0 auto;
    }

    .left-control .splitter {
      display: none;
    }

    .left-control .detail {
      margin-left: 0;
      max-width: none;
      min-width: 0;
    }

    .left-control .detail .title {
      font-size: 14px;
      margin: 2px 0;
    }

    .left-control .detail .more-info {
      margin: 2px 0;
    }

    .main-info {
      min-width: 0;
      width: 148px;
      justify-self: center;
    }

    .cover {
      width: 64px;
      height: 64px;
      flex: 0 0 64px;
      top: -8px;
    }

    .cover .cover-list,
    .cover-stage {
      width: 148px;
      height: 64px;
    }

    .cover .cover-list .a,
    .cover-stage .stage.a {
      width: 28px;
      height: 40px;
      left: 0;
      border-radius: 14px;
    }

    .cover .cover-list .b,
    .cover-stage .stage.b {
      width: 64px;
      height: 64px;
      left: 42px;
    }

    .cover .cover-list .c,
    .cover-stage .stage.c {
      width: 28px;
      height: 40px;
      left: 120px;
      border-radius: 14px;
    }

    .cover-list .a svg,
    .cover-list .c svg {
      width: 16px;
      height: 16px;
    }

    .cover-list .b svg {
      width: 24px;
      height: 24px;
    }

    .circlemark {
      display: none;
    }

    .footertime {
      display: none;
    }

    .right-control {
      flex: none;
      justify-content: center;
      min-width: 0;
    }

    .right-control .ctrl,
    .right-control .lyric-toggle,
    .right-control .variant-switches {
      display: none;
    }

    .right-control .mask {
      margin-right: 0;
      padding: 8px;
    }

    .footer.expanded .footerwrap {
      grid-template-columns: 1fr 148px 42px;
    }

    .menu {
      left: 8px;
      right: 8px;
      bottom: calc(104px + var(--safe-bottom));
      width: auto;
      max-width: none;
    }

    .menu.slideup {
      bottom: calc(108px + var(--safe-bottom));
      height: min(460px, calc(100dvh - 180px - var(--safe-bottom)));
    }

    .menu ul.menu-list {
      height: calc(100% - 90px);
      padding: 0 16px;
    }

    .add-menu {
      left: 8px;
      right: 8px;
      bottom: calc(108px + var(--safe-bottom));
      width: auto;
      max-height: min(420px, calc(100dvh - 150px - var(--safe-bottom)));
    }
  }
</style>
