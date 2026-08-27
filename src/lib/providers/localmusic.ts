import { readAudioTags, scanMusicDirectory, type AudioMeta } from "../tauri";
import { open } from "@tauri-apps/plugin-dialog";
import type { SearchResult, Playlist, PlaylistInfo, LyricResult, UrlResult, PlaylistFilter } from "./types";
import type { Track } from "../stores/player";
import { getParameterByName } from "./utils";

const LOCAL_PLAYLIST_ID = "lmplaylist_reserve";
/**
 * 3 = 内嵌封面改成落盘后回 file:// 路径（2 及以前把 data URL 剥成空串又不再重读，
 * 导致封面永久丢失），并且只有真的读到标签才算扫描完成。老条目靠这个版本号重扫。
 */
const LOCAL_META_VERSION = 3;
type LocalTrack = Track & {
  meta_scanned?: boolean;
  meta_version?: number;
  /** 联网补齐元数据的时间戳。有值表示试过了，避免每次播放都再搜一遍。 */
  online_meta_at?: number;
  /**
   * 歌词是联网补来的而不是从文件读的。stripHeavyFields 靠它豁免：内嵌歌词随时能
   * 重读文件，联网这份不行（online_meta_at 一写上就不会再联网）。
   */
  lyric_from_online?: boolean;
};

