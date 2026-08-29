/**
 * 轻量模式的交接层。
 *
 * 轻量模式下 WebView 会被真正销毁，provider 解析、歌词抓取、封面代理这些只存在于
 * 前端的能力全都没了。所以切换之前必须把原生播放器需要的一切准备好并落盘：
 *
 * - 队列元信息、当前下标、进度、音量、循环模式；
 * - 每首歌的 LRC 原文（歌词是每次播放重新抓的，磁盘上没有任何缓存）；
 * - 封面本地文件（`img_url` 多是远端地址，原生窗口没法自己去代理）；
 * - 当前及后续若干首的音频预缓存（直链带时效签名，过期后原生侧无从续命）。
 *
 * 音频本身不必内嵌：Rust 侧用 `source:id` 就能在磁盘缓存里查到完整可播文件，
 * 这条路不经过前端。
 */

import { get } from "svelte/store";
import { MediaService } from "./providers/index";
import { player } from "./player";
import { playbackClock, playerState, type LoopMode, type Track } from "./stores/player";
import { miniFetchCover, miniPrecacheAudio, miniEnter, miniLoadSnapshot } from "./tauri";

/** 预准备的曲目数：当前这首加上后面几首。再多就是白等，签名照样会过期。 */
export const PREPARE_AHEAD = 5;

/** 与 Rust `MiniTrack` 一一对应（serde 那边是 camelCase）。 */
export interface LiteTrack {
  id: string;
  title: string;
  artist: string;
  album: string;
  source: string;
  duration: number;
  url: string | null;
  localPath: string | null;
  coverPath: string | null;
  lyric: string | null;
  tlyric: string | null;
}

/** 与 Rust `MiniSnapshot` 一一对应。 */
export interface LiteSnapshot {
  version: number;
  savedAt: number;
  index: number;
  position: number;
  volume: number;
  muted: boolean;
  loopMode: number;
  tracks: LiteTrack[];
}

export const LITE_SNAPSHOT_VERSION = 1;

/** 本地曲目：URL 是 file://，不参与远端解析与音频缓存。 */
function localPathOf(track: Track): string | null {
  const candidate = track.url ?? track.sound_url ?? "";
  if (!candidate.startsWith("file://")) return null;
  try {
    return decodeURIComponent(candidate.replace(/^file:\/+/, ""));
  } catch {
    return candidate.replace(/^file:\/+/, "");
  }
}

function baseTrack(track: Track): LiteTrack {
  return {
    id: track.id,
    title: track.title ?? "",
    artist: track.artist ?? "",
    album: track.album ?? "",
    source: track.source ?? "",
    duration: typeof track.duration === "number" && track.duration > 0 ? track.duration : 0,
    url: null,
    localPath: localPathOf(track),
    coverPath: null,
    lyric: track.lyric ?? null,
    tlyric: null,
  };
}

/**
 * 把一首歌准备到"原生侧能独立播放"的程度：解析直链、抓歌词、下封面、预缓存音频。
 *
 * 每一步都是尽力而为——某一步失败不该让整次切换失败，原生侧对缺失字段都有兜底
 * （没直链就查缓存，没歌词就显示暂无，没封面就画占位）。
 */
async function prepare(track: Track, precache: boolean): Promise<LiteTrack> {
  const entry = baseTrack(track);

  if (!entry.localPath) {
    const resolved = await MediaService.getUrl(track.id, track).catch(() => null);
    if (resolved?.url) entry.url = resolved.url;
  }

  if (!entry.lyric) {
    const lyric = await MediaService.getLyric(
      track.id,
      track.album_id ?? "",
      track.lyric_url,
      track.tlyric_url,
    ).catch(() => null);
    if (lyric?.lyric) entry.lyric = lyric.lyric;
    if (lyric?.tlyric) entry.tlyric = lyric.tlyric;
  }

  if (track.img_url) {
    entry.coverPath = await miniFetchCover(track.img_url).catch(() => null);
  }

  // 预缓存要放在最后：它最慢，而且失败了也只是回到"靠直链播"的老路。
  if (precache && entry.url && !entry.localPath) {
    const cached = await miniPrecacheAudio(`${entry.source}:${entry.id}`, entry.url).catch(
      () => null,
    );
    if (cached) entry.localPath = cached;
  }

  return entry;
}

