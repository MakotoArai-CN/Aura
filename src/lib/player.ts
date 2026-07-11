import { Howl, Howler } from "howler";
import { playerState, type Track, type LoopMode } from "./stores/player";
import { get } from "svelte/store";
import { MediaService } from "./providers/index";
import { localmusic } from "./providers/localmusic";
import { isTauriRuntime } from "./tauri";
import { proxyResourceUrl } from "./resourceUrl";
import { settings } from "./stores/settings";
import { toast } from "./stores/toast";

const BROKEN_STREAM_PREFIX = "http://stream.localhost/";
const LEGACY_STREAM_PREFIX = "stream://localhost/";
const TAURI_STREAM_PREFIX = "stream://localhost/";
const LOCAL_STREAM_RE = /^http:\/\/(?:127\.0\.0\.1|localhost):\d+\/stream\/([^?]+)/i;

class Listen1Player {
  private playlist: Track[] = [];
  private howls = new Map<string, Howl>();
  private index = -1;
  private _loopMode: LoopMode = 0;
  private refreshTimer: ReturnType<typeof setInterval> | null = null;
  private playedFrom = 0;
  private preloadTrackId: string | null = null;
  private pauseFadeTimer: ReturnType<typeof setTimeout> | null = null;
  private readonly preloadThresholdSeconds = 12;
  private readonly fadeInMs = 160;

  // Auto-skip protection: counts consecutive failures in one play session
  private _failCount = 0;
  private _isAutoSkipping = false;
  private failoverAttemptIds = new Set<string>();
  private failedSourcesByTrackKey = new Map<string, Set<string>>();
  private handlingFailedTrackIds = new Set<string>();
  private failureConfirmTimers = new Map<string, ReturnType<typeof setTimeout>>();
  private transientRetryKeys = new Set<string>();
  private lastPlayAtByTrackId = new Map<string, number>();
  private manualSkipDirection: "next" | "prev" | "random" | null = null;

  constructor() {
    this.setRefreshRate(5);
    this.restoreFromStorage();
    this.setupMediaSession();
  }

  // ─── Internal ─────────────────────────────────────────────

  private setRefreshRate(fps: number) {
    if (this.refreshTimer) clearInterval(this.refreshTimer);
    this.refreshTimer = setInterval(() => {
      const h = this.currentHowl;
      if (h && h.playing()) {
        const pos = h.seek() as number;
        const duration = h.duration() ?? 0;
        playerState.patch({ position: pos, playing: true, duration });
        this.maybePreloadUpcoming(pos, duration);
      }
    }, 1000 / fps);
  }

  private setupMediaSession() {
    if (!("mediaSession" in navigator)) return;
    navigator.mediaSession.setActionHandler("play", () => this.play());
    navigator.mediaSession.setActionHandler("pause", () => this.pause());
    navigator.mediaSession.setActionHandler("nexttrack", () => { this._failCount = 0; this.skip("next"); });
    navigator.mediaSession.setActionHandler("previoustrack", () => { this._failCount = 0; this.skip("prev"); });
  }

  private updateMediaSession() {
    if (!("mediaSession" in navigator)) return;
    const t = this.currentTrack;
    if (!t) return;
    navigator.mediaSession.metadata = new MediaMetadata({
      title: t.title,
      artist: t.artist,
      album: t.album ?? "",
      artwork: t.img_url ? [{ src: proxyResourceUrl(t.img_url) }] : [],
    });
    navigator.mediaSession.playbackState = this.currentHowl?.playing() ? "playing" : "paused";
  }

  private syncState() {
    const track = this.currentTrack;
    const h = this.currentHowl;
    playerState.patch({
      playlist: [...this.playlist],
      currentIndex: this.index,
      currentTrack: track,
      playing: h ? h.playing() : false,
      duration: h ? (h.duration() ?? 0) : 0,
    });
  }

  private isCurrentTrack(track: Track): boolean {
    return this.currentTrack?.id === track.id;
  }

  private targetHowlVolume(): number {
    const volume = Howler.volume();
    return Number.isFinite(volume) ? Math.max(0, Math.min(1, volume)) : 1;
  }

  private clearPauseFadeTimer() {
    if (!this.pauseFadeTimer) return;
    clearTimeout(this.pauseFadeTimer);
    this.pauseFadeTimer = null;
  }

  private clearFailureConfirm(trackId: string) {
    const timer = this.failureConfirmTimers.get(trackId);
    if (!timer) return;
    clearTimeout(timer);
    this.failureConfirmTimers.delete(trackId);
  }

  private startHowl(h: Howl, fade = true) {
    this.clearPauseFadeTimer();
    const state = get(playerState);
    const storedVolume = Math.max(0.01, Math.min(1, (state.volume || 90) / 100));
    Howler.mute(Boolean(state.muted));
    if (!state.muted && this.targetHowlVolume() <= 0) {
      Howler.volume(storedVolume);
    }
    const normalizedVolume = state.muted ? 0 : Math.max(0.01, this.targetHowlVolume() || storedVolume);
    playerState.patch({ playing: true, loading: false, muted: state.muted });
    if (!fade) {
      h.volume(normalizedVolume);
      h.play();
      this.updateMediaSession();
      return;
    }

    try {
      const currentVolume = Number(h.volume());
      const fromVolume = h.playing()
        ? Math.max(0, Math.min(normalizedVolume, Number.isFinite(currentVolume) ? currentVolume : 0))
        : 0;
      h.volume(fromVolume);
      if (!h.playing()) h.play();
      h.fade(fromVolume, normalizedVolume, this.fadeInMs);
      this.updateMediaSession();
    } catch {
      h.volume(normalizedVolume);
      h.play();
      this.updateMediaSession();
    }
  }