function fileUriFromPath(filePath: string): string {
  const normalized = filePath.replace(/\\/g, "/");
  if (/^file:\/\/[A-Za-z]:\//.test(normalized)) {
    return normalized.replace(/^file:\/\//, "file:///");
  }
  if (normalized.startsWith("file://")) return normalized;
  if (/^[A-Za-z]:\//.test(normalized)) return `file:///${normalized}`;
  if (normalized.startsWith("/")) return `file://${normalized}`;
  return `file:///${normalized}`;
}

function lsGet<T>(key: string): T | null {
  try {
    const v = localStorage.getItem(key);
    return v ? JSON.parse(v) : null;
  } catch {
    return null;
  }
}

function lsSet(key: string, value: unknown) {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch (err) {
    const name = (err as { name?: string } | null)?.name ?? "";
    if (name === "QuotaExceededError" || name === "NS_ERROR_DOM_QUOTA_REACHED") {
      throw new Error("本地存储空间已满，请减少扫描的曲目数量或清理已导入的本地音乐后重试。");
    }
    throw err;
  }
}

// Base64-encoded cover art can easily exceed localStorage quotas when many
// tracks are scanned. 内嵌封面现在由 Rust 落盘、这里只存 file:// 路径，所以正常
// 情况下不会再出现 data URL；这条兜底只对 sidecar 里自带 data URL 的老条目生效。
function stripHeavyFields<T extends LocalTrack>(track: T): T {
  const cover = track.img_url ?? "";
  const lyric = track.lyric ?? "";
  const next: T = { ...track };
  if (cover.startsWith("data:")) next.img_url = "";
  // Embedded lyrics can also be sizeable; drop them from persistent storage
  // and rely on on-demand reads via `lyric()` when the user plays the track.
  // 但联网补来的那份不能剥：文件里本来就没有歌词，online_meta_at 又已经写上了不会
  // 再联网，剥掉就是永久丢失。
  if (lyric.length > 2000 && !track.lyric_from_online) next.lyric = "";
  return next;
}

function filenameFromPath(filePath: string): string {
  return filePath.split(/[/\\]/).pop() ?? "";
}

function localPathFromTrack(track: Track): string {
  if (track.id.startsWith("lmtrack_")) return track.id.slice("lmtrack_".length);
  const url = track.sound_url ?? track.url ?? "";
  if (!url.startsWith("file://")) return "";
  try {
    const raw = url.replace(/^file:\/\/\/?/, "");
    return decodeURIComponent(raw);
  } catch {
    return "";
  }
}

/** 内嵌封面：Rust 已经把它落盘，这里只把路径转成 file://，渲染时再经流服务器取。 */
function coverFromMeta(meta: AudioMeta): string {
  if (meta.cover_path) return fileUriFromPath(meta.cover_path);
  return meta.cover ?? "";
}

async function trackFromPath(filePath: string): Promise<LocalTrack> {
  const fallbackTitle = filenameFromPath(filePath);
  const baseTrack: LocalTrack = {
    id: `lmtrack_${filePath}`,
    title: fallbackTitle,
    artist: "",
    artist_id: "",
    album: "",
    album_id: "",
    source: "localmusic",
    source_url: "",
    img_url: "",
    url: fileUriFromPath(filePath),
    sound_url: undefined,
    platform: "local",
    meta_scanned: true,
    meta_version: LOCAL_META_VERSION,
  };

  try {
    const meta = await readAudioTags(filePath);
    return {
      ...baseTrack,
      title: meta.title ?? fallbackTitle,
      artist: meta.artist ?? "",
      artist_id: `lmartist_${meta.artist ?? ""}`,
      album: meta.album ?? "",
      album_id: `lmalbum_${meta.album ?? ""}`,
      img_url: coverFromMeta(meta),
      lyric: meta.lyrics ?? "",
      duration: meta.duration,
      bitrate: typeof meta.bitrate === "number" ? `${Math.round(meta.bitrate)}kbps` : undefined,
      // 标签没读出来就不算扫描完成，否则这首歌会被永久记成「没有信息」，
      // 既不会重读文件也不会走联网兜底。
      meta_scanned: meta.tags_read !== false,
    };
  } catch (err) {
    console.warn("[localmusic] failed to read audio tags", filePath, err);
    return { ...baseTrack, meta_scanned: false };
  }
}

async function persistLocalTracks(tracks: Track[]) {
  const existing = lsGet<Playlist>(LOCAL_PLAYLIST_ID) ?? {
    info: { id: LOCAL_PLAYLIST_ID, title: "本地音乐" },
    tracks: [],
  };
  const existingIndex = new Map(existing.tracks.map((t, idx) => [t.id, idx]));
  let added = 0;
  let updated = 0;
  for (const track of tracks) {
    const slim = stripHeavyFields(track as LocalTrack);
    const idx = existingIndex.get(slim.id);
    if (idx == null) {
      existingIndex.set(slim.id, existing.tracks.length);
      existing.tracks.push(slim);
      added++;
    } else {
      // 走 mergeMeta 而不是直接展开：重扫时若某字段读空了，不能把之前（可能是联网
      // 补齐来的）封面/歌手抹掉。
      existing.tracks[idx] = {
        ...mergeMeta(existing.tracks[idx] as LocalTrack, slim),
        disabled: false,
      };
      updated++;
    }
  }
  updatePlaylistCover(existing);
  lsSet(LOCAL_PLAYLIST_ID, existing);
  return { added, updated, total: existing.tracks.length };
}

/** 只用非空值覆盖，避免重扫时把之前（尤其是联网补齐）拿到的信息抹成空串。 */
function mergeMeta(base: LocalTrack, next: LocalTrack): LocalTrack {
  const merged: LocalTrack = { ...base, ...next };
  const keep = ["title", "artist", "album", "img_url", "lyric"] as const;
  for (const key of keep) {
    if (!next[key] && base[key]) merged[key] = base[key];
  }
  if (next.duration == null && base.duration != null) merged.duration = base.duration;
  if (!next.bitrate && base.bitrate) merged.bitrate = base.bitrate;
  // 歌词来源标记要跟着歌词走：换成文件里读出来的就清掉，否则这条会一直被当成联网
  // 歌词豁免落盘，白占配额。
  if (next.lyric) merged.lyric_from_online = next.lyric_from_online ?? false;
  else if (base.lyric) merged.lyric_from_online = base.lyric_from_online;
  return merged;
}

/**
 * 返回 changed=false 表示这条不需要落盘。以前这里恒返回新对象，调用方拿 `!==`
 * 判脏，于是每打开一次本地歌单就把整个歌单重写一遍 localStorage。
 */
async function refreshStoredTrack(track: Track): Promise<{ track: LocalTrack; changed: boolean }> {
  const localTrack = track as LocalTrack;
  if (localTrack.meta_scanned && localTrack.meta_version === LOCAL_META_VERSION) {
    return { track: { ...localTrack, disabled: false }, changed: localTrack.disabled === true };
  }

  const filePath = localPathFromTrack(track);
  if (!filePath) {
    return {
      track: {
        ...localTrack,
        disabled: false,
        meta_scanned: true,
        meta_version: LOCAL_META_VERSION,
      },
      changed: true,
    };
  }

  const refreshed = await trackFromPath(filePath);
  return {
    track: {
      ...mergeMeta(localTrack, refreshed),
      disabled: false,
      sound_url: undefined,
    },
    changed: true,
  };
}

function updatePlaylistCover(pl: Playlist) {
  if (!pl.info.cover_img_url) {
    pl.info.cover_img_url = pl.tracks.find((track) => track.img_url)?.img_url ?? "";
  }
}

function updateStoredLocalTrack(trackId: string, refreshed: Partial<Track>) {
  const playlist = lsGet<Playlist>(LOCAL_PLAYLIST_ID);
  if (playlist) {
    const idx = playlist.tracks.findIndex((item) => item.id === trackId);
    if (idx >= 0) {
      playlist.tracks[idx] = { ...playlist.tracks[idx], ...refreshed };
      updatePlaylistCover(playlist);
      lsSet(LOCAL_PLAYLIST_ID, playlist);
    }
  }

  const queue = lsGet<Track[]>("current-playing");
  if (queue) {
    const idx = queue.findIndex((item) => item.id === trackId);
    if (idx >= 0) {
      queue[idx] = { ...queue[idx], ...refreshed };
      lsSet("current-playing", queue);
    }
  }

  // 播放器和各视图持的是内存里的 Track 副本，改完存储还得广播一次，
  // 否则补到的封面要等下次重新进歌单才看得到。
  if (typeof window !== "undefined") {
    window.dispatchEvent(
      new CustomEvent("listen1-local-meta-updated", { detail: { trackId, patch: refreshed } })
    );
  }
}

/** 从本地歌单或当前播放队列里找这首歌。歌词/补全都要先拿到它的文件路径。 */
function findStoredLocalTrack(trackId: string): LocalTrack | null {
  const playlist = lsGet<Playlist>(LOCAL_PLAYLIST_ID);
  const fromPlaylist = playlist?.tracks.find((item) => item.id === trackId);
  if (fromPlaylist) return fromPlaylist as LocalTrack;
  const queue = lsGet<Track[]>("current-playing");
  return (queue?.find((item) => item.id === trackId) as LocalTrack | undefined) ?? null;
}

/** 文件里读到、但存储里还缺的字段。用于播放时顺手把歌单条目补齐。 */
function metaPatchFromFile(track: LocalTrack, meta: AudioMeta): Partial<LocalTrack> {
  const patch: Partial<LocalTrack> = {};
  const cover = coverFromMeta(meta);
  if (cover && cover !== track.img_url) patch.img_url = cover;
  if (meta.artist && !track.artist) {
    patch.artist = meta.artist;
    patch.artist_id = `lmartist_${meta.artist}`;
  }
  if (meta.album && !track.album) {
    patch.album = meta.album;
    patch.album_id = `lmalbum_${meta.album}`;
  }
  return patch;
}

function updateStoredQueueTracks(tracks: Track[]) {
  const refreshedById = new Map(tracks.map((track) => [track.id, track]));
  const queue = lsGet<Track[]>("current-playing");
  if (!queue) return;

  let changed = false;
  const nextQueue = queue.map((track) => {
    const refreshed = refreshedById.get(track.id);
    if (!refreshed) return track;
    changed = true;
    return { ...track, ...refreshed };
  });
  if (changed) lsSet("current-playing", nextQueue);
}

export const localmusic = {
  async search(url: string): Promise<SearchResult> {
    const keywords = decodeURIComponent(getParameterByName("keywords", url) ?? "").trim().toLowerCase();
    const playlist = await this.get_playlist();
    const result = keywords
      ? playlist.tracks.filter((track) =>
          [track.title, track.artist, track.album]
            .filter(Boolean)
            .some((text) => text!.toLowerCase().includes(keywords))
        )
      : playlist.tracks;
    return { result, total: result.length, type: "song" };
  },

  async show_playlist(): Promise<{ result: PlaylistInfo[] }> {
    return { result: [] };
  },

  async get_playlist(): Promise<Playlist> {
    const pl = lsGet<Playlist>(LOCAL_PLAYLIST_ID);
    if (!pl) return { info: { id: LOCAL_PLAYLIST_ID, title: "本地音乐" }, tracks: [] };
    let changed = false;
    const refreshedTracks: Track[] = [];
    for (const track of pl.tracks) {
      const result = await refreshStoredTrack(track);
      // 重读出来的内嵌歌词必须剥掉再落盘。trackFromPath 带回的是完整歌词（一首几十
      // KB），这条路径以前不经 stripHeavyFields，上千首本地歌直接把 localStorage
      // 配额撑爆，lsSet 抛错后整个本地歌单都加载不出来。联网补来的那份带
      // lyric_from_online，被 stripHeavyFields 豁免，不会被这一刀带走。
      refreshedTracks.push(stripHeavyFields(result.track));
      if (result.changed) changed = true;
    }
    if (changed) {
      pl.tracks = refreshedTracks;
      updatePlaylistCover(pl);
      lsSet(LOCAL_PLAYLIST_ID, pl);
      updateStoredQueueTracks(refreshedTracks);
    }
    const fallbackCover = pl.tracks.find((track) => track.img_url)?.img_url ?? "";
    return {
      ...pl,
      info: { ...pl.info, cover_img_url: pl.info.cover_img_url || fallbackCover },
    };
  },

  async get_song_url(trackId: string): Promise<UrlResult | null> {
    const path = trackId.replace("lmtrack_", "");
    return { url: fileUriFromPath(path), platform: "localmusic" };
  },

  /**
   * 歌词按需从文件重读，不从 localStorage 取。
   *
   * 歌词动辄几十 KB，上千首本地歌全存进 localStorage 会直接把配额撑爆，所以
   * 持久化时被剥掉了；只在真正要显示时读一次文件。顺手把封面/歌手也补进歌单。
   * 文件里没有歌词时返回空串，联网兜底由 MediaService.getLyric 负责。
   */
  async lyric(url: string): Promise<LyricResult> {
    const trackId = getParameterByName("track_id", url);
    if (!trackId) return { lyric: "", tlyric: "" };
    const track = findStoredLocalTrack(trackId);
    if (!track) return { lyric: "", tlyric: "" };

    const filePath = localPathFromTrack(track);
    if (!filePath) return { lyric: track.lyric ?? "", tlyric: "" };

    const meta = await readAudioTags(filePath).catch((error) => {
      console.warn("[localmusic] failed to read lyrics", filePath, error);
      return null;
    });
    if (!meta) return { lyric: track.lyric ?? "", tlyric: "" };

    const patch = metaPatchFromFile(track, meta);
    if (Object.keys(patch).length > 0) updateStoredLocalTrack(trackId, patch);
    return { lyric: meta.lyrics ?? track.lyric ?? "", tlyric: "" };
  },

  /** 供 MediaService 的联网兜底用：拿到这首本地歌当前的存储状态。 */
  getStoredTrack(trackId: string): Track | null {
    return findStoredLocalTrack(trackId);
  },

  /** 供 MediaService 的联网兜底用：把补到的信息写回歌单/队列并广播。 */
  applyMetaPatch(trackId: string, patch: Partial<Track>) {
    updateStoredLocalTrack(trackId, patch);
  },

  async parse_url(): Promise<PlaylistInfo | null> {
    return null;
  },

  get_playlist_filters(): PlaylistFilter[] {
    return [];
  },

  async openFilePicker(): Promise<Track[]> {
    const files = await open({
      multiple: true,
      filters: [{ name: "Music", extensions: ["mp3", "flac", "ogg", "oga", "opus", "wav", "aif", "aiff", "m4a", "mp4", "aac", "webm"] }],
    });
    if (!files) return [];

    const paths = Array.isArray(files) ? files : [files];
    const tracks: Track[] = [];

    for (const filePath of paths) {
      tracks.push(await trackFromPath(filePath));
    }

    await persistLocalTracks(tracks);

    return tracks;
  },

  async scanDirectory(directory: string): Promise<{ tracks: Track[]; added: number; updated: number; total: number }> {
    const paths = await scanMusicDirectory(directory);
    const tracks: Track[] = [];
    for (const filePath of paths) {
      tracks.push(await trackFromPath(filePath));
    }
    const stats = await persistLocalTracks(tracks);
    return { tracks, ...stats };
  },

  async refreshDownloadedTrack(filePath: string): Promise<{ track: Track; added: number; updated: number; total: number }> {
    const track = await trackFromPath(filePath);
    const stats = await persistLocalTracks([track]);
    return { track, ...stats };
  },
};