/**
 * 组装快照。当前这首及其后 `PREPARE_AHEAD - 1` 首会完整准备（含预缓存），
 * 更靠后的只带元信息——真播到那儿时缓存里若没有，原生侧会提示回完整模式。
 */
export async function buildLiteSnapshot(): Promise<LiteSnapshot> {
  const state = get(playerState);
  const clock = get(playbackClock);
  const index = Math.max(0, state.currentIndex);
  const tracks = state.playlist ?? [];

  const prepared: LiteTrack[] = [];
  for (let i = 0; i < tracks.length; i += 1) {
    const withinWindow = i >= index && i < index + PREPARE_AHEAD;
    prepared.push(withinWindow ? await prepare(tracks[i], true) : baseTrack(tracks[i]));
  }

  // 进度必须读 playbackClock。playerState.position 只在换歌时被 patch 成 0，播放期间的
  // 高频位置为了避免全应用订阅者每 tick 重算，是单独写进 playbackClock 的——读 playerState
  // 永远拿到 0，切过去就从头开始放。
  const position = Number.isFinite(clock.position) && clock.position > 0
    ? clock.position
    : (Number.isFinite(state.position) ? state.position : 0);

  return {
    version: LITE_SNAPSHOT_VERSION,
    savedAt: Math.floor(Date.now() / 1000),
    index,
    position,
    // 前端音量是 0~100，原生侧和系统播放器都用 0~1。
    volume: Math.min(1, Math.max(0, (state.volume ?? 90) / 100)),
    muted: Boolean(state.muted),
    loopMode: state.loopMode ?? 0,
    tracks: prepared,
  };
}

/**
 * 切换到轻量模式：先把快照准备好并交给 Rust，Rust 建好原生窗口之后才销毁 WebView。
 * 顺序不能反——先销毁窗口的话，万一原生窗口建不起来，用户就只剩一个托盘图标了。
 */
export async function enterLiteMode(): Promise<void> {
  const snapshot = await buildLiteSnapshot();
  await miniEnter(snapshot);
}

/** 等 Howl 报出时长再按比例定位——加载是异步的，早了 seek 会被丢掉。 */
async function seekWhenReady(position: number): Promise<void> {
  for (let i = 0; i < 80; i += 1) {
    const duration = get(playbackClock).duration;
    if (duration > 0) {
      // 已经播到尾巴上就别倒回去了，让它自然进下一首。
      if (position < duration - 1) player.seek((position / duration) * 100);
      return;
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
}

/**
 * 从轻量模式回到完整模式后接着播。
 *
 * WebView 是重新建出来的，内存里什么都没剩下：队列靠 localStorage 恢复，
 * 但"原生窗口播到了第几首、第几秒"只在快照里。快照取一次就被删掉，
 * 所以正常冷启动不会误触发。
 *
 * 队列按 id 对齐而不是整体替换：快照里的曲目元信息是精简过的（没有 img_url、
 * 没有各种 *_url），拿它覆盖 localStorage 里那份完整队列是净亏。对不上就什么都不做。
 */
export async function resumeFromLiteSnapshot(): Promise<boolean> {
  const snapshot = await miniLoadSnapshot().catch(() => null);
  if (!snapshot || snapshot.tracks.length === 0) return false;

  const index = Math.min(Math.max(0, snapshot.index), snapshot.tracks.length - 1);
  const current = snapshot.tracks[index];
  if (!current) return false;

  // 音量、静音、循环模式在原生窗口里是可以改的，这几项与队列无关，先无条件对齐。
  player.setVolume(Math.round(snapshot.volume * 100));
  if (Boolean(snapshot.muted) !== get(playerState).muted) player.toggleMute();
  player.loopMode = (snapshot.loopMode ?? 0) as LoopMode;

  const queue = get(playerState).playlist ?? [];
  const target = queue.findIndex((track) => track.id === current.id);
  if (target < 0) return false;

  player.loadByIndex(target);
  if (snapshot.position > 1) await seekWhenReady(snapshot.position);
  return true;
}