  private upcomingTrackForPreload(): Track | null {
    if (this.index < 0 || this.playlist.length < 2 || this._loopMode !== 0) return null;
    const nextIndex = this.safeIndex(this.index + 1);
    if (nextIndex === this.index) return null;
    const track = this.playlist[nextIndex];
    return track && !track.disabled ? track : null;
  }

  private maybePreloadUpcoming(position: number, duration: number) {
    if (!Number.isFinite(duration) || duration <= 0) return;
    if (duration - position > this.preloadThresholdSeconds) return;
    void this.preloadUpcoming();
  }

  private pruneHowls(extraKeepIds: string[] = []) {
    const keep = new Set(extraKeepIds);
    if (this.currentTrack) keep.add(this.currentTrack.id);
    const preloadTrack = this.upcomingTrackForPreload();
    if (preloadTrack) keep.add(preloadTrack.id);

    for (const [id, howl] of this.howls) {
      if (keep.has(id)) continue;
      howl.unload();
      this.howls.delete(id);
    }
  }

  private buildHowl(track: Track, autoplay = false): Howl {
    const src = track.sound_url ?? track.url ?? "";
    const self = this;
    const h = new Howl({
      src: [src],
      format: this.inferHowlFormats(src),
      html5: true,
      preload: true,
      autoplay,
      volume: autoplay ? 0 : Howler.volume(),
      onplay() {
        if (!self.isCurrentTrack(track)) return;
        const state = get(playerState);
        const storedVolume = Math.max(0.01, Math.min(1, (state.volume || 90) / 100));
        if (!state.muted) {
          Howler.mute(false);
          if (self.targetHowlVolume() <= 0) Howler.volume(storedVolume);
          if (Number(h.volume()) <= 0 && !autoplay) h.volume(self.targetHowlVolume() || storedVolume);
        }
        self.manualSkipDirection = null;
        self.clearFailureConfirm(track.id);
        self.lastPlayAtByTrackId.set(track.id, Date.now());
        self._failCount = 0;
        self._isAutoSkipping = false;
        playerState.patch({ playing: true, loading: false, muted: state.muted });
        self.updateMediaSession();
        if (autoplay) h.fade(0, state.muted ? 0 : (self.targetHowlVolume() || storedVolume), self.fadeInMs);
        window.dispatchEvent(new CustomEvent("l1:play_state", { detail: { isPlaying: true, track } }));
      },
      onpause() {
        if (!self.isCurrentTrack(track)) return;
        playerState.patch({ playing: false });
        self.updateMediaSession();
        window.dispatchEvent(new CustomEvent("l1:play_state", { detail: { isPlaying: false, reason: "Paused" } }));
      },
      onstop() {
        if (!self.isCurrentTrack(track)) return;
        playerState.patch({ playing: false, position: 0 });
      },
      onend() {
        if (!self.isCurrentTrack(track)) return;
        const position = Number(h.seek() || 0);
        const duration = Number(h.duration() || 0);
        const playAge = Date.now() - (self.lastPlayAtByTrackId.get(track.id) ?? 0);
        if ((!Number.isFinite(duration) || duration <= 1 || position < 1) && playAge < 5000) {
          self.scheduleTrackFailure(track, h, "early-end");
          return;
        }
        self.playedFrom = Date.now();
        playerState.patch({ playing: false });
        self.updateMediaSession();
        window.dispatchEvent(new CustomEvent("l1:play_state", { detail: { isPlaying: false, reason: "Ended" } }));
        self._failCount = 0; // natural end = success, reset for next song
        self._autoSkip();
      },
      onloaderror() {
        if (!self.isCurrentTrack(track)) {
          if (self.preloadTrackId === track.id) self.preloadTrackId = null;
          self.howls.delete(track.id);
          h.unload();
          return;
        }
        self.scheduleTrackFailure(track, h, "error");
      },
      onplayerror() {
        if (!self.isCurrentTrack(track)) {
          if (self.preloadTrackId === track.id) self.preloadTrackId = null;
          self.howls.delete(track.id);
          h.unload();
          return;
        }
        self.scheduleTrackFailure(track, h, "error");
      },
      onload() {
        if (self.preloadTrackId === track.id) self.preloadTrackId = null;
        if (!self.isCurrentTrack(track)) return;
        playerState.patch({ duration: h.duration() ?? 0, loading: false });
      },
    });
    return h;
  }

