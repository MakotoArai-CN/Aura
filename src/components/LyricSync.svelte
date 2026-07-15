<script lang="ts">
  import { onMount } from "svelte";
  import { playerState } from "../lib/stores/player";
  import { settings, type AppSettings } from "../lib/stores/settings";
  import { MediaService } from "../lib/providers/index";
  import { getTauriRuntimeDiagnostics, hideFloatWindow, listen, setFloatingLyricPayload, setFloatingLyricSettings, showFloatWindow } from "../lib/tauri";
  import { getActiveTwoLineLyricPayload, getLineVariant, getLyricsVariantAvailability, getNextLyricVariantMode, normalizeLyricVariantMode, parseLyric, type LyricLine, type LyricVariantMode } from "../lib/lyrics";

  type FloatingLyricPayload = {
    lyric: string;
    tlyric: string;
    variantKind?: "translation" | "phonetic";
    nextLyric?: string;
    nextTlyric?: string;
    nextVariantKind?: "translation" | "phonetic";
    translationCount?: number;
    translationIndex?: number;
    variantMode?: LyricVariantMode;
    hasTranslation?: boolean;
    hasPhonetic?: boolean;
    seq?: number;
  };

  type FloatingLyricSettingsPayload = {
    lyricWindow: AppSettings["lyricWindow"];
    enableLyricFloatingWindow: boolean;
    hideLyricFloatingWindowWhenMainVisible: boolean;
  };

  let lyricLines = $state<LyricLine[]>([]);
  let lastTrackId = $state("");
  let lastLyricSignature = $state("");
  let lastPayloadKey = $state("");
  let floatingTranslationIndex = $state(0);
  let floatingVariantMode = $derived<LyricVariantMode>(
    normalizeLyricVariantMode($settings.lyricWindow.variantMode, getLyricsVariantAvailability(lyricLines))
  );
  const WAITING_LYRIC = "等待歌词";
  const LOADING_LYRIC = "加载歌词";
  const EMPTY_LYRIC = "暂无歌词";
  let lyricLoadingTrackId = $state("");
  let currentPayload = $state<FloatingLyricPayload>({ lyric: WAITING_LYRIC, tlyric: "" });
  let lastSettingsPayloadKey = $state("");
  let lastTranslationControlId = "";
  let lastCloseControlId = "";
  let payloadSeq = 0;
  let prevFloatEnabled: boolean | null = null;
  let prevHideWhenMainVisible: boolean | null = null;
  let prevHideWhenMainVisible: boolean | null = null;
  let prevHideWhenMainVisible: boolean | null = null;

  function fallbackPayload(): FloatingLyricPayload {
    const track = $playerState.currentTrack;
    if (!track) return { lyric: WAITING_LYRIC, tlyric: "" };
    return { lyric: lyricLoadingTrackId === track.id ? LOADING_LYRIC : EMPTY_LYRIC, tlyric: "" };
  }

  function payloadForCurrentPosition(): { key: string; payload: FloatingLyricPayload } {
    const position = Math.max(0, $playerState.position + (($settings.lyricWindow.offsetMs ?? 0) / 1000));
    const availability = lyricAvailability();
    const mode = floatingVariantMode;
    const active = getActiveTwoLineLyricPayload(
      lyricLines,
      position,
      mode,
      floatingTranslationIndex
    );
    if (active) {
      return {
        key: `${active.index}:${floatingVariantMode}:${floatingTranslationIndex}:${active.lyric}:${active.tlyric}:${active.nextLyric}:${active.nextTlyric}`,
        payload: {
          lyric: active.lyric,
          tlyric: active.tlyric,
          variantKind: active.variantKind,
          nextLyric: active.nextLyric,
          nextTlyric: active.nextTlyric,
          nextVariantKind: active.nextVariantKind,
          translationCount: active.translationCount,
          translationIndex: floatingTranslationIndex,
          variantMode: mode,
          hasTranslation: availability.hasTranslation,
          hasPhonetic: availability.hasPhonetic,
        },
      };
    }

    const firstLine = lyricLines.find((line) => !line.translationFlag && line.content.trim());
    if (firstLine) {
      const nextLine = lyricLines.find((line) =>
        !line.translationFlag &&
        line.seconds > firstLine.seconds &&
        line.content.trim()
      );
      const variant = getLineVariant(firstLine, floatingTranslationIndex, mode);
      const nextVariant = getLineVariant(nextLine, floatingTranslationIndex, mode);
      const payload = {
        lyric: firstLine.content,
        tlyric: variant?.text ?? "",
        variantKind: variant?.kind ?? (mode === "phonetic" ? "phonetic" as const : "translation" as const),
        nextLyric: nextLine?.content ?? "",
        nextTlyric: nextVariant?.text ?? "",
        nextVariantKind: nextVariant?.kind,
        translationCount: firstLine.variants?.length ?? firstLine.translations?.length ?? 0,
        translationIndex: floatingTranslationIndex,
        variantMode: mode,
        hasTranslation: availability.hasTranslation,
        hasPhonetic: availability.hasPhonetic,
      };
      return { key: `first:${floatingVariantMode}:${payload.lyric}:${payload.tlyric}`, payload };
    }

    const payload = fallbackPayload();
    return { key: `fallback:${floatingVariantMode}:${payload.lyric}:${payload.tlyric}`, payload: { ...payload, variantMode: mode, ...availability } };
  }

  function lyricAvailability() {
    return getLyricsVariantAvailability(lyricLines);
  }

  function preferredVariantMode(): LyricVariantMode {
    return $settings.lyricWindow.variantMode;
  }

  function setFloatingVariantMode(mode: LyricVariantMode) {
    settings.patch({
      lyricWindow: {
        ...$settings.lyricWindow,
        variantMode: mode,
      },
    });
  }

  function syncCurrentLyric(force = false) {
    const { key, payload } = payloadForCurrentPosition();
    if (!force && key === lastPayloadKey) return;
    lastPayloadKey = key;
    sendFloatingLyric(payload);
  }

  function sendFloatingLyric(payload: FloatingLyricPayload = currentPayload) {
    payload = { ...payload, seq: ++payloadSeq };
    currentPayload = payload;
    // 唯一歌词通道：写入 Rust 缓存 + Rust eval 注入浮窗（listen1-native-lyric-update）。
    void setFloatingLyricPayload(payload).catch((error) => {
      if ($settings.enableLyricFloatingWindow) {
        console.error("[LyricSync] set native lyric payload failed", {
          error,
          tauri: getTauriRuntimeDiagnostics(),
        });
      }
    });
  }

  function settleLyricLoading(trackId: string) {
    window.setTimeout(() => {
      if ($playerState.currentTrack?.id !== trackId || lyricLoadingTrackId !== trackId || lyricLines.length > 0) return;
      lyricLoadingTrackId = "";
      syncCurrentLyric(true);
    }, 6000);
  }

  function syncFloatWindowNow() {
    syncFloatingLyricSettings(true);
    syncCurrentLyric(true);
    window.setTimeout(() => syncCurrentLyric(true), 120);
    window.setTimeout(() => syncCurrentLyric(true), 420);
    window.setTimeout(() => syncCurrentLyric(true), 1000);
  }

  function nextVariantMode(): LyricVariantMode {
    return getNextLyricVariantMode(floatingVariantMode, lyricAvailability());
  }

  function handleTranslationNext(controlId?: string, mode?: LyricVariantMode) {
    if (controlId && controlId === lastTranslationControlId) return;
    if (controlId) lastTranslationControlId = controlId;
    const availability = lyricAvailability();
    const requestedMode = normalizeLyricVariantMode(mode ?? nextVariantMode(), availability);
    setFloatingVariantMode(requestedMode);
    if (requestedMode === "off") floatingTranslationIndex = 0;
    syncCurrentLyric(true);
  }

  function handleFloatLyricClosed(controlId?: string) {
    if (controlId && controlId === lastCloseControlId) return;
    if (controlId) lastCloseControlId = controlId;
    settings.patch({ enableLyricFloatingWindow: false });
    hideFloatWindow();
  }

  function floatingLyricSettingsPayload(): FloatingLyricSettingsPayload {
    return {
      lyricWindow: { ...$settings.lyricWindow },
      enableLyricFloatingWindow: $settings.enableLyricFloatingWindow,
      hideLyricFloatingWindowWhenMainVisible: $settings.hideLyricFloatingWindowWhenMainVisible,
    };
  }

  function syncFloatingLyricSettings(force = false) {
    const payload = floatingLyricSettingsPayload();
    const key = JSON.stringify(payload);
    if (!force && key === lastSettingsPayloadKey) return;
    lastSettingsPayloadKey = key;
    // 唯一设置通道：写入 Rust 快照（供可见性/鼠标穿透判定），Rust 再 emit_to("float",
    // "lyric-settings-update") 送达浮窗。设置为低频事件，单通道足够。
    void setFloatingLyricSettings(payload).catch((error) => {
      if ($settings.enableLyricFloatingWindow) {
        console.error("[LyricSync] set native lyric settings failed", {
          error,
          tauri: getTauriRuntimeDiagnostics(),
        });
      }
    });
  }

  onMount(() => {
    let unlistenReady: (() => void) | null = null;
    let unlistenVariant: (() => void) | null = null;
    let unlistenClosed: (() => void) | null = null;
    let unlistenLockChanged: (() => void) | null = null;
    let unlistenSettingsChanged: (() => void) | null = null;
    let disposed = false;

    // 浮窗就绪握手：全量重推歌词 + 设置。
    void listen("float-lyric-ready", () => syncFloatWindowNow())
      .then((cleanup) => {
        unlistenReady = cleanup;
        if (disposed) cleanup();
      })
      .catch((error) => console.error("[LyricSync] float lyric ready listener failed", {
        error,
        tauri: getTauriRuntimeDiagnostics(),
      }));

    // 浮窗回控（唯一通道，均为 emitTo("main")）：切换变体。
    void listen<{ id?: string; mode?: LyricVariantMode }>("float-lyric-variant-next", (event) => handleTranslationNext(event.payload?.id, event.payload?.mode))
      .then((cleanup) => {
        unlistenVariant = cleanup;
        if (disposed) cleanup();
      })
      .catch((error) => console.error("[LyricSync] float lyric variant listener failed", {
        error,
        tauri: getTauriRuntimeDiagnostics(),
      }));

    // 浮窗回控：关闭浮窗。
    void listen<{ id?: string }>("float-lyric-closed", (event) => handleFloatLyricClosed(event.payload?.id))
      .then((cleanup) => {
        unlistenClosed = cleanup;
        if (disposed) cleanup();
      })
      .catch((error) => console.error("[LyricSync] float lyric close listener failed", {
        error,
        tauri: getTauriRuntimeDiagnostics(),
      }));

    // 浮窗回控：锁定状态变化。
    void listen<{ locked?: boolean }>("float-lyric-lock-changed", (event) => {
      settings.patch({
        lyricWindow: {
          ...$settings.lyricWindow,
          locked: Boolean(event.payload?.locked),
        },
      });
    })
      .then((cleanup) => {
        unlistenLockChanged = cleanup;
        if (disposed) cleanup();
      })
      .catch((error) => console.error("[LyricSync] float lyric lock listener failed", {
        error,
        tauri: getTauriRuntimeDiagnostics(),
      }));

    // 浮窗回控：工具栏改设置（字体/背景等）。
    void listen<{ lyricWindow?: Partial<AppSettings["lyricWindow"]> }>("float-lyric-settings-changed", (event) => {
      const payload = event.payload;
      if (!payload?.lyricWindow) return;
      settings.patch({
        lyricWindow: {
          ...$settings.lyricWindow,
          ...payload.lyricWindow,
        },
      });
    })
      .then((cleanup) => {
        unlistenSettingsChanged = cleanup;
        if (disposed) cleanup();
      })
      .catch((error) => console.error("[LyricSync] float lyric settings listener failed", {
        error,
        tauri: getTauriRuntimeDiagnostics(),
      }));

    function handleWindowVisibilityChange() {
      if (!$settings.enableLyricFloatingWindow) return;
      if ($settings.hideLyricFloatingWindowWhenMainVisible && document.visibilityState === "visible") {
        hideFloatWindow();
        return;
      }
      void showFloatWindow()
        .then(() => syncFloatWindowNow())
        .catch((error) => console.error("[LyricSync] show float on visibility change failed", {
          error,
          tauri: getTauriRuntimeDiagnostics(),
        }));
    }
    window.addEventListener("focus", handleWindowVisibilityChange);
    document.addEventListener("visibilitychange", handleWindowVisibilityChange);

    return () => {
      disposed = true;
      unlistenReady?.();
      unlistenVariant?.();
      unlistenClosed?.();
      unlistenLockChanged?.();
      unlistenSettingsChanged?.();
      window.removeEventListener("focus", handleWindowVisibilityChange);
      document.removeEventListener("visibilitychange", handleWindowVisibilityChange);
    };
  });

  $effect(() => {
    const track = $playerState.currentTrack;
    if (!track) {
      lyricLines = [];
      lastTrackId = "";
      lastLyricSignature = "";
      lastPayloadKey = "";
      lyricLoadingTrackId = "";
      sendFloatingLyric({ lyric: WAITING_LYRIC, tlyric: "" });
      return;
    }

    const inlineLyric = track.lyric ?? "";
    const lyricSignature = `${track.id}|${inlineLyric}|${track.lyric_url ?? ""}|${track.tlyric_url ?? ""}`;
    const isNewTrack = track.id !== lastTrackId;
    if (!isNewTrack && lyricSignature === lastLyricSignature) return;

    lastTrackId = track.id;
    lastLyricSignature = lyricSignature;
    lastPayloadKey = "";
    if (isNewTrack) {
      floatingTranslationIndex = 0;
      lyricLines = [];
      lyricLoadingTrackId = track.id;
      settleLyricLoading(track.id);
    }

    if (inlineLyric.trim()) {
      lyricLoadingTrackId = "";
      lyricLines = parseLyric(inlineLyric);
      syncCurrentLyric(true);
    } else if (isNewTrack) {
      sendFloatingLyric(fallbackPayload());
    }

    MediaService.getLyric(track.id, track.album_id ?? "", track.lyric_url, track.tlyric_url)
      .then((result) => {
        if (track.id !== $playerState.currentTrack?.id) return;
        lyricLoadingTrackId = "";
        const lyric = result.lyric || inlineLyric;
        lyricLines = lyric ? parseLyric(lyric, result.tlyric) : [];
        syncCurrentLyric(true);
      })
      .catch((error) => {
        console.error("[LyricSync] get lyric failed", error);
        lyricLoadingTrackId = "";
        syncCurrentLyric(true);
      });
  });

  $effect(() => {
    const enabled = $settings.enableLyricFloatingWindow;
    const hideWhenMainVisible = $settings.hideLyricFloatingWindowWhenMainVisible;
    // 仅 false→true 的显式开启视为「用户主动开启」，此时无条件显示浮窗以给出反馈、便于定位；
    // 挂载或其它设置变化则遵循「主界面可见时隐藏」。后续自动隐藏由 focus/visibilitychange 事件驱动。
    const justEnabledByUser = prevFloatEnabled === false && enabled;
    // 忽略 enabled/hideWhenMainVisible 以外的设置变化（如 lock 切换），
    // 否则 settings.patch 替换整个对象导致此 effect 误触发 hideFloatWindow。
    const relevantChanged = enabled !== prevFloatEnabled || hideWhenMainVisible !== prevHideWhenMainVisible;
    prevFloatEnabled = enabled;
    prevHideWhenMainVisible = hideWhenMainVisible;
    if (!relevantChanged) return;

    if (!enabled) {
      hideFloatWindow();
      return;
    }
    if (!justEnabledByUser && hideWhenMainVisible && document.visibilityState === "visible") {
      hideFloatWindow();
      return;
    }
    void showFloatWindow()
      .then(() => syncFloatWindowNow())
      .catch((error) => console.error("[LyricSync] show float window failed", {
        error,
        tauri: getTauriRuntimeDiagnostics(),
      }));
  });

  $effect(() => {
    // 显式读取所有依赖，确保歌词行、播放进度、变体模式变化时都会重新推送。
    lyricLines;
    floatingVariantMode;
    floatingTranslationIndex;
    $playerState.position;
    $settings.lyricWindow.offsetMs;
    syncCurrentLyric();
  });

  $effect(() => {
    $settings.enableLyricFloatingWindow;
    $settings.hideLyricFloatingWindowWhenMainVisible;
    $settings.lyricWindow.fontSize;
    $settings.lyricWindow.color;
    $settings.lyricWindow.colorScheme;
    $settings.lyricWindow.backgroundAlpha;
    $settings.lyricWindow.lines;
    $settings.lyricWindow.offsetMs;
    $settings.lyricWindow.staggeredLayout;
    $settings.lyricWindow.locked;
    $settings.lyricWindow.rememberLockState;
    $settings.lyricWindow.variantMode;
    syncFloatingLyricSettings();
  });
</script>
