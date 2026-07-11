<script lang="ts">
  import { playerState } from "../../lib/stores/player";
  import { MediaService } from "../../lib/providers/index";
  import { settings } from "../../lib/stores/settings";
  import { cssImageUrl, proxyResourceUrl } from "../../lib/resourceUrl";
  import { getActiveLyricPayload, getLineVariant, getLyricsVariantAvailability, getNextLyricVariantMode, isLyricVariantModeActive, lyricVariantButtonLabel as getLyricVariantButtonLabel, lyricVariantButtonTitle as getLyricVariantButtonTitle, normalizeLyricVariantMode, parseLyric, type LyricLine, type LyricVariantMode } from "../../lib/lyrics";
  import { runOnActionKey } from "../../lib/keyboard";
  import { tick } from "svelte";

  let {
    visible = false,
    onClose = () => {},
  }: {
    visible?: boolean;
    onClose?: () => void;
  } = $props();

  let lyricLines = $state<LyricLine[]>([]);
  let currentIdx = $state(-1);
  let lastTrackId = $state("");
  let lastLyricSignature = $state("");
  let lyricEl = $state<HTMLElement | null>(null);
  let showQueue = $state(false);
  let translationIndex = $state(0);
  let lyricUserActiveUntil = 0;
  let lyricReturnTimer: ReturnType<typeof window.setTimeout> | null = null;
  let lyricAvailability = $derived.by(() => {
    return getLyricsVariantAvailability(lyricLines);
  });
  let variantMode = $derived<LyricVariantMode>(
    normalizeLyricVariantMode($settings.lyricWindow.variantMode, lyricAvailability)
  );

  function highlightedLyricElement() {
    return lyricEl?.querySelector(".lyric-line.highlight") as HTMLElement | null;
  }

  function centerCurrentLyric(behavior: ScrollBehavior = "smooth") {
    const el = highlightedLyricElement();
    if (!el || !lyricEl) return;
    lyricEl.scrollTo({
      top: el.offsetTop - lyricEl.clientHeight / 2 + el.clientHeight / 2,
      behavior,
    });
  }

  function highlightedLyricInView() {
    const el = highlightedLyricElement();
    if (!el || !lyricEl) return true;
    const top = el.offsetTop - lyricEl.scrollTop;
    const bottom = top + el.offsetHeight;
    const margin = Math.max(32, lyricEl.clientHeight * 0.18);
    return top >= margin && bottom <= lyricEl.clientHeight - margin;
  }

  function scheduleLyricReturn(delay = 1600, reset = false) {
    if (lyricReturnTimer) {
      if (!reset) return;
      window.clearTimeout(lyricReturnTimer);
    }
    lyricReturnTimer = window.setTimeout(() => {
      lyricReturnTimer = null;
      if (Date.now() < lyricUserActiveUntil || !visible) return;
      if (!highlightedLyricInView()) centerCurrentLyric("smooth");
    }, delay);
  }

  function markLyricUserInteraction() {
    lyricUserActiveUntil = Date.now() + 1600;
    scheduleLyricReturn(1650, true);
  }

  $effect(() => {
    const track = $playerState.currentTrack;
    if (!track) {
      lastTrackId = "";
      lastLyricSignature = "";
      lyricLines = [];
      currentIdx = -1;
      return;
    }
    const inlineLyric = track.lyric ?? "";
    const lyricSignature = `${track.id}|${inlineLyric}|${track.lyric_url ?? ""}|${track.tlyric_url ?? ""}`;
    const isNewTrack = track.id !== lastTrackId;
    if (!isNewTrack && lyricSignature === lastLyricSignature) return;

    lastTrackId = track.id;
    lastLyricSignature = lyricSignature;
    if (isNewTrack) {
      lyricLines = [];
      currentIdx = -1;
      translationIndex = 0;
    }
    if (inlineLyric.trim()) {
      lyricLines = parseLyric(inlineLyric);
    }
    MediaService.getLyric(track.id, track.album_id ?? "", track.lyric_url, track.tlyric_url)
      .then((r) => {
        if (track.id !== $playerState.currentTrack?.id) return;
        const lyric = r.lyric || inlineLyric;
        lyricLines = lyric ? parseLyric(lyric, r.tlyric) : [];
      });
  });

  $effect(() => {
    const offsetSec = ($settings.lyricWindow.offsetMs ?? 0) / 1000;
    const payload = getActiveLyricPayload(lyricLines, $playerState.position + offsetSec, variantMode, translationIndex);
    if (!payload) return;
    const idx = payload.index;
    if (idx !== currentIdx) {
      currentIdx = idx;
      tick().then(() => {
        if (Date.now() >= lyricUserActiveUntil) centerCurrentLyric("smooth");
        else scheduleLyricReturn(1600, true);
      });
    }
  });

  $effect(() => {
    visible;
    currentIdx;
    $playerState.position;
    if (!visible || currentIdx < 0 || Date.now() < lyricUserActiveUntil) return;
    if (!highlightedLyricInView()) scheduleLyricReturn(500);
  });

  let rawBgUrl = $derived($playerState.currentTrack?.img_url ?? "");
  let bgUrl = $derived(proxyResourceUrl(rawBgUrl));

  function nextVariantMode(): LyricVariantMode {
    return getNextLyricVariantMode(variantMode, lyricAvailability);
  }

  function variantButtonLabel() {
    return getLyricVariantButtonLabel(variantMode, lyricAvailability);
  }

  function variantButtonTitle() {
    return getLyricVariantButtonTitle(variantMode, lyricAvailability);
  }

  function toggleVariantMode() {
    if (!lyricAvailability.hasTranslation && !lyricAvailability.hasPhonetic) return;
    const nextMode = nextVariantMode();
    translationIndex = 0;
    settings.patch({
      lyricWindow: { ...$settings.lyricWindow, variantMode: nextMode },
    });
  }

  let offsetLabel = $derived.by(() => {
    const ms = $settings.lyricWindow.offsetMs ?? 0;
    if (ms > 0) return `+${ms}ms`;
    if (ms < 0) return `${ms}ms`;
    return "0ms";
  });

  function adjustOffset(deltaMs: number) {
    const next = Math.max(-2000, Math.min(2000, ($settings.lyricWindow.offsetMs ?? 0) + deltaMs));
    settings.patch({ lyricWindow: { ...$settings.lyricWindow, offsetMs: next } });
  }