  private scheduleTrackFailure(track: Track, h: Howl, reason: "error" | "early-end" = "error") {
    if (this.failureConfirmTimers.has(track.id)) return;
    const timer = setTimeout(() => {
      this.failureConfirmTimers.delete(track.id);
      if (!this.isCurrentTrack(track)) return;

      const position = Number(h.seek() || 0);
      const playedRecently = Date.now() - (this.lastPlayAtByTrackId.get(track.id) ?? 0) < 2000;
      if (h.playing() || position > 0.35 || playedRecently) {
        playerState.patch({ playing: h.playing(), loading: false, position });
        return;
      }

      const retryTarget = this.decodeStreamTarget(track.sound_url ?? "") ?? track.url ?? track.sound_url ?? "";
      const retryKey = `${track.id}::${retryTarget}`;
      if (retryTarget && !this.transientRetryKeys.has(retryKey)) {
        this.transientRetryKeys.add(retryKey);
        this.howls.get(track.id)?.unload();
        this.howls.delete(track.id);
        track.disabled = false;
        delete track.sound_url;
        playerState.patch({ loading: true, playing: false });
        void this._resolveAndPlay(track);
        return;
      }

      if (reason === "early-end") {
        track.disabled = false;
        playerState.patch({ loading: false, playing: false });
        this.syncState();
        return;
      }

      playerState.patch({ loading: false });
      void this._onTrackFailed(track);
    }, 900);
    this.failureConfirmTimers.set(track.id, timer);
  }

  private async _onTrackFailed(track?: Track) {
    const failedId = track?.id ?? "";
    if (failedId && this.handlingFailedTrackIds.has(failedId)) return;
    if (failedId) this.handlingFailedTrackIds.add(failedId);

    try {
      if (track && await this.tryFailoverTrack(track)) return;

      if (track) track.disabled = true;
      this.syncState();
      this._failCount++;
      const total = this.playlist.filter((t) => !t.disabled).length;
      if (total === 0 || this._failCount >= this.playlist.length) {
        this._failCount = 0;
        this._isAutoSkipping = false;
        this.manualSkipDirection = null;
        playerState.patch({ loading: false, playing: false });
        return;
      }
      const manualDirection = this.manualSkipDirection;
      if (manualDirection) {
        setTimeout(() => this.continueManualSkip(manualDirection), 0);
      } else {
        this._isAutoSkipping = true;
        setTimeout(() => this._autoSkip(), 0);
      }
    } finally {
      if (failedId) this.handlingFailedTrackIds.delete(failedId);
    }
  }

  private failoverKey(track: Track): string {
    return `${track.title.trim().toLowerCase()}::${track.artist.trim().toLowerCase()}`;
  }

  private async tryFailoverTrack(track: Track): Promise<boolean> {
    if (this.isLocalTrack(track)) return false;
    if (this.isUnsupportedMp4Track(track)) return false;
    const key = this.failoverKey(track);
    const failedSources = this.failedSourcesByTrackKey.get(key) ?? new Set<string>();
    const currentSource = track.platform || track.source;
    if (currentSource) failedSources.add(currentSource);
    if (track.source) failedSources.add(track.source);
    this.failedSourcesByTrackKey.set(key, failedSources);

    const oldId = track.id;
    this.howls.get(oldId)?.unload();
    this.howls.delete(oldId);

    const attemptKey = `${key}::${[...failedSources].sort().join(",")}`;
    if (this.failoverAttemptIds.has(attemptKey)) return false;
    this.failoverAttemptIds.add(attemptKey);

    const result = await MediaService.getUrl(oldId, track, true, [...failedSources]).catch((error) => {
      console.error("[player] failover get url failed", error);
      return null;
    });
    if (!result?.url) return false;

    const replacement = result.track;
    const replacementSource = result.platform ?? replacement?.source;
    if (replacementSource) failedSources.delete(replacementSource);
    if (replacement) {
      Object.assign(track, {
        ...replacement,
        url: result.url,
        sound_url: undefined,
        bitrate: result.bitrate,
        platform: result.platform ?? replacement.source,
        disabled: false,
      });
    } else {
      track.url = result.url;
      track.sound_url = undefined;
      track.bitrate = result.bitrate;
      track.platform = result.platform;
      track.disabled = false;
    }

    playerState.patch({
      playlist: [...this.playlist],
      currentTrack: track,
      currentIndex: this.index,
      loading: true,
      playing: false,
      position: 0,
    });
    this.saveToStorage();
    await this._resolveAndPlay(track);
    return true;
  }

  private _autoSkip() {
    if (this.playlist.length === 0) return;
    const enabledIndices = this.playlist
      .map((track, index) => ({ track, index }))
      .filter(({ track }) => !track.disabled)
      .map(({ index }) => index);
    if (enabledIndices.length === 0) {
      this._isAutoSkipping = false;
      playerState.patch({ loading: false, playing: false });
      return;
    }

    let next = this.index;
    if (this._loopMode === 1) {
      if (this.playlist[this.index]?.disabled) next = enabledIndices[0];
    } else if (this._loopMode === 2) {
      next = enabledIndices[Math.floor(Math.random() * enabledIndices.length)];
    } else {
      for (let step = 1; step <= this.playlist.length; step++) {
        const candidate = (this.index + step) % this.playlist.length;
        if (!this.playlist[candidate]?.disabled) {
          next = candidate;
          break;
        }
      }
    }
    this._loadInternal(next);
  }

  private continueManualSkip(direction: "next" | "prev" | "random") {
    if (this.playlist.length === 0) return;
    const next = this.nextPlayableIndex(direction);
    if (next < 0) {
      this.manualSkipDirection = null;
      playerState.patch({ loading: false, playing: false });
      return;
    }
    this._loadInternal(next);
  }

