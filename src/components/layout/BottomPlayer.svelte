<script lang="ts">
  import { playerState, progressPercent, positionFormatted, durationFormatted } from "../../lib/stores/player";
  import { player } from "../../lib/player";
  import { settings } from "../../lib/stores/settings";
  import { MediaService, myplaylistLib } from "../../lib/providers/index";
  import { proxyResourceUrl } from "../../lib/resourceUrl";
  import { runOnActionKey } from "../../lib/keyboard";
  import { toast } from "../../lib/stores/toast";
  import LiquidGlassSurface from "../effects/LiquidGlassSurface.svelte";
  import NowPlayingView from "../views/NowPlayingView.svelte";
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
  let currentCoverUrl = $derived(proxyResourceUrl($playerState.currentTrack?.img_url));
  let coverAccentStyle = $derived.by(() => {
    if (!$settings.enableCoverAdaptiveTheme || !adaptiveAccent) return "";
    const { r, g, b } = adaptiveAccent;
    return `--cover-accent-rgb:${r},${g},${b};--cover-accent:rgb(${r} ${g} ${b});`;
  });
  let footerMainEl = $state<HTMLElement | null>(null);
  let glassSurfaceEnabled = $derived(
    $settings.theme === "liquidGlass" &&
    !isNowPlaying &&
    Boolean($playerState.currentTrack) &&
    !$playerState.playing
  );
  let prevCoverUrl = $derived.by(() => {
    const list = $playerState.playlist;
    const index = $playerState.currentIndex;
    if (!list.length || index < 0) return "";
    return proxyResourceUrl(list[(index - 1 + list.length) % list.length]?.img_url);
  });
  let nextCoverUrl = $derived.by(() => {
    const list = $playerState.playlist;
    const index = $playerState.currentIndex;
    if (!list.length || index < 0) return "";
    return proxyResourceUrl(list[(index + 1) % list.length]?.img_url);
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
  <div class="footer-main" class:slidedown={isNowPlaying} bind:this={footerMainEl}>
    <LiquidGlassSurface
      target={footerMainEl}
      enabled={glassSurfaceEnabled}
      variant="liquid"
      backgroundSelector="#listen1-glass-scene"
    />
    <NowPlayingView visible={nowPlayingOpen} onClose={onCloseNowPlaying} />

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
                  <div class="cur" style="width:{isDragging ? dragPercent : $progressPercent}%">
                    <span class="btn"><i></i></span>
                  </div>
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
                  <div class="cur" style="width:{$playerState.volume}%">
                    <span class="btn"><i></i></span>
                  </div>
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
                  <button type="button" class="icon" onclick={(e) => { e.stopPropagation(); window.open(track.source_url, "_blank"); }} aria-label="打开来源">
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
    height: 100px;
    display: flex;
    align-items: flex-end;
    z-index: 130;
    margin: 1vh 1vw;
    border-radius: 10px;
    position: fixed;
    bottom: 0;
    width: 98vw;
    transition: 0.5s;
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
    transition: 0.5s;
    pointer-events: none;
  }

  .footer-main {
    position: relative;
    z-index: 140;
    height: 100px;
    border-radius: 10px;
    display: flex;
    flex: 1;
    transition: 0.5s;
    -webkit-backdrop-filter: saturate(180%) blur(20px);
    backdrop-filter: saturate(180%) blur(20px);
    background-color: var(--nav-background-color);
    border: 1px solid rgba(255, 255, 255, 0.08);
    box-shadow: 0 0 16px rgb(0 0 0 / 10%);
    border-top: solid 1px var(--line-default-color);
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
    -webkit-backdrop-filter: saturate(180%) blur(20px);
    backdrop-filter: saturate(180%) blur(20px);
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
    transition: opacity 180ms ease, transform 180ms ease;
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
    transform: translateY(3px);
    transition: opacity 180ms ease, transform 180ms ease;
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
    transform: translateY(-3px);
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
  }

  .barbg .cur {
    height: 100%;
    background: var(--footer-player-bar-cur-background-color);
    position: relative;
  }

  .playbar .playbar-clickable:hover .cur,
  .playbar .playbar-clickable:focus-visible .cur,
  .m-pbar:focus-visible .barbg .cur,
  .m-pbar:hover .barbg .cur {
    background: var(--theme-color);
  }

  .barbg .cur .btn {
    background: var(--footer-player-bar-cur-button-color);
    height: 8px;
    width: 2px;
    position: absolute;
    right: -2px;
    top: -5px;
    transition: 0.3s;
  }

  .playbar .playbar-clickable:hover .barbg .cur .btn,
  .playbar .playbar-clickable:focus-visible .barbg .cur .btn,
  .m-pbar:focus-visible .barbg .cur .btn,
  .m-pbar:hover .barbg .cur .btn {
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
    -webkit-backdrop-filter: saturate(180%) blur(20px);
    backdrop-filter: saturate(180%) blur(20px);
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
    -webkit-backdrop-filter: saturate(180%) blur(20px);
    backdrop-filter: saturate(180%) blur(20px);
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
