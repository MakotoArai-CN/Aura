import { Howl, Howler } from "howler";
import { playerState, playbackClock, type Track, type LoopMode } from "./stores/player";
import { get } from "svelte/store";
import { MediaService } from "./providers/index";
import { localmusic } from "./providers/localmusic";
import { isTauriRuntime, audioCacheLookup } from "./tauri";
import { proxyResourceUrl } from "./resourceUrl";
import { settings } from "./stores/settings";
import { deviceTier } from "./stores/device";
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
  private playedFrom = 0;
  private preloadTrackId: string | null = null;
  /** 每次加载自增，用来判断一次异步解析回来时是否已被更新的加载接棒。 */
  private loadSeq = 0;
  private preloadBackoffUntil = 0;
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

  // ─── 断网保护 ───────────────────────────────────────────
  // 之前只有「连续失败次数 >= 播放列表长度」这一个上限，断网时会把整个列表
  // 飞速走一遍（每首 900ms 确认 + 一次换源请求），表现为疯狂切下一曲。
  // 现在：离网直接停住，在线也把连续失败上限压到 MAX_CONSECUTIVE_FAILURES，
  // 并给失败驱动的切歌加最小间隔，避免刷屏式跳曲。
  private static readonly MAX_CONSECUTIVE_FAILURES = 6;
  private static readonly FAILURE_SKIP_SPACING_MS = 700;
  /** 断网自动续播时最多连跳几首有缓存的歌，防止缓存被清导致的新一轮跳曲循环。 */
  private static readonly MAX_OFFLINE_ADVANCES = 3;
  /** 断网期间探测网络是否恢复的心跳间隔。 */
  private static readonly NETWORK_PROBE_INTERVAL_MS = 15_000;
  private online = typeof navigator === "undefined" ? true : navigator.onLine !== false;
  private offlineNoticeAt = 0;
  private lastFailureSkipAt = 0;
  /** 失败驱动切歌的延迟句柄。必须留着，否则这段间隔内用户点的歌会被它顶掉。 */
  private failureSkipTimer: ReturnType<typeof setTimeout> | null = null;
  /** 当前这次加载是否由自动续播触发（而非用户点歌）。断网时只有自动续播才会跳到下一首有缓存的歌。 */
  private autoAdvanceInFlight = false;
  private offlineAdvanceCount = 0;
  /**
   * 断网期间可播放（已缓存或本地）曲目的 id 集合，null = 还没扫过。
   * 断网瞬间就整表扫一遍，之后切歌只查内存集合：既不用切一首查一次 IPC，
   * 也不会出现「先加载再失败再跳」的实时试错。用 id 而非下标，删歌不会错位。
   */
  private offlinePlayableIds: Set<string> | null = null;
  /** 并发去重：多个入口同时要用可播放集合时只扫一次。 */
  private offlineScanPromise: Promise<Set<string>> | null = null;
  private networkProbeTimer: ReturnType<typeof setInterval> | null = null;
  private networkProbeInFlight = false;

  constructor() {
    this.restoreFromStorage();
    this.setupMediaSession();
    this.setupNetworkWatch();
    this.setupLocalMetaWatch();
    window.addEventListener("beforeunload", () => {
      if (this.saveStorageTimer) {
        clearTimeout(this.saveStorageTimer);
        this.saveStorageTimer = null;
        this.writeQueueToStorage();
      }
    });
  }

  /**
   * 本地歌的元数据（内嵌封面/歌手，或联网补齐的结果）是异步补上来的，而队列里存的是
   * Track 的内存副本。localmusic 那边写完存储会广播这个事件，这里把补丁打到队列上，
   * 否则补到的封面得等下次重新进歌单才看得到。
   *
   * 用事件而不是直接调用：player.ts 已经 import 了 providers/localmusic，反向 import
   * 会成环。
   */
  private setupLocalMetaWatch() {
    if (typeof window === "undefined") return;
    window.addEventListener("listen1-local-meta-updated", (event) => {
      const detail = (event as CustomEvent<{ trackId?: string; patch?: Partial<Track> }>).detail;
      const trackId = detail?.trackId;
      const patch = detail?.patch;
      if (!trackId || !patch) return;
      const index = this.playlist.findIndex((t) => t?.id === trackId);
      if (index < 0) return;
      this.playlist[index] = { ...this.playlist[index], ...patch };
      this.saveToStorage();
      this.syncState();
      if (this.currentTrack?.id === trackId) this.updateMediaSession();
    });
  }

  // ─── 断网保护 ─────────────────────────────────────────────

  private setupNetworkWatch() {
    if (typeof window === "undefined") return;
    window.addEventListener("offline", () => this.handleNetworkLost());
    window.addEventListener("online", () => void this.handleNetworkRecovered());
    // 冷启动就处于断网状态时也要先把可播放集合扫出来，并开启心跳。
    if (!this.online) this.handleNetworkLost();
  }

  /** 断网瞬间：整表扫一遍可播放（已缓存/本地）曲目，并开始心跳探测恢复。 */
  private handleNetworkLost() {
    const wasOnline = this.online;
    this.online = false;
    if (wasOnline) this.offlinePlayableIds = null;
    void this.ensureOfflinePlayableScan();
    this.startNetworkProbe();
  }

  /**
   * 网络恢复：把断网期间的 disabled 标记全部放行，回到正常列表。
   * 不自动开播——用户可能已经手动暂停，恢复网络不该抢走控制权。
   */
  private async handleNetworkRecovered() {
    if (this.online) return;
    // 只有真提示过「网络已断开」才回报恢复，避免几秒的网络抖动也弹一条 toast。
    const hadOfflineNotice = this.offlineNoticeAt > 0;
    this.online = true;
    this.offlineNoticeAt = 0;
    this.offlinePlayableIds = null;
    this.offlineScanPromise = null;
    this.stopNetworkProbe();
    // 断网期间被标记为不可用的曲目重新放行，否则恢复网络后列表还是空的。
    this.clearNetworkDisabledFlags();
    if (hadOfflineNotice) toast.success("网络已恢复，已回到完整播放列表");
  }

  private startNetworkProbe() {
    if (this.networkProbeTimer || typeof window === "undefined") return;
    this.networkProbeTimer = setInterval(
      () => void this.probeNetwork(),
      Listen1Player.NETWORK_PROBE_INTERVAL_MS,
    );
  }

  private stopNetworkProbe() {
    if (!this.networkProbeTimer) return;
    clearInterval(this.networkProbeTimer);
    this.networkProbeTimer = null;
  }

  /**
   * 心跳：先看 navigator.onLine（假阴性极少），再用当前曲目真实解析一次 URL 验证。
   * 只在浅拷贝上解析，绝不写回 track，避免探测污染真实播放状态。
   */
  private async probeNetwork() {
    if (this.online) {
      this.stopNetworkProbe();
      return;
    }
    if (this.networkProbeInFlight) return;
    if (typeof navigator !== "undefined" && navigator.onLine === false) return;
    const probeTrack = this.playlist.find((t) => t && !this.isLocalTrack(t));
    if (!probeTrack) {
      // 全是本地曲目，没法用业务接口探测，退化为信任 navigator.onLine。
      await this.handleNetworkRecovered();
      return;
    }
    this.networkProbeInFlight = true;
    try {
      const probe = { ...probeTrack } as Track;
      const result = await MediaService.getUrl(probe.id, probe).catch(() => null);
      if (result?.url) await this.handleNetworkRecovered();
    } finally {
      this.networkProbeInFlight = false;
    }
  }

  /** 扫描（或复用上次扫描结果）断网期间可播放的曲目 id 集合。 */
  private ensureOfflinePlayableScan(): Promise<Set<string>> {
    if (this.offlinePlayableIds) return Promise.resolve(this.offlinePlayableIds);
    if (this.offlineScanPromise) return this.offlineScanPromise;
    this.offlineScanPromise = this.scanOfflinePlayable().finally(() => {
      this.offlineScanPromise = null;
    });
    return this.offlineScanPromise;
  }

  private async scanOfflinePlayable(): Promise<Set<string>> {
    const ids = new Set<string>();
    // 快照下标遍历：扫描期间用户删歌也不会漏扫或越界。
    for (const track of [...this.playlist]) {
      if (!track?.id) continue;
      if (this.isLocalTrack(track)) {
        ids.add(track.id);
        continue;
      }
      if (await this.cachedFileUrl(track)) ids.add(track.id);
    }
    // 扫描是异步的，期间可能已经恢复网络，此时结果作废。
    if (this.online) return ids;
    this.offlinePlayableIds = ids;
    return ids;
  }

  /**
   * 断网期间新入队的曲目要单独补扫，否则它们永远进不了可播放集合，
   * 明明有缓存也会被跳过。只查新增的几首，不整表重扫。
   */
  private async patchOfflinePlayable(added: Track[]) {
    if (this.online || added.length === 0) return;
    const ids = await this.ensureOfflinePlayableScan();
    for (const track of added) {
      if (!track?.id || ids.has(track.id)) continue;
      if (this.isLocalTrack(track) || (await this.cachedFileUrl(track))) ids.add(track.id);
    }
  }

  private isOfflinePlayable(track: Track | undefined): boolean {
    if (!track?.id) return false;
    if (this.isLocalTrack(track)) return true;
    return this.offlinePlayableIds?.has(track.id) ?? false;
  }

  /** 断网时按方向在「预扫出的可播放集合」里找下一首；找不到返回 -1。 */
  private async offlinePlayableIndex(from: number, direction: "next" | "prev" | "random" = "next"): Promise<number> {
    await this.ensureOfflinePlayableScan();
    const len = this.playlist.length;
    if (len === 0) return -1;
    const base = from < 0 ? (direction === "prev" ? 0 : -1) : from;
    if (direction === "random") {
      const candidates: number[] = [];
      for (let i = 0; i < len; i++) {
        if (i !== from && this.isOfflinePlayable(this.playlist[i])) candidates.push(i);
      }
      if (candidates.length === 0) return this.isOfflinePlayable(this.playlist[from]) ? from : -1;
      return candidates[Math.floor(Math.random() * candidates.length)]!;
    }
    const step = direction === "prev" ? -1 : 1;
    for (let n = 1; n <= len; n++) {
      const candidate = ((base + step * n) % len + len) % len;
      if (this.isOfflinePlayable(this.playlist[candidate])) return candidate;
    }
    return -1;
  }

  /** 断网时的手动切歌：只在可播放集合内跳，不做「先加载再失败再跳」的实时试错。 */
  private async skipOffline(direction: "next" | "prev" | "random") {
    const target = await this.offlinePlayableIndex(this.index, direction);
    if (target < 0) {
      this.haltForOffline();
      return;
    }
    this.manualSkipDirection = direction;
    this._loadInternal(target);
  }

  private clearNetworkDisabledFlags() {
    let changed = false;
    for (const track of this.playlist) {
      if (track.disabled) {
        track.disabled = false;
        changed = true;
      }
    }
    this.failoverAttemptIds.clear();
    this.failedSourcesByTrackKey.clear();
    this.transientRetryKeys.clear();
    this._failCount = 0;
    this.offlineAdvanceCount = 0;
    if (changed) this.syncState();
  }

  /**
   * 自动续播链终止。任何「不再往下播」的出口都要走这里，否则 autoAdvanceInFlight
   * 会一直挂着 true，下一次加载失败时会被误判成自动续播，悄悄把用户点的歌换掉。
   */
  private endAutoAdvanceChain() {
    this.clearFailureSkipTimer();
    this.autoAdvanceInFlight = false;
    this.offlineAdvanceCount = 0;
  }

  /**
   * 取消排队中的失败切歌。用户主动换歌 / 换列表时必须调用：
   * 这个延迟最长 FAILURE_SKIP_SPACING_MS，期间用户点的歌会被它覆盖掉。
   */
  private clearFailureSkipTimer() {
    if (!this.failureSkipTimer) return;
    clearTimeout(this.failureSkipTimer);
    this.failureSkipTimer = null;
  }

  /** 网络不可用时停在当前曲目，而不是继续往下扫整个列表。 */
  private haltForOffline() {
    this._failCount = 0;
    this._isAutoSkipping = false;
    this.endAutoAdvanceChain();
    this.manualSkipDirection = null;
    this.stopProgressLoop();
    playerState.patch({ loading: false, playing: false });
    this.syncState();
    // 同一次断网只提示一次，避免逐曲刷 toast；
    // 若在异步扫描期间网络刚好恢复，就不要再补一条「已断开」自相矛盾的提示。
    if (!this.online && Date.now() - this.offlineNoticeAt > 20_000) {
      this.offlineNoticeAt = Date.now();
      toast.warn("网络已断开，当前只能播放已缓存的音频");
    }
    window.dispatchEvent(new CustomEvent("l1:play_state", { detail: { isPlaying: false, reason: "Offline" } }));
  }

  // ─── Internal ─────────────────────────────────────────────

  /**
   * 进度循环：仅播放期间运行（250ms），暂停/停止即清除。
   * 高频 position/duration 写入 playbackClock，不再触碰 playerState，
   * 避免全应用订阅者每 tick 失效重算；空闲时定时器不存在，CPU 占用为零。
   */
  private progressTimer: ReturnType<typeof setInterval> | null = null;

  private progressTick() {
    const h = this.currentHowl;
    if (!h || !h.playing()) return;
    const pos = Number(h.seek() ?? 0);
    const duration = Number(h.duration() ?? 0);
    playbackClock.update(Number.isFinite(pos) ? pos : 0, Number.isFinite(duration) ? duration : 0);
    this.maybePreloadUpcoming(pos, duration);
  }

  private startProgressLoop() {
    if (this.progressTimer) return;
    // 低档位降到 1Hz：每 tick 都要写 playbackClock，进度条与歌词都会跟着重算/重绘。
    // 250ms 在低配机上就是每秒 4 次无谓的布局+合成，而进度条本身按秒显示，
    // 1Hz 肉眼看不出差别。换档只在下一次开始播放时生效，够用且不需要重建定时器。
    const interval = get(deviceTier) === "low" ? 1000 : 250;
    this.progressTimer = setInterval(() => this.progressTick(), interval);
    this.progressTick();
  }

  private stopProgressLoop(finalPosition?: number) {
    if (this.progressTimer) {
      clearInterval(this.progressTimer);
      this.progressTimer = null;
    }
    if (finalPosition !== undefined) {
      const h = this.currentHowl;
      playbackClock.update(finalPosition, Number(h?.duration() ?? 0));
    }
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
    // 断网时下一首大概率不是相邻那首，预载必须对准真正会播的那首，
    // 否则切过去还是「实时加载 → 失败 → 再跳」。
    if (!this.online) return this.upcomingOfflineTrackForPreload();
    const nextIndex = this.safeIndex(this.index + 1);
    if (nextIndex === this.index) return null;
    const track = this.playlist[nextIndex];
    return track && !track.disabled ? track : null;
  }

  /** 断网预载目标：预扫集合里当前曲目之后的第一首可播放曲目（纯内存查表，无 IPC）。 */
  private upcomingOfflineTrackForPreload(): Track | null {
    const len = this.playlist.length;
    for (let n = 1; n < len; n++) {
      const candidate = (this.index + n) % len;
      if (candidate === this.index) break;
      const track = this.playlist[candidate];
      if (this.isOfflinePlayable(track)) return track ?? null;
    }
    return null;
  }

  private maybePreloadUpcoming(position: number, duration: number) {
    if (!Number.isFinite(duration) || duration <= 0) return;
    if (duration - position > this.preloadThresholdSeconds) return;
    // 解析失败退避：避免结尾窗口内每 250ms 重试 IPC/网络。
    if (Date.now() < this.preloadBackoffUntil) return;
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
        // 成功起播即结束本轮自动续播，离线连跳配额也随之归零；
        // 同时作废任何还排队着的失败切歌，否则它会把刚响起来的这首顶掉。
        self.endAutoAdvanceChain();
        playerState.patch({ playing: true, loading: false, muted: state.muted });
        self.startProgressLoop();
        self.updateMediaSession();
        if (autoplay) h.fade(0, state.muted ? 0 : (self.targetHowlVolume() || storedVolume), self.fadeInMs);
        window.dispatchEvent(new CustomEvent("l1:play_state", { detail: { isPlaying: true, track } }));
      },
      onpause() {
        if (!self.isCurrentTrack(track)) return;
        playerState.patch({ playing: false });
        self.stopProgressLoop(Number(h.seek() ?? 0));
        self.updateMediaSession();
        window.dispatchEvent(new CustomEvent("l1:play_state", { detail: { isPlaying: false, reason: "Paused" } }));
      },
      onstop() {
        if (!self.isCurrentTrack(track)) return;
        // 重新起播（stop → play）时 Howler 可能把 stop 事件排在 play 之后派发，
        // 这时音频其实正在响，别把 playing 打回 false 造成按钮与实际不一致。
        if (h.playing()) return;
        playerState.patch({ playing: false, position: 0 });
        self.stopProgressLoop(0);
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
        self.stopProgressLoop(duration);
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
        if (this.transientRetryKeys.size > 100) this.transientRetryKeys.clear();
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
        // 不再往下播，自动续播链在这里结束。
        this.endAutoAdvanceChain();
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
      // 断网时换源和继续往下扫都是白费：换源要发请求，扫列表只会把整张列表标记成不可用。
      if (!this.online) {
        this.haltForOffline();
        return;
      }

      if (track && await this.tryFailoverTrack(track)) return;

      // 换源是个 await，期间用户可能已经点了别的歌。旧曲的失败处理必须到此为止：
      // 否则下面会把它标成 disabled 再排一次切歌，把用户刚点的那首顶掉。
      // 同样比对象引用：换源已经改过 track.id，比 id 会在「新 id 撞上用户刚点那首」时误判。
      // 这里不动 autoAdvanceInFlight —— 真有新一轮续播在跑时，它自己的出口会收尾。
      if (track && this.currentTrack !== track) return;

      if (track) track.disabled = true;
      this.syncState();
      this._failCount++;
      const total = this.playlist.filter((t) => !t.disabled).length;
      const ceiling = Math.min(Listen1Player.MAX_CONSECUTIVE_FAILURES, this.playlist.length);
      if (total === 0 || this._failCount >= ceiling) {
        this._failCount = 0;
        this._isAutoSkipping = false;
        // 自动续播链在这里终止，标志位必须跟着落下。
        this.endAutoAdvanceChain();
        this.manualSkipDirection = null;
        playerState.patch({ loading: false, playing: false });
        if (total > 0) toast.warn("连续多首无法播放，已暂停自动跳曲");
        return;
      }
      // 失败驱动的切歌要留间隔，否则界面会以每首几百毫秒的速度疯狂翻页。
      // 句柄要留住：这段间隔内用户完全有可能自己点一首歌，那时必须能取消。
      // lastFailureSkipAt 记的是「上一次排队的切歌何时触发」，可能还在未来，
      // 此时 elapsed 为负、delay 大于间隔，正是想要的：两次切歌之间恒定间隔。
      const elapsed = Date.now() - this.lastFailureSkipAt;
      const delay = Math.max(0, Listen1Player.FAILURE_SKIP_SPACING_MS - elapsed);
      this.lastFailureSkipAt = Date.now() + delay;
      const manualDirection = this.manualSkipDirection;
      this.clearFailureSkipTimer();
      if (manualDirection) {
        this.failureSkipTimer = setTimeout(() => {
          this.failureSkipTimer = null;
          this.continueManualSkip(manualDirection);
        }, delay);
      } else {
        this._isAutoSkipping = true;
        this.failureSkipTimer = setTimeout(() => {
          this.failureSkipTimer = null;
          void this._autoSkip();
        }, delay);
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
    // 内存卫生：长会话下防止无界增长。
    if (this.failedSourcesByTrackKey.size > 300) this.failedSourcesByTrackKey.clear();
    if (this.failoverAttemptIds.size > 300) this.failoverAttemptIds.clear();
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

    this.saveToStorage();
    // 上面那句 getUrl 是个 await，期间用户完全可能已经点了别的歌。
    // 换好的 URL 留在 track 对象上（下次点它能直接用），但绝不能再改 currentTrack /
    // 起播：那会把界面改回旧曲、currentIndex 却是新的，卡在 loading，
    // 实际响的却是用户刚点的那首。syncState 发布的是当前真相，不是这首旧曲。
    // 这里比对象引用而不是 id：currentTrack 是 playlist[index] 的 getter，同一首歌
    // 就是同一个对象。换源后 track.id 已被改成新源的 id，万一它正好等于用户刚点的
    // 那首的 id，比 id 会误判成「还是当前曲」，把用户刚点的歌从 00:00 重放一遍。
    if (this.currentTrack !== track) {
      this.syncState();
      return false;
    }
    playerState.patch({
      playlist: [...this.playlist],
      currentTrack: track,
      currentIndex: this.index,
      loading: true,
      playing: false,
      position: 0,
    });
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
      this.endAutoAdvanceChain();
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
    // 断网时只有自动续播允许跳到下一首有缓存的歌；用户点歌仍然停下来给提示。
    this.autoAdvanceInFlight = true;
    this._loadInternal(next);
  }

  private continueManualSkip(direction: "next" | "prev" | "random") {
    if (this.playlist.length === 0) return;
    const next = this.nextPlayableIndex(direction);
    if (next < 0) {
      this.manualSkipDirection = null;
      this.endAutoAdvanceChain();
      playerState.patch({ loading: false, playing: false });
      return;
    }
    this._loadInternal(next);
  }

  // Internal load — does NOT reset _failCount (called from auto-skip)
  private _loadInternal(idx: number) {
    const safeidx = this.safeIndex(idx);
    if (safeidx < 0) return;
    // 排队中的失败切歌一律作废：这一次加载（不论用户点的还是续播）才是最新意图。
    this.clearFailureSkipTimer();
    this.clearPauseFadeTimer();
    this.currentHowl?.stop();
    this.index = safeidx;
    const track = this.playlist[this.index];
    if (!track) return;
    this.playedFrom = Date.now();
    playbackClock.update(0, 0);
    playerState.patch({ loading: true, position: 0, currentTrack: track, currentIndex: this.index });
    void this._resolveAndPlay(track, ++this.loadSeq);
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

  /**
   * 稳定缓存键：平台+歌曲ID，让带时效签名的 URL 不再反复 miss / 重复落盘。
   * 本地曲目不参与（其 URL 是 file://，走本地分支不缓存）。
   * 离线探测与写缓存必须用同一个键，否则断网时永远查不到自己写下的文件。
   */
  private cacheIdFor(track?: Track): string {
    if (!track?.id || this.isLocalTrack(track)) return "";
    return `${track.source ?? ""}:${track.id}`;
  }

  /** 断网兜底：若该曲目的音频已落盘，返回可直接播放的 file:// 地址。 */
  private async cachedFileUrl(track: Track): Promise<string | null> {
    if (!isTauriRuntime() || !get(settings).audioCache.enabled) return null;
    const cacheId = this.cacheIdFor(track);
    if (!cacheId) return null;
    const path = await audioCacheLookup(cacheId).catch(() => null);
    if (!path) return null;
    return `file:///${path.replace(/\\/g, "/").replace(/^\/+/, "")}`;
  }

  /** 命中缓存则把 sound_url 指向本地文件；返回是否命中。 */
  private async playFromCacheIfPossible(track: Track): Promise<boolean> {
    const fileUrl = await this.cachedFileUrl(track);
    if (!fileUrl) return false;
    try {
      track.sound_url = await this.proxyUrl(fileUrl);
      track.disabled = false;
      return true;
    } catch {
      return false;
    }
  }

  /**
   * 断网 + 自动续播：跳到下一首预扫出的可播放曲目，而不是逐首失败地扫下去。
   * offlineAdvanceCount 是防御性上限——万一缓存文件在扫描之后被清掉，
   * 探测命中但加载又失败，就会形成新的跳曲循环，这正是本次要消灭的行为。
   */
  private async advanceToCachedOrHalt(failed: Track) {
    // 链路终止点：曲目已被用户换掉，这次自动续播就此结束，标志位必须落地，
    // 否则下一次用户点歌前 autoAdvanceInFlight 会一直挂着 true。
    if (!this.isCurrentTrack(failed)) {
      this.endAutoAdvanceChain();
      return;
    }
    if (this.offlineAdvanceCount >= Listen1Player.MAX_OFFLINE_ADVANCES) {
      this.haltForOffline();
      return;
    }
    const next = await this.offlinePlayableIndex(this.index, "next");
    if (next < 0 || next === this.index || !this.isCurrentTrack(failed)) {
      this.haltForOffline();
      return;
    }
    this.offlineAdvanceCount++;
    this._loadInternal(next);
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
    const cacheId = this.cacheIdFor(track);
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

    // 断网：先看缓存，命中就播本地文件；miss 则停在这里。
    // 关键是不要清掉已有 URL、不要 disabled、不要触发失败链——否则会一路扫下去。
    if (!this.online && !this.isLocalTrack(track)) {
      if (await this.playFromCacheIfPossible(track)) return true;
      if (markLoading) {
        // 自动续播时跳到下一首有缓存的歌；用户主动点的这首没缓存就停下来提示，
        // 不要偷偷换成另一首歌。
        if (this.autoAdvanceInFlight) await this.advanceToCachedOrHalt(track);
        else this.haltForOffline();
      }
      return false;
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
          // 取地址失败可能只是断网（navigator.onLine 未必及时翻转），先试缓存。
          if (await this.playFromCacheIfPossible(track)) return true;
          track.disabled = true;
          if (markLoading) {
            this.syncState();
            void this._onTrackFailed(track);
          }
          return false;
        }
      } catch {
        if (await this.playFromCacheIfPossible(track)) return true;
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
      this.preloadBackoffUntil = Date.now() + 5000;
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

  private async _resolveAndPlay(track: Track, seq: number = this.loadSeq) {
    const resolved = await this.resolveTrackUrl(track, true);
    if (!this.isCurrentTrack(track)) {
      // 曲目已被换掉。只有在没有更新的加载接棒时才收尾，
      // 否则会把新一轮自动续播刚设上的标志位清掉。
      if (seq === this.loadSeq) this.endAutoAdvanceChain();
      return;
    }
    if (!resolved) return;

    let h = this.howls.get(track.id);
    if (!h) {
      h = this.buildHowl(track, false);
      this.howls.set(track.id, h);
    } else {
      // 预载好但没播过的 Howl 本来就停着，再 stop 一次只会白白派发一次 onstop。
      if (h.playing()) h.stop();
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
    // 用户主动播放：结束任何还挂着的自动续播链，并作废排队中的失败切歌。
    // 否则断网时这里会走进 resolveTrackUrl 的自动分支，把用户点的歌悄悄换成另一首。
    this.endAutoAdvanceChain();
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
      this.stopProgressLoop(Number(h.seek() ?? 0));
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
    this.stopProgressLoop(Number(h.seek() ?? 0));
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
    this.endAutoAdvanceChain();
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
    this.endAutoAdvanceChain();
    this.failoverAttemptIds.clear();
    this.failedSourcesByTrackKey.clear();
    this.handlingFailedTrackIds.clear();
    this.failureConfirmTimers.forEach((timer) => clearTimeout(timer));
    this.failureConfirmTimers.clear();
    this.transientRetryKeys.clear();
    // 断网时不按普通列表顺序跳，直接落到预扫出的可播放集合上。
    if (!this.online) {
      void this.skipOffline(direction);
      return;
    }
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
    if (!h) return;
    const duration = Number(h.duration() || 0);
    const target = (percent / 100) * duration;
    h.seek(target);
    playbackClock.update(Number.isFinite(target) ? target : 0, duration);
  }

  seekRelative(seconds: number) {
    const h = this.currentHowl;
    if (!h) return;
    const duration = Number(h.duration() || 0);
    if (!Number.isFinite(duration) || duration <= 0) return;
    const current = Number(h.seek() || 0);
    const next = Math.max(0, Math.min(duration, current + seconds));
    h.seek(next);
    playbackClock.update(next, duration);
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
    this.endAutoAdvanceChain();
    this.failoverAttemptIds.clear();
    this.failedSourcesByTrackKey.clear();
    this.handlingFailedTrackIds.clear();
    this.failureConfirmTimers.forEach((timer) => clearTimeout(timer));
    this.failureConfirmTimers.clear();
    this.transientRetryKeys.clear();
    this.playlist = tracks.map((t) => this.normalizeTrackForQueue(t));
    this.index = -1;
    // 列表整体换掉，旧的可播放集合失效；断网时立刻重扫，保证切歌仍是查表而非试错。
    this.offlinePlayableIds = null;
    this.offlineScanPromise = null;
    if (!this.online) void this.ensureOfflinePlayableScan();
    playerState.patch({ playlist: [...this.playlist], currentIndex: -1, currentTrack: null });
    this.saveToStorage();
  }

  appendTracks(tracks: Track[]) {
    const existing = new Set(this.playlist.map((t) => t.id));
    const added = tracks.filter((t) => !existing.has(t.id)).map((t) => this.normalizeTrackForQueue(t));
    this.playlist.push(...added);
    void this.patchOfflinePlayable(added);
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
    void this.patchOfflinePlayable([queuedTrack]);
    playerState.patch({ playlist: [...this.playlist] });
    this.saveToStorage();
  }

  removeTrack(index: number) {
    const removed = this.playlist[index];
    if (!removed) return;
    const wasCurrent = index === this.index;
    const wasPlaying = wasCurrent ? Boolean(this.currentHowl?.playing()) : false;
    // 删歌是用户操作：即使下面会走 _loadInternal 接续播放，也不算自动续播，
    // 否则断网时这一步会被当成续播链，悄悄跳到另一首有缓存的歌。
    this.endAutoAdvanceChain();
    this._isAutoSkipping = false;
    this.clearFailureConfirm(removed.id);
    if (wasCurrent) {
      this.clearPauseFadeTimer();
      this.currentHowl?.stop();
      this.stopProgressLoop();
    }
    // 被删曲目的 Howl 不再有任何引用，顺手释放，避免解码缓冲留在内存里。
    if (removed.id && !this.playlist.some((t, i) => i !== index && t?.id === removed.id)) {
      this.howls.get(removed.id)?.unload();
      this.howls.delete(removed.id);
      if (this.preloadTrackId === removed.id) this.preloadTrackId = null;
    }
    if (index < this.index) this.index--;
    this.playlist.splice(index, 1);

    if (this.playlist.length === 0) {
      this.index = -1;
      playbackClock.update(0, 0);
      playerState.patch({ playlist: [], currentIndex: -1, currentTrack: null, playing: false, loading: false, duration: 0, position: 0 });
      this.saveToStorage();
      return;
    }

    if (wasCurrent) {
      // 删掉的是正在播的那首：让滑进这个位置的曲目接棒，
      // 原本在播就继续播，原本暂停就只把指针挪过去（play() 能自行解析未加载的曲目）。
      this.index = this.safeIndex(this.index);
      if (wasPlaying) {
        this._loadInternal(this.index);
        return;
      }
      playbackClock.update(0, 0);
      playerState.patch({ position: 0, duration: 0 });
    }
    this.syncState();
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
    this.endAutoAdvanceChain();
    // 列表清空，可播放集合一并作废（空集合和「没扫过」在语义上都是没得播）。
    this.offlinePlayableIds = null;
    this.offlineScanPromise = null;
    playerState.patch({ playlist: [], currentIndex: -1, currentTrack: null, playing: false });
    this.saveToStorage();
  }

  getPlayedFrom(): number { return this.playedFrom; }
  getTrackById(id: string): Track | undefined { return this.playlist.find((t) => t.id === id); }

  private saveStorageTimer: ReturnType<typeof setTimeout> | null = null;

  /** 持久化播放队列（防抖合并，千首级队列不再每次操作全量 stringify 卡顿主线程）。 */
  private saveToStorage(immediate = false) {
    if (immediate) {
      if (this.saveStorageTimer) {
        clearTimeout(this.saveStorageTimer);
        this.saveStorageTimer = null;
      }
      this.writeQueueToStorage();
      return;
    }
    if (this.saveStorageTimer) return;
    this.saveStorageTimer = setTimeout(() => {
      this.saveStorageTimer = null;
      this.writeQueueToStorage();
    }, 400);
  }

  private writeQueueToStorage() {
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