  // Internal load — does NOT reset _failCount (called from auto-skip)
  private _loadInternal(idx: number) {
    const safeidx = this.safeIndex(idx);
    if (safeidx < 0) return;
    this.clearPauseFadeTimer();
    this.currentHowl?.stop();
    this.index = safeidx;
    const track = this.playlist[this.index];
    if (!track) return;
    this.playedFrom = Date.now();
    playerState.patch({ loading: true, position: 0, currentTrack: track, currentIndex: this.index });
    void this._resolveAndPlay(track);
    this.saveToStorage();
  }

  private decodeStreamTarget(url: string): string | null {
    const normalized = url.trim();
    const localMatch = normalized.match(LOCAL_STREAM_RE);
    const encodedTarget = localMatch?.[1]
      ?? (normalized.startsWith(BROKEN_STREAM_PREFIX) ? normalized.slice(BROKEN_STREAM_PREFIX.length).split("?")[0] : null)
      ?? (normalized.startsWith(LEGACY_STREAM_PREFIX) ? normalized.slice(LEGACY_STREAM_PREFIX.length).split("?")[0] : null);

    if (!encodedTarget) return null;
    try {
      return decodeURIComponent(encodedTarget);
    } catch {
      return encodedTarget;
    }
  }

  private normalizeUrlInput(url: string): string {
    let normalized = url.trim();
    if (/^[a-z][a-z0-9+.-]*%3A/i.test(normalized)) {
      try {
        normalized = decodeURIComponent(normalized);
      } catch {
        // Keep the original URL if it is not a valid percent-encoded URL.
      }
    }
    if (/^file:\/\/[A-Za-z]:[\\/]/.test(normalized)) {
      normalized = normalized.replace(/^file:\/\//, "file:///");
    }
    if (normalized.startsWith("file://")) {
      normalized = normalized.replace(/\\/g, "/");
    }
    return normalized;
  }

  private isUnsupportedMp4Url(url: string): boolean {
    const target = this.decodeStreamTarget(url) ?? url;
    const normalized = this.normalizeUrlInput(target);
    const path = normalized.split(/[?#]/)[0].toLowerCase();
    return path.endsWith(".mp4");
  }

  private isUnsupportedMp4Track(track: Track): boolean {
    return [track.sound_url, track.url]
      .filter((url): url is string => Boolean(url))
      .some((url) => this.isUnsupportedMp4Url(url));
  }

  private rejectUnsupportedMp4(track: Track, markLoading: boolean): false {
    track.disabled = true;
    if (markLoading) {
      toast.warn("当前前端暂不支持播放 MP4 音频文件");
      playerState.patch({ loading: false, playing: false });
      this.syncState();
      void this._onTrackFailed(track);
    }
    return false;
  }

  private streamBaseUrl(): string {
    return (window as Window & { __LISTEN1_STREAM_BASE_URL__?: string }).__LISTEN1_STREAM_BASE_URL__ ?? "";
  }

  private normalizeComparableText(value: string | undefined): string {
    return (value ?? "")
      .toLowerCase()
      .replace(/[（(].*?[）)]/g, "")
      .replace(/[\s._\-·,，、/\\|&]+/g, "")
      .trim();
  }

  private primaryArtist(value: string | undefined): string {
    return (value ?? "").split(/[,，、/&|;；]/)[0]?.trim() ?? "";
  }

  private parseBitrateKbps(value: string | undefined): number | null {
    if (!value) return null;
    const match = value.toLowerCase().match(/(\d+(?:\.\d+)?)\s*(k|kbps|m|mbps)?/);
    if (!match) return null;
    const raw = Number(match[1]);
    if (!Number.isFinite(raw) || raw <= 0) return null;
    const unit = match[2] ?? "k";
    return unit.startsWith("m") ? raw * 1000 : raw;
  }

  private async hasLocalTrackWithSufficientQuality(track: Track, networkBitrate?: string): Promise<boolean> {
    const config = get(settings).audioCache;
    if (!config.enabled || !config.skipWhenLocalQualitySufficient || this.isLocalTrack(track)) return false;

    const targetTitle = this.normalizeComparableText(track.title);
    const targetArtist = this.normalizeComparableText(this.primaryArtist(track.artist));
    if (!targetTitle || !targetArtist) return false;

    const networkKbps = this.parseBitrateKbps(networkBitrate ?? track.bitrate);
    if (networkKbps == null) return false;

    const playlist = await localmusic.get_playlist().catch(() => null);
    if (!playlist?.tracks?.length) return false;

    return playlist.tracks.some((localTrack) => {
      const localTitle = this.normalizeComparableText(localTrack.title);
      const localArtist = this.normalizeComparableText(this.primaryArtist(localTrack.artist));
      if (!localTitle || !localArtist || localTitle !== targetTitle || localArtist !== targetArtist) return false;

      if (typeof track.duration === "number" && typeof localTrack.duration === "number") {
        if (Math.abs(track.duration - localTrack.duration) > 3) return false;
      }

      const localKbps = this.parseBitrateKbps(localTrack.bitrate);
      return localKbps != null && localKbps >= networkKbps;
    });
  }

  /**
   * 本地优先：网络曲目若在本地库有同名同艺人（时长±3s）的匹配文件，返回该本地曲目，
   * 命中则改播本地 file://（零流量零缓存）。本地曲目/本地音质明显偏低（<网络一半）时不替换以免降音质。
   */
  private async findLocalReplacement(track: Track): Promise<Track | null> {
    if (this.isLocalTrack(track)) return null;
    const targetTitle = this.normalizeComparableText(track.title);
    const targetArtist = this.normalizeComparableText(this.primaryArtist(track.artist));
    if (!targetTitle || !targetArtist) return null;

    const playlist = await localmusic.get_playlist().catch(() => null);
    if (!playlist?.tracks?.length) return null;

    const networkKbps = this.parseBitrateKbps(track.bitrate);
    const match = playlist.tracks.find((localTrack) => {
      const localTitle = this.normalizeComparableText(localTrack.title);
      const localArtist = this.normalizeComparableText(this.primaryArtist(localTrack.artist));
      if (!localTitle || !localArtist || localTitle !== targetTitle || localArtist !== targetArtist) return false;
      if (typeof track.duration === "number" && typeof localTrack.duration === "number") {
        if (Math.abs(track.duration - localTrack.duration) > 3) return false;
      }
      // 避免明显降音质：本地已知 bitrate 且不到网络的一半时跳过（未知则视为可接受）。
      const localKbps = this.parseBitrateKbps(localTrack.bitrate);
      if (networkKbps != null && localKbps != null && localKbps < networkKbps / 2) return false;
      return true;
    });
    return match ?? null;
  }

  private async proxyUrl(url: string, track?: Track, networkBitrate?: string): Promise<string> {
    let normalized = this.normalizeUrlInput(url);
    const streamTarget = this.decodeStreamTarget(normalized);
    if (streamTarget) normalized = this.normalizeUrlInput(streamTarget);

    if (
      normalized.startsWith("blob:") ||
      normalized.startsWith("data:")
    ) return normalized;
    if (!isTauriRuntime()) return normalized;
    const streamBaseUrl = this.streamBaseUrl();
    const encoded = encodeURIComponent(normalized);
    const noCacheWrite = track
      ? await this.hasLocalTrackWithSufficientQuality(track, networkBitrate).catch(() => false)
      : false;
    // 稳定缓存键：平台+歌曲ID，让带时效签名的 URL 不再反复 miss / 重复落盘。
    // 本地曲目不参与（其 URL 是 file://，走本地分支不缓存）。
    const cacheId = track && track.id && !this.isLocalTrack(track)
      ? `${track.source ?? ""}:${track.id}`
      : "";
    const params = new URLSearchParams();
    if (noCacheWrite) params.set("no_cache_write", "1");
    if (cacheId) params.set("cache_key", cacheId);
    const query = params.toString();
    const suffix = query ? `?${query}` : "";
    return streamBaseUrl ? `${streamBaseUrl}${encoded}${suffix}` : `${TAURI_STREAM_PREFIX}${encoded}${suffix}`;
  }

  private inferHowlFormats(src: string): string[] | undefined {
    const target = this.decodeStreamTarget(src) ?? src;
    const path = target.split("?")[0].toLowerCase();
    if (path.endsWith(".m4s") || path.endsWith(".m4a") || path.endsWith(".aac")) return ["mp4", "m4a", "aac"];
    if (path.endsWith(".mp3")) return ["mp3"];
    if (path.endsWith(".flac")) return ["flac"];
    if (path.endsWith(".ogg") || path.endsWith(".oga")) return ["ogg"];
    if (path.endsWith(".opus")) return ["opus"];
    if (path.endsWith(".wav")) return ["wav"];
    return undefined;
  }

  private shouldRefreshExistingUrl(track: Track, url: string): boolean {
    const source = track.platform || track.source;
    if (source !== "bilibili" && !track.id.startsWith("bitrack")) return false;
    return /^https?:\/\//i.test(url);
  }

  private async resolveTrackUrl(track: Track, markLoading: boolean): Promise<boolean> {
    if (markLoading) playerState.patch({ loading: true });

    let existingUrl = track.sound_url || track.url;
    const existingTarget = existingUrl ? (this.decodeStreamTarget(existingUrl) ?? existingUrl) : "";
    if (existingTarget && this.isUnsupportedMp4Url(existingTarget)) {
      return this.rejectUnsupportedMp4(track, markLoading);
    }
    if (existingTarget && this.shouldRefreshExistingUrl(track, existingTarget)) {
      delete track.sound_url;
      delete track.url;
      existingUrl = "";
    }

    if (existingUrl === "") {
      if (track.sound_url === "") delete track.sound_url;
      if (track.url === "") delete track.url;
      existingUrl = "";
    }

    if (track.disabled) {
      track.disabled = true;
      if (markLoading) {
        this.syncState();
        void this._onTrackFailed(track);
      }
      return false;
    }

    // 本地优先：网络曲目若有本地匹配文件，改播本地 file://（零流量零缓存）。
    if (!this.isLocalTrack(track)) {
      const localMatch = await this.findLocalReplacement(track).catch(() => null);
      if (localMatch?.url) {
        try {
          track.sound_url = await this.proxyUrl(localMatch.url, localMatch, localMatch.bitrate);
          track.bitrate = localMatch.bitrate ?? track.bitrate;
          return true;
        } catch {
          // 本地文件解析失败则回退到网络路径，不阻断播放。
        }
      }
    }

    if (existingUrl) {
      try {
        track.sound_url = await this.proxyUrl(existingUrl, track, track.bitrate);
      } catch {
        track.disabled = true;
        if (markLoading) {
          this.syncState();
          void this._onTrackFailed(track);
        }
        return false;
      }
      return true;
    } else {
      try {
        const result = await MediaService.getUrl(track.id, track);
        if (result?.url) {
          if (this.isUnsupportedMp4Url(result.url)) {
            track.url = result.url;
            delete track.sound_url;
            return this.rejectUnsupportedMp4(track, markLoading);
          }
          if (result.track) Object.assign(track, { ...result.track, disabled: false });
          track.bitrate = result.bitrate;
          track.platform = result.platform;
          track.url = result.url;
          track.sound_url = await this.proxyUrl(result.url, track, result.bitrate);
        } else {
          track.disabled = true;
          if (markLoading) {
            this.syncState();
            void this._onTrackFailed(track);
          }
          return false;
        }
      } catch {
        track.disabled = true;
        if (markLoading) {
          this.syncState();
          void this._onTrackFailed(track);
        }
        return false;
      }
    }

    return true;
  }

  private async preloadUpcoming() {
    const track = this.upcomingTrackForPreload();
    if (!track || this.preloadTrackId === track.id || this.howls.has(track.id)) return;

    this.preloadTrackId = track.id;
    const resolved = await this.resolveTrackUrl(track, false).catch(() => false);
    if (!resolved) {
      if (this.preloadTrackId === track.id) this.preloadTrackId = null;
      return;
    }
    if (this.howls.has(track.id)) {
      if (this.preloadTrackId === track.id) this.preloadTrackId = null;
      return;
    }

    const h = this.buildHowl(track, false);
    this.howls.set(track.id, h);
    this.pruneHowls([track.id]);
  }

  private async _resolveAndPlay(track: Track) {
    const resolved = await this.resolveTrackUrl(track, true);
    if (!this.isCurrentTrack(track)) return;
    if (!resolved) return;

    let h = this.howls.get(track.id);
    if (!h) {
      h = this.buildHowl(track, false);
      this.howls.set(track.id, h);
    } else {
      h.stop();
      h.seek(0);
    }

    if (this.preloadTrackId === track.id) this.preloadTrackId = null;
    this.syncState();
    this.saveToStorage();
    this.startHowl(h);
    this.pruneHowls([track.id]);
  }

  private safeIndex(i: number): number {
    if (this.playlist.length === 0) return -1;
    return ((i % this.playlist.length) + this.playlist.length) % this.playlist.length;
  }

  private nextPlayableIndex(direction: "next" | "prev" | "random"): number {
    const length = this.playlist.length;
    if (length === 0) return -1;

    const enabledIndices = this.playlist
      .map((track, index) => ({ track, index }))
      .filter(({ track }) => !track.disabled)
      .map(({ index }) => index);
    if (enabledIndices.length === 0) return -1;

    if (direction === "random" || this._loopMode === 2) {
      if (enabledIndices.length === 1) return enabledIndices[0];
      let candidate = enabledIndices[Math.floor(Math.random() * enabledIndices.length)];
      if (candidate === this.index) {
        const currentEnabledIndex = enabledIndices.indexOf(candidate);
        candidate = enabledIndices[(currentEnabledIndex + 1) % enabledIndices.length];
      }
      return candidate;
    }

    const step = direction === "prev" ? -1 : 1;
    for (let offset = 1; offset <= length; offset++) {
      const candidate = this.safeIndex(this.index + step * offset);
      if (!this.playlist[candidate]?.disabled) return candidate;
    }

    return enabledIndices[0];
  }

  private clampVolume(value: number): number {
    const volume = Number(value);
    if (!Number.isFinite(volume)) return 0;
    return Math.max(0, Math.min(100, volume));
  }

  private readPlayerSettings(): Record<string, unknown> {
    const raw = localStorage.getItem("player-settings");
    if (!raw) return {};
    const parsed = JSON.parse(raw) as unknown;
    return parsed && typeof parsed === "object" ? parsed as Record<string, unknown> : {};
  }

  private savePlayerSettings(partial: Record<string, unknown> = {}) {
    const existing = this.readPlayerSettings();
    localStorage.setItem("player-settings", JSON.stringify({
      ...existing,
      playmode: this._loopMode,
      nowplaying_track_id: this.currentTrack?.id ?? null,
      volume: Math.round(this.volume),
      ...partial,
    }));
  }

  private isLocalTrack(track: Track): boolean {
    return track.source === "localmusic" || track.id.startsWith("lmtrack_");
  }

  private localFileUrlFromTrack(track: Track): string {
    let candidate = track.url || track.sound_url || "";
    const streamTarget = candidate ? this.decodeStreamTarget(candidate) : null;
    if (streamTarget) candidate = streamTarget;
    if (!candidate && track.id.startsWith("lmtrack_")) {
      candidate = track.id.slice("lmtrack_".length);
    }
    if (!candidate) return "";

    candidate = this.normalizeUrlInput(candidate);
    if (candidate.startsWith("file://")) return candidate;
    const normalizedPath = candidate.replace(/\\/g, "/");
    if (/^[A-Za-z]:\//.test(normalizedPath)) return `file:///${normalizedPath}`;
    if (normalizedPath.startsWith("/")) return `file://${normalizedPath}`;
    return "";
  }

  private normalizeTrackForQueue(track: Track): Track {
    const normalized: Track = { ...track, disabled: false };
    const soundTarget = normalized.sound_url ? this.decodeStreamTarget(normalized.sound_url) : null;
    if (soundTarget) normalized.sound_url = this.normalizeUrlInput(soundTarget);

    if (this.isLocalTrack(normalized)) {
      const fileUrl = this.localFileUrlFromTrack(normalized);
      if (fileUrl) normalized.url = fileUrl;
      delete normalized.sound_url;
    }

    return normalized;
  }

  private reviveNetworkTrack(track: Track) {
    if (this.isLocalTrack(track)) return;
    track.disabled = false;
    if (track.url === "") delete track.url;
    if (track.sound_url === "") delete track.sound_url;
  }

  private trackForStorage(track: Track): Track {
    const stored: Track = { ...track };
    const soundTarget = stored.sound_url ? this.decodeStreamTarget(stored.sound_url) : null;
    if (soundTarget) stored.sound_url = this.normalizeUrlInput(soundTarget);

    if (this.isLocalTrack(stored)) {
      const fileUrl = this.localFileUrlFromTrack(stored);
      if (fileUrl) stored.url = fileUrl;
      delete stored.sound_url;
    }

    if (!this.isLocalTrack(stored)) {
      delete stored.sound_url;
      if (stored.url && /^https?:\/\//i.test(stored.url)) delete stored.url;
    }

    return stored;
  }

  // ─── Public API ───────────────────────────────────────────

  get currentHowl(): Howl | null {
    const t = this.playlist[this.index];
    return t ? this.howls.get(t.id) ?? null : null;
  }

  get currentTrack(): Track | null {
    return this.playlist[this.index] ?? null;
  }

  play() {
    const h = this.currentHowl;
    if (h) {
      this.startHowl(h);
    } else if (this.currentTrack) {
      this._failCount = 0;
      this._resolveAndPlay(this.currentTrack);
    } else if (this.playlist.length > 0) {
      this.loadByIndex(0);
    }
  }

  pause() {
    const h = this.currentHowl;
    if (!h) return;
    this.clearPauseFadeTimer();
    playerState.patch({ playing: false });
    if (!h.playing()) {
      h.pause();
      this.updateMediaSession();
      return;
    }

    try {
      h.pause();
      h.volume(this.targetHowlVolume());
    } catch {
      h.pause();
      h.volume(this.targetHowlVolume());
    }
    this.updateMediaSession();
  }

  togglePlayPause() {
    const h = this.currentHowl;
    if (h?.playing()) this.pause();
    else this.play();
  }

  // Public loadByIndex — resets fail counter (user-initiated)
  loadByIndex(idx: number) {
    this._failCount = 0;
    this._isAutoSkipping = false;
    this.failoverAttemptIds.clear();
    this.failedSourcesByTrackKey.clear();
    this.handlingFailedTrackIds.clear();
    this.failureConfirmTimers.forEach((timer) => clearTimeout(timer));
    this.failureConfirmTimers.clear();
    this.transientRetryKeys.clear();
    const safeidx = this.safeIndex(idx);
    if (safeidx >= 0) this.reviveNetworkTrack(this.playlist[safeidx]);
    this._loadInternal(idx);
  }

  playById(id: string) {
    const idx = this.playlist.findIndex((t) => t.id === id);
    if (idx >= 0) this.loadByIndex(idx);
  }

  // Public skip — resets fail counter (user-initiated)
  skip(direction: "next" | "prev" | "random") {
    if (this.playlist.length === 0) return;
    this._failCount = 0;
    this._isAutoSkipping = false;
    this.failoverAttemptIds.clear();
    this.failedSourcesByTrackKey.clear();
    this.handlingFailedTrackIds.clear();
    this.failureConfirmTimers.forEach((timer) => clearTimeout(timer));
    this.failureConfirmTimers.clear();
    this.transientRetryKeys.clear();
    const next = this.nextPlayableIndex(direction);
    if (next < 0) {
      playerState.patch({ loading: false, playing: false });
      return;
    }
    this.manualSkipDirection = direction;
    this._loadInternal(next);
  }

  seek(percent: number) {
    const h = this.currentHowl;
    if (h) h.seek((percent / 100) * h.duration());
  }

  seekRelative(seconds: number) {
    const h = this.currentHowl;
    if (!h) return;
    const duration = Number(h.duration() || 0);
    if (!Number.isFinite(duration) || duration <= 0) return;
    const current = Number(h.seek() || 0);
    const next = Math.max(0, Math.min(duration, current + seconds));
    h.seek(next);
    playerState.patch({ position: next, duration });
  }

  get volume(): number { return Howler.volume() * 100; }
  set volume(v: number) {
    this.setVolume(v);
  }

  setVolume(v: number, persist = true) {
    const volume = this.clampVolume(v);
    Howler.volume(volume / 100);
    playerState.patch({ volume });
    if (persist) this.savePlayerSettings({ volume: Math.round(volume) });
  }

  commitVolume() {
    this.savePlayerSettings({ volume: Math.round(this.volume) });
  }

  adjustVolume(increase: boolean) {
    this.setVolume(this.volume + (increase ? 5 : -5));
  }

  mute() { Howler.mute(true); playerState.patch({ muted: true }); }
  unmute() { Howler.mute(false); playerState.patch({ muted: false }); }
  toggleMute() { get(playerState).muted ? this.unmute() : this.mute(); }

  get loopMode(): LoopMode { return this._loopMode; }
  set loopMode(m: LoopMode) {
    this._loopMode = m;
    playerState.patch({ loopMode: m });
    this.savePlayerSettings({ playmode: m });
  }

  setPlaylist(tracks: Track[]) {
    this.clearPauseFadeTimer();
    this.currentHowl?.stop();
    this.howls.forEach((h) => h.unload());
    this.howls.clear();
    this.preloadTrackId = null;
    this._failCount = 0;
    this._isAutoSkipping = false;
    this.failoverAttemptIds.clear();
    this.failedSourcesByTrackKey.clear();
    this.handlingFailedTrackIds.clear();
    this.failureConfirmTimers.forEach((timer) => clearTimeout(timer));
    this.failureConfirmTimers.clear();
    this.transientRetryKeys.clear();
    this.playlist = tracks.map((t) => this.normalizeTrackForQueue(t));
    this.index = -1;
    playerState.patch({ playlist: [...this.playlist], currentIndex: -1, currentTrack: null });
    this.saveToStorage();
  }

  appendTracks(tracks: Track[]) {
    const existing = new Set(this.playlist.map((t) => t.id));
    this.playlist.push(...tracks.filter((t) => !existing.has(t.id)).map((t) => this.normalizeTrackForQueue(t)));
    playerState.patch({ playlist: [...this.playlist] });
    this.saveToStorage();
  }

  insertTrack(track: Track, afterId?: string) {
    if (this.playlist.find((t) => t.id === track.id)) return;
    const queuedTrack = this.normalizeTrackForQueue(track);
    if (afterId) {
      const idx = this.playlist.findIndex((t) => t.id === afterId);
      this.playlist.splice(idx + 1, 0, queuedTrack);
    } else {
      this.playlist.push(queuedTrack);
    }
    playerState.patch({ playlist: [...this.playlist] });
    this.saveToStorage();
  }

  removeTrack(index: number) {
    if (index === this.index) {
      this.clearPauseFadeTimer();
      this.currentHowl?.stop();
    }
    if (index < this.index) this.index--;
    this.playlist.splice(index, 1);
    playerState.patch({ playlist: [...this.playlist], currentIndex: this.index });
    this.saveToStorage();
  }

  clearPlaylist() {
    this.clearPauseFadeTimer();
    this.currentHowl?.stop();
    this.howls.forEach((h) => h.unload());
    this.howls.clear();
    this._failCount = 0;
    this.failoverAttemptIds.clear();
    this.failedSourcesByTrackKey.clear();
    this.handlingFailedTrackIds.clear();
    this.failureConfirmTimers.forEach((timer) => clearTimeout(timer));
    this.failureConfirmTimers.clear();
    this.transientRetryKeys.clear();
    this.preloadTrackId = null;
    this.playlist = [];
    this.index = -1;
    playerState.patch({ playlist: [], currentIndex: -1, currentTrack: null, playing: false });
    this.saveToStorage();
  }

  getPlayedFrom(): number { return this.playedFrom; }
  getTrackById(id: string): Track | undefined { return this.playlist.find((t) => t.id === id); }

  private saveToStorage() {
    localStorage.setItem("current-playing", JSON.stringify(this.playlist.map((t) => this.trackForStorage(t))));
    this.savePlayerSettings();
  }

  private restoreFromStorage() {
    try {
      const stored = localStorage.getItem("current-playing");
      if (stored) {
        const tracks: Track[] = JSON.parse(stored);
        this.playlist = tracks.map((t) => this.normalizeTrackForQueue(t));
          playerState.patch({ playlist: [...this.playlist] });
        }
      const settings = this.readPlayerSettings();
      if (typeof settings.volume === "number") {
        this.setVolume(settings.volume, false);
      }
      if (typeof settings.playmode === "number" && [0, 1, 2].includes(settings.playmode)) {
        this._loopMode = settings.playmode as LoopMode;
        playerState.patch({ loopMode: this._loopMode });
      }
      if (settings.nowplaying_track_id) {
        const trackId = String(settings.nowplaying_track_id);
        if (trackId) {
          const idx = this.playlist.findIndex((t) => t.id === trackId);
          if (idx >= 0) {
            this.index = idx;
            playerState.patch({ currentIndex: idx, currentTrack: this.playlist[idx] });
          }
        }
      } else if (this.playlist.length > 0) {
        this.index = 0;
        playerState.patch({ currentIndex: 0, currentTrack: this.playlist[0] });
      }
    } catch {}
  }
}

export const player = new Listen1Player();
window.l1Player = player;