</script>

<div
  class="songdetail-wrapper"
  class:slidedown={!visible}
  class:coverbg={$settings.enableNowplayingCoverBackground}
>
  <div class="draggable-zone"></div>

  {#if bgUrl && $settings.enableNowplayingCoverBackground}
    <div class="bgwrapper">
      <div class="bg" style:background-image={cssImageUrl(rawBgUrl)}></div>
    </div>
  {/if}

  <div class="close" onclick={onClose} role="button" tabindex="0" onkeydown={(e) => runOnActionKey(e, onClose)}>
    <svg width="19" height="19" viewBox="0 0 32 32" fill="currentColor" stroke="none">
      <path d="M14.77,23.795L5.185,14.21c-0.879-0.879-0.879-2.317,0-3.195l0.8-0.801c0.877-0.878,2.316-0.878,3.194,0l7.315,7.315l7.316-7.315c0.878-0.878,2.317-0.878,3.194,0l0.8,0.801c0.879,0.878,0.879,2.316,0,3.195l-9.587,9.585c-0.471,0.472-1.104,0.682-1.723,0.647C15.875,24.477,15.243,24.267,14.77,23.795z"/>
    </svg>
  </div>

  <!-- Main content -->
  <div class="playsong-detail">
    <!-- Left: cover -->
    <div class="detail-head">
      <div>
        <div class="detail-head-cover">
          {#if bgUrl}
            <div class="covershadow" style:background-image={cssImageUrl(rawBgUrl)}></div>
            <img src={bgUrl} alt="cover" />
          {:else}
            <div class="empty-cover">
              <svg width="60" height="60" viewBox="0 0 24 24" stroke="rgba(255,255,255,0.3)">
                <circle cx="12" cy="12" r="10"/><circle cx="12" cy="12" r="3"/>
              </svg>
            </div>
          {/if}
        </div>

        {#if $playerState.currentTrack}
          <div class="detail-head-title">
            <div class="title">
              <h2>{$playerState.currentTrack.title}</h2>
              {#if $settings.enableNowplayingBitrate && $playerState.currentTrack.bitrate}
                <span class="badge">{$playerState.currentTrack.bitrate}</span>
              {/if}
              {#if $settings.enableNowplayingPlatform && $playerState.currentTrack.platform}
                <span class="badge platform">{$playerState.currentTrack.platform}</span>
              {/if}
            </div>
            <div class="info">
              <span class="singer">{$playerState.currentTrack.artist}</span>
              {#if $playerState.currentTrack.album}
                <span>-</span>
                <span class="album">{$playerState.currentTrack.album}</span>
              {/if}
            </div>
          </div>
        {/if}
      </div>
    </div>

    <!-- Right: info + lyrics -->
    <div class="detail-songinfo">
      <!-- Lyrics -->
      <div
        class="lyric"
        bind:this={lyricEl}
        onwheel={markLyricUserInteraction}
        onpointerdown={markLyricUserInteraction}
        ontouchstart={markLyricUserInteraction}
        role="region"
        aria-label="歌词"
      >
        <div class="variant-toggles" aria-label="歌词译文和音标">
          <div class="offset-adjust" title="歌词进度微调（提前/延后）">
            <button type="button" class="offset-btn" aria-label="歌词延后 100ms" onclick={() => adjustOffset(-100)}>−</button>
            <span class="offset-value">{offsetLabel}</span>
            <button type="button" class="offset-btn" aria-label="歌词提前 100ms" onclick={() => adjustOffset(100)}>+</button>
          </div>
          <button
            type="button"
            class="translate-toggle"
            class:active={isLyricVariantModeActive(variantMode, lyricAvailability)}
            disabled={!lyricAvailability.hasTranslation && !lyricAvailability.hasPhonetic}
            onclick={toggleVariantMode}
            title={variantButtonTitle()}
          >{variantButtonLabel()}</button>
        </div>
        <div class="placeholder"></div>
        {#each lyricLines as line, i (i)}
          <p class="lyric-line" class:highlight={i === currentIdx}>
            {line.content}
          </p>
          {@const variant = getLineVariant(line, translationIndex, variantMode)}
          {#if variant}
            <p class="lyric-line translate" class:highlight={i === currentIdx}>
              {variant.text}
            </p>
          {/if}
        {/each}
        {#if !lyricLines.length}
          <p style="opacity:0.3;padding:18px">暂无歌词</p>
        {/if}
      </div>
    </div>
  </div>
</div>

<style>
  .songdetail-wrapper {
    --nowplaying-control-height: 100px;
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    bottom: var(--nowplaying-control-height);
    overflow: hidden;
    -webkit-app-region: no-drag;
    transition: all 0.5s;
    z-index: 100;
    opacity: 1;
  }

  .songdetail-wrapper.slidedown {
    top: calc(100% - var(--nowplaying-control-height));
    pointer-events: none;
  }

  .draggable-zone {
    position: absolute;
    left: 0;
    top: 0;
    right: 0;
    height: 80px;
    -webkit-app-region: drag;
  }

  .songdetail-wrapper.slidedown .draggable-zone {
    display: none;
    -webkit-app-region: no-drag;
  }

  .bgwrapper {
    overflow: hidden;
    border-radius: 10px;
    width: 100%;
    position: absolute;
    inset: 0;
  }

  .bg {
    opacity: 0.6;
    width: 100%; height: 100%;
    filter: blur(72px) contrast(82%) brightness(132%);
    background-repeat: no-repeat;
    background-position: center;
    background-size: cover;
    transition: background ease-in-out 1.5s;
    position: absolute;
  }

  .close {
    position: absolute;
    top: 24px; left: 24px;
    height: 19px; width: 19px;
    cursor: pointer;
    padding: 5px;
    box-sizing: content-box;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: 50%;
    -webkit-app-region: no-drag;
    transition: 0.2s;
    z-index: 100;
  }

  .close:hover { background-color: var(--songlist-hover-background-color); }

  .close svg {
    color: var(--now-playing-close-icon-color);
  }

  /* Content layout */
  .playsong-detail {
    position: absolute;
    top: 0; right: 0; left: 0; bottom: 0;
    display: flex;
    clip: rect(auto, auto, auto, auto);
  }

  .detail-head {
    flex: 1;
    display: flex;
    justify-content: flex-end;
    margin-right: 32px;
    margin-top: 24px;
    align-items: center;
    transition: all 0.5s;
    z-index: 1;
  }

  .detail-head-cover { position: relative; }

  .covershadow {
    transition: opacity 0.2s, transform 0.2s;
    opacity: 1;
    position: absolute;
    top: 12px;
    height: 100%;
    width: 100%;
    filter: blur(16px) opacity(0.6);
    transform: scale(0.92, 0.96);
    z-index: -1;
    background-size: cover;
    border-radius: 0.75em;
  }

  .detail-head img {
    border-radius: 10px;
    width: 54vh; height: 54vh;
    user-select: none;
    object-fit: cover;
  }

  .empty-cover {
    width: 54vh;
    height: 54vh;
    background: rgba(255,255,255,0.1);
    border-radius: 10px;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .detail-songinfo {
    flex: 1;
    font-weight: 600;
    margin-right: 24px;
    z-index: 0;
    display: flex;
    flex-direction: column;
  }

  .detail-head-title {
    max-width: 54vh;
    margin-top: 24px;
  }

  .title {
    display: flex;
    align-items: center;
  }

  .title h2 {
    font-size: var(--h2-title-font-size);
    margin-top: 8px;
    margin-bottom: 0;
    font-weight: 600;
    opacity: 0.88;
    overflow: hidden;
    text-overflow: ellipsis;
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 1;
    line-clamp: 1;
  }

  .badge {
    font-size: 12px;
    color: var(--theme-color);
    border: solid 1px var(--theme-color);
    border-radius: 5px;
    margin-left: 10px;
    padding: 0 4px;
    height: 20px;
    display: flex;
    align-items: center;
    justify-content: center;
    white-space: nowrap;
    font-weight: 400;
    margin-top: 4px;
  }

  .badge.platform { padding-top: 1px; }
  .badge:first-of-type { margin-left: 15px; }

  .info {
    margin-top: 4px;
    font-size: 16px;
    opacity: 0.58;
    overflow: hidden;
    text-overflow: ellipsis;
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 1;
    line-clamp: 1;
  }

  .singer, .album { display: inline; }

  /* Lyrics (matches original .lyric) */
  .lyric {
    font-size: 16px;
    flex: 1;
    display: flex;
    flex-direction: column;
    padding-left: 78px;
    max-width: 460px;
    overflow-y: auto;
    transition: 0.5s;
    color: var(--lyric-default-color);
    -webkit-app-region: no-drag;
    position: relative;
  }

  .variant-toggles {
    position: sticky;
    top: 18px;
    align-self: flex-end;
    z-index: 4;
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .offset-adjust {
    display: flex;
    align-items: center;
    gap: 4px;
    height: 28px;
    padding: 0 4px;
    border-radius: 8px;
    background: var(--button-background-color);
    border: 1px solid var(--line-default-color);
  }

  .offset-btn {
    width: 22px;
    height: 22px;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: var(--text-default-color);
    font-size: 15px;
    font-weight: 800;
    line-height: 1;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .offset-btn:hover {
    background: var(--songlist-hover-background-color);
    color: var(--theme-color);
  }

  .offset-value {
    min-width: 46px;
    text-align: center;
    font-size: 11px;
    font-weight: 700;
    color: var(--text-subtitle-color);
    font-variant-numeric: tabular-nums;
  }

  .translate-toggle {
    position: static;
    width: 34px;
    height: 28px;
    border-radius: 8px;
    color: var(--text-subtitle-color);
    background: var(--button-background-color);
    border: 1px solid var(--line-default-color);
    font-size: 13px;
    font-weight: 800;
    cursor: pointer;
  }

  .translate-toggle:hover,
  .translate-toggle.active {
    color: #fff;
    background: var(--theme-color);
    border-color: var(--theme-color);
  }

  .translate-toggle:disabled {
    opacity: 0.45;
    cursor: default;
  }

  .translate-toggle:disabled:hover {
    color: var(--text-subtitle-color);
    background: var(--button-background-color);
    border-color: var(--line-default-color);
  }

  .lyric::-webkit-scrollbar { display: none; }

  .lyric .placeholder { margin-top: 50vh; }

  .lyric p {
    padding: 12px 18px;
    transition: opacity 0.18s, color 0.18s, background-color 0.18s, font-size 0.18s;
    border-radius: 12px;
    margin: 0;
    opacity: 0.28;
    cursor: default;
    font-size: 16px;
    background: transparent;
    color: var(--text-default-color);
  }

  .lyric p:hover {
    background: hsla(0, 0%, 100%, 0.08);
    opacity: 0.6;
    color: var(--text-default-color);
  }

  .lyric p.translate {
    margin: -8px 0 8px;
    padding-top: 0;
    font-size: 14px;
  }

  .lyric p.highlight {
    color: var(--text-default-color);
    opacity: 1;
    font-size: 26px;
  }

  .lyric p.translate.highlight {
    font-size: 15px;
    opacity: 0.72;
  }

  .coverbg .info span,
  .coverbg .lyric {
    color: var(--lyric-on-cover-color);
  }

  @media (max-height: 720px) and (min-width: 721px) {
    .songdetail-wrapper {
      bottom: var(--nowplaying-control-height);
    }

    .playsong-detail {
      bottom: clamp(24px, 5vh, 42px);
    }

    .detail-head {
      align-items: flex-start;
      margin-top: clamp(30px, 6vh, 46px);
    }

    .detail-head img,
    .empty-cover {
      width: clamp(220px, 42vh, 340px);
      height: clamp(220px, 42vh, 340px);
    }

    .detail-head-title {
      max-width: clamp(220px, 42vh, 340px);
      margin-top: clamp(10px, 2vh, 18px);
    }

    .title h2 {
      font-size: clamp(19px, 3.2vh, var(--h2-title-font-size));
      -webkit-line-clamp: 2;
      line-clamp: 2;
    }

    .info {
      font-size: clamp(13px, 2.2vh, 16px);
      -webkit-line-clamp: 2;
      line-clamp: 2;
    }

    .lyric .placeholder {
      margin-top: clamp(180px, 36vh, 50vh);
    }

    .lyric p {
      padding: clamp(12px, 2.2vh, 18px) 18px;
    }

    .lyric p.highlight {
      font-size: clamp(21px, 3.8vh, 26px);
    }
  }

  @media (max-width: 720px) {
    .songdetail-wrapper {
      bottom: var(--nowplaying-control-height);
      overflow-y: auto;
    }

    .songdetail-wrapper.slidedown {
      top: calc(100% - var(--nowplaying-control-height));
    }

    .draggable-zone {
      height: 58px;
    }

    .bgwrapper {
      border-radius: 0;
    }

    .close {
      top: calc(var(--safe-top) + 18px);
      left: 18px;
      width: 34px;
      height: 34px;
      display: flex;
      align-items: center;
      justify-content: center;
      padding: 0;
      background: rgba(0, 0, 0, 0.18);
    }

    .playsong-detail {
      position: relative;
      min-height: 100%;
      display: flex;
      flex-direction: column;
      overflow-y: auto;
      padding: calc(var(--safe-top) + 64px) 18px 24px;
      gap: 18px;
    }

    .detail-head {
      flex: 0 0 auto;
      justify-content: center;
      align-items: center;
      margin: 0;
      width: 100%;
    }

    .detail-head > div {
      width: 100%;
      display: flex;
      flex-direction: column;
      align-items: center;
    }

    .detail-head img,
    .empty-cover {
      width: min(66vw, 280px);
      height: min(66vw, 280px);
    }

    .detail-head-title {
      width: 100%;
      max-width: calc(100vw - 36px);
      margin-top: 18px;
      text-align: center;
    }

    .title {
      justify-content: center;
      flex-wrap: wrap;
      gap: 6px;
    }

    .title h2 {
      width: 100%;
      margin-top: 0;
      -webkit-line-clamp: 2;
      line-clamp: 2;
    }

    .badge,
    .badge:first-of-type {
      margin: 2px 0 0;
    }

    .info {
      font-size: 14px;
      -webkit-line-clamp: 2;
      line-clamp: 2;
    }

    .detail-songinfo {
      flex: 1 1 auto;
      min-height: 280px;
      width: 100%;
      margin-right: 0;
    }

    .lyric {
      width: 100%;
      max-width: none;
      min-height: 260px;
      padding-left: 0;
      text-align: center;
    }

    .lyric .placeholder {
      margin-top: 120px;
    }

    .lyric p {
      padding: 12px 8px;
      font-size: 15px;
    }

    .lyric p.highlight {
      font-size: 21px;
    }
  }
</style>
