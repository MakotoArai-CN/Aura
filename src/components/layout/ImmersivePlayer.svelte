<script lang="ts">
  import { playerState, progressPercent, positionFormatted, durationFormatted } from "../../lib/stores/player";
  import { player } from "../../lib/player";
  import { settings } from "../../lib/stores/settings";
  import { cssImageUrl, proxyResourceUrl } from "../../lib/resourceUrl";
  import { runOnActionKey } from "../../lib/keyboard";
  import NowPlayingView from "../views/NowPlayingView.svelte";
  import { fade, fly } from "svelte/transition";

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

  const LOOP_ICONS = [
    `<path d="M17 1l4 4-4 4"/><path d="M3 11V9a4 4 0 0 1 4-4h14"/><path d="M7 23l-4-4 4-4"/><path d="M21 13v2a4 4 0 0 1-4 4H3"/>`,
    `<polyline points="16 3 21 3 21 8"/><line x1="4" y1="20" x2="21" y2="3"/><polyline points="21 16 21 21 16 21"/><line x1="15" y1="15" x2="21" y2="21"/>`,
    `<path d="M17 1l4 4-4 4"/><path d="M3 11V9a4 4 0 0 1 4-4h14"/><path d="M7 23l-4-4 4-4"/><path d="M21 13v2a4 4 0 0 1-4 4H3"/><line x1="12" y1="8" x2="12" y2="16"/>`,
  ];
  const LOOP_TITLES = ["顺序播放", "单曲循环", "随机播放"];
  const SPECTRUM_BARS = [0, 1, 2, 3, 4, 5];

  let isDragging = $state(false);
  let dragPercent = $state(0);
  let showQueue = $state(false);
  let isNowPlaying = $derived(activeView.type === "nowplaying");
  let currentCoverUrl = $derived(proxyResourceUrl($playerState.currentTrack?.img_url));
  let coverBackground = $derived(cssImageUrl($playerState.currentTrack?.img_url));
  let coverSwitchDirection = $state<"next" | "prev" | "">("");
  let lastCoverIndex = -1;
  let lastCoverTrackId = "";
  let displayedProgress = $derived(Math.max(0, Math.min(100, isDragging ? dragPercent : $progressPercent)));
  let displayedVolume = $derived(Math.max(0, Math.min(100, $playerState.muted ? 0 : $playerState.volume)));
  let hasPlayableTarget = $derived(Boolean($playerState.currentTrack) || $playerState.playlist.length > 0);

  function percentFromPointer(el: HTMLElement, clientX: number) {
    const rect = el.getBoundingClientRect();
    if (rect.width <= 0) return 0;
    return Math.max(0, Math.min(100, ((clientX - rect.left) / rect.width) * 100));
  }

  function handleProgressPointerDown(e: PointerEvent) {
    const target = e.currentTarget as HTMLElement;
    isDragging = true;
    dragPercent = percentFromPointer(target, e.clientX);
    e.preventDefault();

    const onMove = (event: PointerEvent) => {
      dragPercent = percentFromPointer(target, event.clientX);
    };
    const onDone = () => {
      player.seek(dragPercent);
      isDragging = false;
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onDone);
      window.removeEventListener("pointercancel", onDone);
    };

    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onDone);
    window.addEventListener("pointercancel", onDone);
  }

  function handleProgressKeydown(e: KeyboardEvent) {
    if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(e.key)) return;
    e.preventDefault();
    const next = e.key === "Home"
      ? 0
      : e.key === "End"
        ? 100
        : Math.max(0, Math.min(100, $progressPercent + (e.key === "ArrowRight" ? 2 : -2)));
    player.seek(next);
  }

  function setVolumePercent(value: number, persist = false) {
    player.setVolume(Math.max(0, Math.min(100, value)), persist);
    if ($playerState.muted) player.unmute();
  }

  function handleVolumePointerDown(e: PointerEvent) {
    const target = e.currentTarget as HTMLElement;
    const update = (event: PointerEvent) => setVolumePercent(percentFromPointer(target, event.clientX));
    e.preventDefault();
    update(e);

    const onMove = (event: PointerEvent) => update(event);
    const onDone = () => {
      player.commitVolume();
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onDone);
      window.removeEventListener("pointercancel", onDone);
    };

    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onDone);
    window.addEventListener("pointercancel", onDone);
  }

  function handleVolumeWheel(e: WheelEvent) {
    e.preventDefault();
    player.adjustVolume(e.deltaY < 0);
    if ($playerState.muted) player.unmute();
  }

  function handleVolumeKeydown(e: KeyboardEvent) {
    if (!["ArrowLeft", "ArrowRight", "ArrowDown", "ArrowUp", "Home", "End"].includes(e.key)) return;
    e.preventDefault();
    const next = e.key === "Home"
      ? 0
      : e.key === "End"
        ? 100
        : $playerState.volume + (e.key === "ArrowRight" || e.key === "ArrowUp" ? 5 : -5);
    setVolumePercent(next, true);
  }

  function toggleNowPlaying() {
    if (isNowPlaying) onCloseNowPlaying();
    else navigate({ type: "nowplaying" });
  }

  function playQueueIndex(index: number) {
    player.loadByIndex(index);
    showQueue = false;
  }

  function removeQueueIndex(index: number, e: MouseEvent) {
    e.stopPropagation();
    player.removeTrack(index);
  }

  function toggleFloatingLyric() {
    // 仅切换开关，浮窗显隐由 LyricSync 统一决策（显式开启会立即显示以给出反馈）。
    settings.patch({ enableLyricFloatingWindow: !$settings.enableLyricFloatingWindow });
  }

  $effect(() => {
    const track = $playerState.currentTrack;
    if (track) {
      document.title = $playerState.playing
        ? `▶ ${track.title} - ${track.artist}`
        : `${track.title} - ${track.artist}`;
    }
  });

  $effect(() => {
    activeView.type;
    showQueue = false;
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
  class="immersive-player"
  class:empty={!$playerState.currentTrack}
  class:expanded={isNowPlaying}
  data-immersive-player
>
  <div class="native-shell" style:--cover-image={coverBackground}>
    <div class="cover-wash" aria-hidden="true"></div>
    <div class="surface-grid" aria-hidden="true"></div>

    <NowPlayingView visible={nowPlayingOpen} onClose={onCloseNowPlaying} />

    {#if showQueue && !isNowPlaying}
      <button type="button" class="queue-scrim" transition:fade={{ duration: 150 }} aria-label="关闭队列" onclick={() => showQueue = false}></button>
      <div
        class="queue-panel"
        transition:fly={{ y: 18, duration: 180 }}
        role="dialog"
        tabindex="-1"
        aria-label="播放队列"
        onclick={(e) => e.stopPropagation()}
        onkeydown={(e) => e.stopPropagation()}
      >
        <header>
          <div>
            <strong>播放队列</strong>
            <span>{$playerState.playlist.length} 首</span>
          </div>
          <button type="button" class="text-action" onclick={() => player.clearPlaylist()}>清空</button>
          <button type="button" class="icon-button small" aria-label="关闭队列" onclick={() => showQueue = false}>
            <svg viewBox="0 0 24 24"><path d="M18 6 6 18M6 6l12 12"/></svg>
          </button>
        </header>

        <ul class="queue-list">
          {#if $playerState.playlist.length === 0}
            <li class="queue-empty">队列为空</li>
          {:else}
            {#each $playerState.playlist as track, i (`${track.id}-${i}`)}
              <!-- svelte-ignore a11y_no_noninteractive_element_to_interactive_role -->
              <li
                class:playing={i === $playerState.currentIndex}
                onclick={() => playQueueIndex(i)}
                role="button"
                tabindex="0"
                onkeydown={(e) => runOnActionKey(e, () => playQueueIndex(i))}
              >
                <span class="queue-index">{String(i + 1).padStart(2, "0")}</span>
                <div class="queue-meta">
                  <span class="queue-title" class:disabled={track.disabled || track.url === ""}>{track.title}</span>
                  <span class="queue-artist">{track.artist}</span>
                </div>
                <button type="button" class="icon-button small ghost" aria-label="移除" onclick={(e) => removeQueueIndex(i, e)}>
                  <svg viewBox="0 0 24 24"><path d="M18 6 6 18M6 6l12 12"/></svg>
                </button>
                {#if i === $playerState.currentIndex}
                  <div class="queue-spectrum" class:active={$playerState.playing} aria-hidden="true">
                    {#each SPECTRUM_BARS as bar}
                      <span style:--bar-index={bar}></span>
                    {/each}
                  </div>
                {/if}
              </li>
            {/each}
          {/if}
        </ul>
      </div>
    {/if}

    <div class="player-bar" class:switch-next={coverSwitchDirection === "next"} class:switch-prev={coverSwitchDirection === "prev"}>
      <div class="track-zone">
        <button
          type="button"
          class="queue-button"
          class:active={showQueue}
          aria-label="播放队列"
          onclick={() => showQueue = !showQueue}
        >
          <svg viewBox="0 0 24 24"><path d="M4 7h10M4 12h10M4 17h7"/><path d="M18 8v8l4-4-4-4Z"/></svg>
        </button>

        <div class="cover-card" class:playing={$playerState.playing} class:switch-next={coverSwitchDirection === "next"} class:switch-prev={coverSwitchDirection === "prev"}>
          {#if currentCoverUrl}
            <img src={currentCoverUrl} alt="cover" />
          {:else}
            <svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="9"/><circle cx="12" cy="12" r="3"/><path d="M12 3v6"/></svg>
          {/if}
        </div>

        <div class="track-meta">
          {#if $playerState.currentTrack}
            <strong>{$playerState.currentTrack.title}</strong>
            <span>
              {$playerState.currentTrack.artist}
              {#if $playerState.currentTrack.album}
                · {$playerState.currentTrack.album}
              {/if}
            </span>
          {/if}
        </div>
      </div>

      <div class="control-zone">
        <div class="transport">
          <button type="button" class="transport-button" aria-label="上一首" onclick={() => player.skip("prev")}>
            <svg viewBox="0 0 24 24" fill="currentColor" stroke="none"><path d="M6 5h2v14H6zM19 6.5v11L10 12z"/></svg>
          </button>
          <button
            type="button"
            class="play-button"
            class:loading={$playerState.loading}
            aria-label={$playerState.playing ? "暂停" : "播放"}
            onclick={() => player.togglePlayPause()}
            disabled={!hasPlayableTarget}
          >
            {#if $playerState.loading}
              <span class="spinner"></span>
            {:else if $playerState.playing}
              <svg viewBox="0 0 24 24" fill="currentColor" stroke="none"><path d="M7 5h4v14H7zM13 5h4v14h-4z"/></svg>
            {:else}
              <svg viewBox="0 0 24 24" fill="currentColor" stroke="none"><path d="M8 5v14l11-7z"/></svg>
            {/if}
          </button>
          <button type="button" class="transport-button" aria-label="下一首" onclick={() => player.skip("next")}>
            <svg viewBox="0 0 24 24" fill="currentColor" stroke="none"><path d="M16 5h2v14h-2zM5 6.5v11l9-5.5z"/></svg>
          </button>
        </div>

        <div class="timeline-switch">
          <div class="time-readout">
            <span>{$positionFormatted}</span>
            <span>/</span>
            <span>{$durationFormatted}</span>
          </div>
          <div class="hover-controls">
            <div
              class="scrubber progress"
              onpointerdown={handleProgressPointerDown}
              onkeydown={handleProgressKeydown}
              role="slider"
              tabindex="0"
              aria-label="播放进度"
              aria-valuemin="0"
              aria-valuemax="100"
              aria-valuenow={displayedProgress}
            >
              <div class="rail">
                <div class="fill" style:width={`${displayedProgress}%`}>
                  <span></span>
                </div>
              </div>
            </div>

            <div class="volume-control" onwheel={handleVolumeWheel}>
              <button type="button" class="volume-button" aria-label={$playerState.muted ? "取消静音" : "静音"} onclick={() => player.toggleMute()}>
                {#if $playerState.muted || $playerState.volume === 0}
                  <svg viewBox="0 0 24 24"><path d="M11 5 6 9H2v6h4l5 4V5Z"/><path d="m22 9-6 6M16 9l6 6"/></svg>
                {:else}
                  <svg viewBox="0 0 24 24"><path d="M11 5 6 9H2v6h4l5 4V5Z"/><path d="M15.5 8.5a5 5 0 0 1 0 7M18.5 5.5a9 9 0 0 1 0 13"/></svg>
                {/if}
              </button>
              <div
                class="scrubber volume"
                onpointerdown={handleVolumePointerDown}
                onkeydown={handleVolumeKeydown}
                role="slider"
                tabindex="0"
                aria-label="音量"
                aria-valuemin="0"
                aria-valuemax="100"
                aria-valuenow={displayedVolume}
              >
                <div class="rail">
                  <div class="fill" style:width={`${displayedVolume}%`}>
                    <span></span>
                  </div>
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>

      <div class="right-zone">
        <button
          type="button"
          class="icon-button"
          aria-label={LOOP_TITLES[$playerState.loopMode]}
          title={LOOP_TITLES[$playerState.loopMode]}
          onclick={() => player.loopMode = (($playerState.loopMode + 1) % 3) as 0 | 1 | 2}
        >
          <svg viewBox="0 0 24 24">
            {@html LOOP_ICONS[$playerState.loopMode]}
          </svg>
        </button>

        <button
          type="button"
          class="icon-button"
          class:active={$settings.enableLyricFloatingWindow}
          aria-label="桌面歌词"
          title="桌面歌词"
          onclick={toggleFloatingLyric}
        >
          <svg viewBox="0 0 24 24"><rect x="4" y="4" width="16" height="16" rx="2"/><path d="M8 9h8M8 13h5"/></svg>
        </button>

        <button
          type="button"
          class="icon-button expand-button"
          class:expanded={isNowPlaying}
          aria-label={isNowPlaying ? "收起播放页" : "展开播放页"}
          onclick={toggleNowPlaying}
        >
          <svg viewBox="0 0 24 24"><path d="m6 15 6-6 6 6"/></svg>
        </button>
      </div>
    </div>
  </div>
</div>

<style>
  .immersive-player {
    position: fixed;
    left: 16px;
    right: 16px;
    bottom: 14px;
    height: 112px;
    z-index: 150;
    color: #f5f7fb;
    pointer-events: none;
    transition:
      left 900ms cubic-bezier(0.16, 1, 0.3, 1),
      right 900ms cubic-bezier(0.16, 1, 0.3, 1),
      bottom 900ms cubic-bezier(0.16, 1, 0.3, 1),
      height 900ms cubic-bezier(0.16, 1, 0.3, 1),
      opacity 320ms ease,
      transform 640ms cubic-bezier(0.16, 1, 0.3, 1);
    contain: layout paint;
  }

  .immersive-player.empty {
    opacity: 0;
    transform: translateY(136px);
    pointer-events: none;
  }

  .immersive-player.expanded {
    left: 0;
    right: 0;
    bottom: 0;
    height: 100vh;
    z-index: 320;
  }

  .native-shell {
    --cover-image: none;
    position: relative;
    width: 100%;
    height: 100%;
    overflow: hidden;
    pointer-events: auto;
    border-radius: 28px;
    border: 1px solid rgba(255, 255, 255, 0.16);
    background:
      radial-gradient(circle at 50% 0%, rgba(var(--immersive-player-accent-rgb), 0.12), transparent 36%),
      linear-gradient(135deg, rgba(28, 31, 38, 0.92), rgba(13, 15, 20, 0.88) 52%, rgba(10, 12, 17, 0.94));
    box-shadow:
      var(--immersive-player-panel-shadow),
      0 20px 60px rgba(0, 0, 0, 0.36);
    -webkit-backdrop-filter: saturate(150%) blur(18px);
    backdrop-filter: saturate(150%) blur(18px);
    transition:
      border-radius 900ms cubic-bezier(0.16, 1, 0.3, 1),
      background 900ms cubic-bezier(0.16, 1, 0.3, 1),
      box-shadow 900ms cubic-bezier(0.16, 1, 0.3, 1);
  }

  .immersive-player.expanded .native-shell {
    border-radius: 0;
    border-color: transparent;
    background:
      radial-gradient(circle at 22% 16%, rgba(var(--immersive-player-accent-rgb), 0.16), transparent 30%),
      radial-gradient(circle at 74% 72%, rgba(93, 95, 239, 0.12), transparent 32%),
      linear-gradient(135deg, rgba(18, 20, 27, 0.98), rgba(7, 9, 14, 0.98));
    box-shadow: none;
  }

  .cover-wash {
    position: absolute;
    inset: -18%;
    pointer-events: none;
    background-image: var(--cover-image);
    background-position: center;
    background-size: cover;
    opacity: 0.12;
    transform: scale(1.02);
    transition: opacity 900ms ease;
  }

  .cover-wash::after {
    content: "";
    position: absolute;
    inset: 0;
    background:
      linear-gradient(90deg, rgba(8, 10, 15, 0.92), rgba(8, 10, 15, 0.54), rgba(8, 10, 15, 0.92)),
      radial-gradient(circle at 50% 44%, transparent, rgba(0, 0, 0, 0.58));
  }

  .immersive-player.expanded .cover-wash {
    opacity: 0.18;
  }

  .surface-grid {
    position: absolute;
    inset: 0;
    pointer-events: none;
    opacity: 0.24;
    background-image:
      linear-gradient(rgba(255, 255, 255, 0.045) 1px, transparent 1px),
      linear-gradient(90deg, rgba(255, 255, 255, 0.035) 1px, transparent 1px);
    background-size: 36px 36px;
    mask-image: linear-gradient(to top, rgba(0,0,0,0.8), transparent 74%);
  }

  .player-bar {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    height: 112px;
    display: grid;
    grid-template-columns: minmax(260px, 1.05fr) minmax(320px, 0.9fr) minmax(220px, 1fr);
    align-items: center;
    gap: 18px;
    padding: 16px 24px;
    z-index: 120;
    -webkit-app-region: no-drag;
    transition:
      height 900ms cubic-bezier(0.16, 1, 0.3, 1),
      background 900ms cubic-bezier(0.16, 1, 0.3, 1),
      border-color 900ms cubic-bezier(0.16, 1, 0.3, 1);
  }

  .player-bar.switch-next .track-zone,
  .player-bar.switch-next .control-zone {
    animation: immersive-meta-next 220ms cubic-bezier(0.2, 0.72, 0.18, 1) both;
  }

  .player-bar.switch-prev .track-zone,
  .player-bar.switch-prev .control-zone {
    animation: immersive-meta-prev 220ms cubic-bezier(0.2, 0.72, 0.18, 1) both;
  }

  .immersive-player.expanded .player-bar {
    background: linear-gradient(180deg, transparent, rgba(4, 6, 10, 0.62) 38%, rgba(4, 6, 10, 0.9));
    border-top: 1px solid rgba(255, 255, 255, 0.08);
  }

  .track-zone,
  .control-zone,
  .right-zone {
    min-width: 0;
    display: flex;
    align-items: center;
  }

  .track-zone {
    gap: 14px;
  }

  .control-zone {
    justify-content: center;
    flex-direction: column;
    gap: 8px;
  }

  .right-zone {
    justify-content: flex-end;
    gap: 12px;
  }

  .cover-card {
    width: 70px;
    height: 70px;
    flex: 0 0 70px;
    display: grid;
    place-items: center;
    overflow: hidden;
    border-radius: 18px;
    color: rgba(255, 255, 255, 0.58);
    background:
      linear-gradient(135deg, rgba(255,255,255,0.14), rgba(255,255,255,0.05)),
      rgba(255, 255, 255, 0.04);
    border: 1px solid rgba(255, 255, 255, 0.14);
    box-shadow: 0 12px 28px rgba(0,0,0,0.28);
  }

  .cover-card img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }

  .cover-card.switch-next {
    animation: immersive-cover-next 220ms cubic-bezier(0.2, 0.72, 0.18, 1) both;
  }

  .cover-card.switch-prev {
    animation: immersive-cover-prev 220ms cubic-bezier(0.2, 0.72, 0.18, 1) both;
  }

  @keyframes immersive-cover-next {
    0% { opacity: 0.55; transform: translateX(14px) scale(0.95); }
    100% { opacity: 1; transform: translateX(0) scale(1); }
  }

  @keyframes immersive-cover-prev {
    0% { opacity: 0.55; transform: translateX(-14px) scale(0.95); }
    100% { opacity: 1; transform: translateX(0) scale(1); }
  }

  @keyframes immersive-meta-next {
    0% { opacity: 0.45; transform: translateX(14px); }
    100% { opacity: 1; transform: translateX(0); }
  }

  @keyframes immersive-meta-prev {
    0% { opacity: 0.45; transform: translateX(-14px); }
    100% { opacity: 1; transform: translateX(0); }
  }

  .cover-card svg {
    width: 32px;
    height: 32px;
  }

  .cover-card.playing {
    border-color: rgba(var(--immersive-player-accent-rgb), 0.32);
  }

  .track-meta {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .track-meta strong,
  .track-meta span {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .track-meta strong {
    font-size: 17px;
    line-height: 1.2;
    font-weight: 700;
    letter-spacing: 0;
  }

  .track-meta span {
    max-width: 34vw;
    color: rgba(245, 247, 251, 0.58);
    font-size: 12px;
  }

  .queue-button,
  .icon-button,
  .transport-button,
  .play-button,
  .volume-button {
    position: relative;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex: 0 0 auto;
    color: inherit;
    -webkit-app-region: no-drag;
  }

  .queue-button,
  .icon-button {
    width: 42px;
    height: 42px;
    border-radius: 14px;
    color: rgba(245, 247, 251, 0.72);
    background: rgba(255, 255, 255, 0.055);
    border: 1px solid rgba(255, 255, 255, 0.09);
    box-shadow: inset 0 1px 0 rgba(255,255,255,0.08);
    transition: transform 160ms ease, color 160ms ease, background-color 160ms ease, border-color 160ms ease;
  }

  .queue-button:hover,
  .icon-button:hover,
  .queue-button.active,
  .icon-button.active {
    color: #ffffff;
    background: rgba(var(--immersive-player-accent-rgb), 0.14);
    border-color: rgba(var(--immersive-player-accent-rgb), 0.32);
  }

  .queue-button:active,
  .icon-button:active,
  .transport-button:active,
  .play-button:active {
    transform: translateY(1px) scale(0.98);
  }

  .queue-button svg,
  .icon-button svg {
    width: 20px;
    height: 20px;
  }

  .queue-button *,
  .icon-button *,
  .transport-button *,
  .play-button *,
  .volume-button * {
    pointer-events: none;
  }

  .transport {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 14px;
  }

  .transport-button {
    width: 42px;
    height: 42px;
    border-radius: 50%;
    color: rgba(245, 247, 251, 0.66);
    background: transparent;
    transition: color 160ms ease, background-color 160ms ease, transform 160ms ease;
  }

  .transport-button:hover {
    color: #ffffff;
    background: rgba(255, 255, 255, 0.08);
  }

  .transport-button svg {
    width: 22px;
    height: 22px;
  }

  .play-button {
    width: 60px;
    height: 60px;
    border-radius: 50%;
    color: #071015;
    background:
      radial-gradient(circle at 32% 24%, rgba(255,255,255,0.92), rgba(255,255,255,0.28) 38%, transparent 39%),
      linear-gradient(135deg, #bdfef3, var(--immersive-player-accent) 48%, #68d8ff);
    box-shadow:
      var(--immersive-player-button-shadow),
      0 0 24px rgba(var(--immersive-player-accent-rgb), 0.18);
    transition: transform 180ms ease, box-shadow 180ms ease, filter 180ms ease;
  }

  .play-button:hover {
    filter: brightness(1.04);
    box-shadow: var(--immersive-player-button-hover-shadow);
  }

  .play-button:disabled {
    cursor: default;
    opacity: 0.45;
    filter: grayscale(0.45);
  }

  .play-button svg {
    width: 27px;
    height: 27px;
    display: block;
  }

  .spinner {
    width: 22px;
    height: 22px;
    border-radius: 50%;
    border: 2px solid rgba(7, 16, 21, 0.24);
    border-top-color: #071015;
    animation: spin 800ms linear infinite;
  }

  .timeline-switch {
    position: relative;
    width: min(430px, 32vw);
    height: 28px;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .time-readout,
  .hover-controls {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: opacity 180ms ease, transform 180ms ease;
  }

  .time-readout {
    gap: 8px;
    color: rgba(245, 247, 251, 0.58);
    font-size: 12px;
    font-weight: 600;
  }

  .hover-controls {
    gap: 12px;
    opacity: 0;
    pointer-events: none;
    transform: translateY(3px);
  }

  .timeline-switch:hover .time-readout,
  .timeline-switch:focus-within .time-readout {
    opacity: 0;
    transform: translateY(-3px);
  }

  .timeline-switch:hover .hover-controls,
  .timeline-switch:focus-within .hover-controls {
    opacity: 1;
    pointer-events: auto;
    transform: translateY(0);
  }

  .scrubber {
    padding: 9px 0;
    cursor: pointer;
    -webkit-app-region: no-drag;
  }

  .scrubber.progress {
    flex: 1;
    min-width: 150px;
  }

  .scrubber.volume {
    width: 82px;
  }

  .rail {
    position: relative;
    height: 4px;
    overflow: visible;
    border-radius: 99px;
    background: rgba(255, 255, 255, 0.16);
  }

  .fill {
    position: relative;
    height: 100%;
    border-radius: inherit;
    background: linear-gradient(90deg, #ffffff, var(--immersive-player-accent));
  }

  .fill span {
    position: absolute;
    top: 50%;
    right: -5px;
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: #ffffff;
    box-shadow: 0 0 0 4px rgba(var(--immersive-player-accent-rgb), 0.18);
    transform: translateY(-50%) scale(0.72);
    opacity: 0.72;
    transition: transform 160ms ease, opacity 160ms ease;
  }

  .scrubber:hover .fill span,
  .scrubber:focus .fill span {
    transform: translateY(-50%) scale(1);
    opacity: 1;
  }

  .volume-control {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: 0 0 auto;
  }

  .volume-button {
    width: 24px;
    height: 24px;
    color: rgba(245, 247, 251, 0.62);
  }

  .volume-button:hover {
    color: #ffffff;
  }

  .volume-button svg {
    width: 18px;
    height: 18px;
  }

  .expand-button.expanded svg {
    transform: rotate(180deg);
  }

  .queue-scrim {
    position: fixed;
    inset: 0;
    z-index: 124;
    background: transparent;
    cursor: default;
  }

  .queue-panel {
    position: absolute;
    left: 18px;
    bottom: 128px;
    z-index: 126;
    width: min(560px, calc(100vw - 36px));
    height: min(520px, calc(100vh - 170px));
    display: flex;
    flex-direction: column;
    overflow: hidden;
    border-radius: 22px;
    border: 1px solid rgba(255, 255, 255, 0.14);
    background:
      radial-gradient(circle at 24% 0%, rgba(var(--immersive-player-accent-rgb), 0.12), transparent 38%),
      rgba(11, 13, 18, 0.9);
    box-shadow: 0 24px 80px rgba(0,0,0,0.44), var(--immersive-player-panel-shadow);
    -webkit-backdrop-filter: saturate(150%) blur(22px);
    backdrop-filter: saturate(150%) blur(22px);
  }

  .queue-panel header {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 20px 20px 14px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
  }

  .queue-panel header div {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .queue-panel header strong {
    font-size: 20px;
    line-height: 1.2;
  }

  .queue-panel header span,
  .text-action {
    color: rgba(245, 247, 251, 0.58);
    font-size: 12px;
  }

  .text-action:hover {
    color: var(--immersive-player-accent);
  }

  .icon-button.small {
    width: 34px;
    height: 34px;
    border-radius: 12px;
  }

  .icon-button.ghost {
    opacity: 0;
    background: transparent;
    border-color: transparent;
  }

  .queue-list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 10px;
  }

  .queue-list li {
    position: relative;
    min-height: 58px;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px 10px 9px 12px;
    overflow: hidden;
    border-radius: 14px;
    cursor: pointer;
    transition: background-color 160ms ease, color 160ms ease;
  }

  .queue-list li:hover,
  .queue-list li.playing {
    background: rgba(255, 255, 255, 0.075);
  }

  .queue-list li:hover .icon-button.ghost {
    opacity: 1;
  }

  .queue-index {
    width: 28px;
    flex: 0 0 28px;
    color: rgba(245, 247, 251, 0.36);
    font-size: 12px;
    font-weight: 700;
    text-align: center;
  }

  .queue-meta {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }

  .queue-title,
  .queue-artist {
    display: block;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .queue-title {
    color: rgba(245, 247, 251, 0.92);
    font-size: 14px;
    font-weight: 650;
  }

  .queue-title.disabled {
    color: rgba(245, 247, 251, 0.36);
  }

  .queue-artist {
    color: rgba(245, 247, 251, 0.46);
    font-size: 12px;
  }

  .queue-list li.playing .queue-title,
  .queue-list li.playing .queue-index {
    color: var(--immersive-player-accent);
  }

  .queue-spectrum {
    position: absolute;
    left: 52px;
    right: 46px;
    bottom: 0;
    height: 9px;
    display: flex;
    align-items: flex-end;
    gap: 3px;
    opacity: 0.74;
    pointer-events: none;
  }

  .queue-spectrum span {
    width: 4px;
    height: 35%;
    border-radius: 99px 99px 0 0;
    background: linear-gradient(180deg, rgba(255,255,255,0.92), var(--immersive-player-accent));
  }

  .queue-spectrum.active span {
    animation: queueSpectrum 980ms ease-in-out infinite;
    animation-delay: calc(var(--bar-index) * 90ms);
  }

  .queue-empty {
    justify-content: center;
    color: rgba(245, 247, 251, 0.44);
    cursor: default;
  }

  @keyframes queueSpectrum {
    0%, 100% { height: 25%; }
    50% { height: 100%; }
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  @media (max-width: 980px) {
    .immersive-player {
      left: 10px;
      right: 10px;
      bottom: 10px;
      height: 126px;
    }

    .player-bar {
      height: 126px;
      grid-template-columns: minmax(0, 1fr) auto;
      grid-template-rows: auto auto;
      gap: 8px 12px;
      padding: 14px;
    }

    .track-zone {
      grid-column: 1 / 2;
    }

    .control-zone {
      grid-column: 1 / -1;
      grid-row: 2;
    }

    .right-zone {
      grid-column: 2;
      grid-row: 1;
      gap: 8px;
    }

    .timeline-switch {
      width: min(520px, calc(100vw - 80px));
    }

    .cover-card {
      width: 58px;
      height: 58px;
      flex-basis: 58px;
      border-radius: 16px;
    }

    .track-meta span {
      max-width: 52vw;
    }

    .icon-button {
      width: 38px;
      height: 38px;
    }
  }

  @media (max-width: 680px) {
    .right-zone .icon-button:not(.expand-button) {
      display: none;
    }

    .queue-button {
      width: 38px;
      height: 38px;
      border-radius: 12px;
    }

    .transport {
      gap: 8px;
    }

    .transport-button {
      width: 36px;
      height: 36px;
    }

    .play-button {
      width: 54px;
      height: 54px;
    }

    .hover-controls {
      gap: 8px;
    }

    .scrubber.progress {
      min-width: 120px;
    }

    .scrubber.volume {
      width: 64px;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .immersive-player,
    .native-shell,
    .player-bar,
    .time-readout,
    .hover-controls,
    .queue-button,
    .icon-button,
    .transport-button,
    .play-button {
      transition-duration: 1ms !important;
    }

    .spinner,
    .queue-spectrum.active span {
      animation: none !important;
    }
  }
</style>
