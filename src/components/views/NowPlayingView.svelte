<script lang="ts">
  import { playerState, playbackClock } from "../../lib/stores/player";
  import { MediaService } from "../../lib/providers/index";
  import { settings } from "../../lib/stores/settings";
  import { cssUrl, sizedImageUrl } from "../../lib/resourceUrl";
  import { deviceTier } from "../../lib/stores/device";
  import { getActiveLyricPayload, getLineVariant, getLyricsVariantAvailability, getNextLyricVariantMode, isLyricVariantModeActive, lyricVariantButtonLabel as getLyricVariantButtonLabel, lyricVariantButtonTitle as getLyricVariantButtonTitle, normalizeLyricVariantMode, parseLyric, type LyricLine, type LyricVariantMode } from "../../lib/lyrics";
  import { runOnActionKey } from "../../lib/keyboard";
  import { windowToggleFullscreen } from "../../lib/tauri";
  import { tick } from "svelte";

  let {
    visible = false,
    onClose = () => {},
    windowControlsRevealed = false,
    windowControlsEl = null,
  }: {
    visible?: boolean;
    onClose?: () => void;
    /** 展开态那三个窗口按钮是否已经翻下来。翻下来时右上角这排控件要让位。 */
    windowControlsRevealed?: boolean;
    /** 那三个按钮的 DOM，用来判断横向区间是否真的和这排控件相交。 */
    windowControlsEl?: HTMLElement | null;
  } = $props();

  let lyricLines = $state<LyricLine[]>([]);
  let currentIdx = $state(-1);
  let lastTrackId = $state("");
  let lastLyricSignature = $state("");
  let lyricEl = $state<HTMLElement | null>(null);
  let variantTogglesEl = $state<HTMLElement | null>(null);
  // 只有那三个按钮和这排控件真的相交时才让位。窗口够宽时歌词列的右边缘远在按钮
  // 左侧（X 不重叠），窄布局下歌词整块被挤到下半屏（Y 不重叠），这两种情况下移都是白费。
  let variantTogglesPushed = $state(false);
  /** 真全屏中。双击大封面切换，用来改提示文案。 */
  let isFullscreen = $state(false);

  /**
   * 双击大封面切真全屏。
   *
   * 和右上角那个「最大化」按钮不是一回事：最大化只铺满工作区，任务栏还在；这里走
   * Rust 的 `set_fullscreen`，独占整个屏幕。做成双击是因为封面本身没有单击行为，
   * 双击不会和任何既有手势打架。
   */
  async function toggleFullscreen() {
    try {
      isFullscreen = await windowToggleFullscreen();
    } catch {
      // 非 Tauri 环境（浏览器里开 dist）没有这个命令，静默忽略即可。
    }
  }

  /**
   * 全屏时 Esc 先退全屏，不要直接把播放页收起来。
   *
   * Esc 默认绑的是「收起播放页」（`settings.keyboardShortcuts.closeNowPlaying`），
   * 分发在 App 那边。全屏是从播放页里进去的，一路退回去的期望顺序是
   * 全屏 → 展开态 → 收起，一步跨两级会让人措手不及；而且真全屏下 Esc 退出是系统级习惯。
   * 用 capture 阶段抢在 App 的 window 监听之前处理，并 stopPropagation 掉。
   */
  $effect(() => {
    if (!isFullscreen) return;
    const onKeydown = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      e.stopPropagation();
      void toggleFullscreen();
    };
    window.addEventListener("keydown", onKeydown, true);
    return () => window.removeEventListener("keydown", onKeydown, true);
  });

  // 播放页收起时顺手退全屏。否则窗口会停在独占整屏的姿态上，而里面显示的是歌单页，
  // 用户既没有全屏的心理预期，也找不到退出的入口（双击封面那块已经不在屏上了）。
  $effect(() => {
    if (!visible && isFullscreen) void toggleFullscreen();
  });
  $effect(() => {
    if (!windowControlsRevealed || !variantTogglesEl || !windowControlsEl) {
      variantTogglesPushed = false;
      return;
    }
    const toggles = variantTogglesEl.getBoundingClientRect();
    // 用布局盒（offsetTop/offsetLeft + offsetWidth/offsetHeight）而不是
    // getBoundingClientRect：按钮此刻正带着 rotateX + perspective，量出来的可视矩形
    // 被投影压过（翻起时高度直接是 0），区间不可信。布局盒不受 transform 影响，
    // 父级 .wc-flyout 自身没有 transform，可以直接叠上它的位置。
    const parent = (windowControlsEl.offsetParent as HTMLElement | null)?.getBoundingClientRect();
    const left = (parent?.left ?? 0) + windowControlsEl.offsetLeft;
    const top = (parent?.top ?? 0) + windowControlsEl.offsetTop;
    const right = left + windowControlsEl.offsetWidth;
    const bottom = top + windowControlsEl.offsetHeight;
    variantTogglesPushed =
      toggles.left < right && left < toggles.right && toggles.top < bottom && top < toggles.bottom;
  });
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
    // 播放页隐藏时跳过逐 tick 的歌词计算与 DOM 查询。
    if (!visible) return;
    const offsetSec = ($settings.lyricWindow.offsetMs ?? 0) / 1000;
    const payload = getActiveLyricPayload(lyricLines, $playbackClock.position + offsetSec, variantMode, translationIndex);
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
    $playbackClock.position;
    if (!visible || currentIdx < 0 || Date.now() < lyricUserActiveUntil) return;
    if (!highlightedLyricInView()) scheduleLyricReturn(500);
  });

  let rawBgUrl = $derived($playerState.currentTrack?.img_url ?? "");
  let bgUrl = $derived(sizedImageUrl(rawBgUrl, 500));
  let bgBlurUrl = $derived(sizedImageUrl(rawBgUrl, 200));

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

  {#if bgUrl && $settings.enableNowplayingCoverBackground && $deviceTier !== "low"}
    <div class="bgwrapper">
      <div class="bg" style:background-image={cssUrl(bgBlurUrl)}></div>
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
        <div
          class="detail-head-cover"
          role="button"
          tabindex="0"
          title={isFullscreen ? "双击退出全屏" : "双击全屏"}
          ondblclick={toggleFullscreen}
          onkeydown={(e) => runOnActionKey(e, toggleFullscreen)}
        >
          {#if bgUrl}
            <div class="covershadow" style:background-image={cssUrl(bgBlurUrl)}></div>
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
        <div class="variant-toggles" class:pushed-down={variantTogglesPushed} bind:this={variantTogglesEl} aria-label="歌词译文和音标">
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
    left: 0;
    /* 盒子必须跟父级 `.footer-main` 的**尺寸动画**彻底解耦，两个方向都要。

       高度方向：只锚在父级底边、高度按视口算。原来是 top:0 + bottom:100px，父级
       100px 高的时候这块被压成 0 高，展开的每一帧都要把歌词列表重新布局一遍。

       宽度方向：用 `width: 100vw` 而不是 `right: 0`。`right: 0` 让宽度跟着父级走，
       而 `.footer` 展开时宽度要从 98vw 长到 100vw —— 于是整页歌词每帧重排，底下那层
       `.bg`（全屏 48~72px 模糊的封面）也每帧按新尺寸重新模糊一遍。换成视口宽之后，
       父级怎么长这块的盒子都不变，模糊纹理只光栅化一次。这一条是让 `.footer` 的
       宽度重新参与过渡的前提。
       收起态父级内缩 1vw，这块会朝右溢出 1vw：那时它是 visibility: hidden，
       而且 html/body 都是 overflow: hidden，不会有滚动条。 */
    width: 100vw;
    bottom: var(--nowplaying-control-height);
    height: calc(100vh - var(--nowplaying-control-height));
    overflow: hidden;
    -webkit-app-region: no-drag;
    /* 揭帘模型：内容从第 0 帧起就已按最终尺寸完整布局，展开时**只是逐渐不再被裁掉**。
       裁剪边与父级 `.footer-main` 的顶边同步推进，看起来是面板长开、内容跟着露出来，
       而不是"面板先长完 300ms、内容再淡进来"——后者是上一版的写法，读起来是两段式。

       两条边为什么能严格对齐、而不是靠调参凑：父级高度 h(t) 从 100px 长到 100vh，
       它的顶边在 viewport 里位于 H - h(t)；而这块盒子恒定占 y=0 到 y=(H-100px)。
       于是所需的 top inset = (H - h(t)) / (H - 100px)。把 h(t) = 100 + (H-100)·e(t)
       代进去得 inset = 1 - e(t)：只要两者用**同一个时长和同一条 easing**，
       `inset(100%…)` → `inset(0%…)` 的插值天然就是父级高度曲线的镜像，逐帧吻合。

       clip-path 只影响绘制，不触发 layout，所以子元素盒子全程不变，那层全屏模糊的封面
       （`.bg`）只光栅化一次。父级不能靠 overflow: hidden 裁（会切掉探出播放条的圆形
       封面），这块自己裁自己正好绕开那个限制——裁剪只作用于自身与后代，够不到兄弟节点
       `.footerwrap` 里的封面。

       写 `0%` 而不是 `0`：两端同为百分比才是同类插值。 */
    clip-path: inset(0% 0 0 0);
    transition:
      clip-path var(--player-expand-dur, 420ms) var(--player-expand-ease, ease),
      visibility 0s;
    visibility: visible;
    z-index: 100;
  }

  .songdetail-wrapper.slidedown {
    /* 收起：帘反向合上，与父级收回同步。刻意不再叠 opacity——"整体变淡"会盖住"被逐渐
       裁掉"，两种运动叠在一起反而读不清，而且淡出先行就是上一版那个两段式的镜像。
       帘完全合上之后才切 visibility: hidden（0s + 一整个时长的延迟）：盒子是整屏大小的，
       光靠裁剪未必能让合成器丢掉那层全屏模糊的封面纹理，visibility: hidden 才是明确的
       "整棵子树不参与绘制"。布局仍然保留，所以下次展开不用重新排版。 */
    clip-path: inset(100% 0 0 0);
    visibility: hidden;
    pointer-events: none;
    transition:
      clip-path var(--player-expand-dur, 420ms) var(--player-expand-ease, ease),
      visibility 0s linear var(--player-expand-dur, 420ms);
  }

  /* app.css 的全局 reduced-motion 只把 transition-duration 压到 0.01ms，delay 不动。
     收起那条 visibility 的延迟是一整个时长，不归零的话帘瞬间合上、子树却还要多绘制
     300ms。所以延迟一起压掉。 */
  @media (prefers-reduced-motion: reduce) {
    .songdetail-wrapper,
    .songdetail-wrapper.slidedown {
      transition-delay: 0s !important;
    }
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
    /* 视觉档位变量：high=72px 完整模糊；mid=48px 轻量；low=无（不渲染该层） */
    filter: var(--visual-cover-bg-filter);
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

  /* 双击这块切真全屏。cursor 不改成 pointer——它不是单击可用的按钮，给个默认光标
     配 title 提示更诚实。`.draggable-zone` 覆盖了顶部 80px，那一段收不到双击，
     所以这里显式声明 no-drag，否则拖窗区会把双击吃成"双击标题栏＝最大化"。 */
  .detail-head-cover {
    position: relative;
    -webkit-app-region: no-drag;
  }

  .covershadow {
    transition: opacity 0.2s, transform 0.2s;
    opacity: 1;
    position: absolute;
    top: 12px;
    height: 100%;
    width: 100%;
    filter: var(--visual-cover-shadow-filter);
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
    /* 原来是 `0.5s` 简写 = all：随便哪个属性变了都会被插值 0.5s，包括
       padding/max-width/font-size 这些一动就要重排整列歌词的。这块本身不需要过渡。 */
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
    /* 时长和曲线跟 .wc-flyout-inner 的翻转对齐，让让位和翻下来看着是同一个动作 */
    transition: transform 340ms cubic-bezier(0.22, 0.9, 0.24, 1);
  }

  /* 三个窗口按钮翻下来时会盖住这一排（band 54px 高，这排原本在 y≈19..47），
     整体下移让开；按钮翻回去就自动滑回原位。 */
  .variant-toggles.pushed-down {
    transform: translateY(34px);
  }

  @media (prefers-reduced-motion: reduce) {
    .variant-toggles {
      transition: none;
    }
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
    /* 刻意不过渡 font-size：高亮行是 16px→26px，插值这 180ms 等于每帧把整列歌词
       重新排版一次（后面所有行都要跟着挪），而这一列上面压着 .footerwrap 的
       backdrop-filter、下面垫着全屏模糊的封面，两层都得跟着重算。换行本来几秒一次，
       但每次都是一串掉帧。字号瞬时切换，淡入淡出交给 opacity/color。 */
    transition: opacity 0.18s, color 0.18s, background-color 0.18s;
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
