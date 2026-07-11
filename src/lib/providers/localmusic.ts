import { readAudioTags, scanMusicDirectory } from "../tauri";
import { open } from "@tauri-apps/plugin-dialog";
import type { SearchResult, Playlist, PlaylistInfo, LyricResult, UrlResult, PlaylistFilter } from "./types";
import type { Track } from "../stores/player";
import { getParameterByName } from "./utils";

const LOCAL_PLAYLIST_ID = "lmplaylist_reserve";
const LOCAL_META_VERSION = 2;
type LocalTrack = Track & { meta_scanned?: boolean; meta_version?: number };

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
  localStorage.setItem(key, JSON.stringify(value));
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
      img_url: meta.cover ?? "",
      lyric: meta.lyrics ?? "",
      duration: meta.duration,
      bitrate: typeof meta.bitrate === "number" ? `${Math.round(meta.bitrate)}kbps` : undefined,
    };
  } catch (err) {
    console.warn("[localmusic] failed to read audio tags", filePath, err);
    return baseTrack;
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
    const idx = existingIndex.get(track.id);
    if (idx == null) {
      existingIndex.set(track.id, existing.tracks.length);
      existing.tracks.push(track);
      added++;
    } else {
      existing.tracks[idx] = {
        ...existing.tracks[idx],
        ...track,
        disabled: false,
      };
      updated++;
    }
  }
  updatePlaylistCover(existing);
  lsSet(LOCAL_PLAYLIST_ID, existing);
  return { added, updated, total: existing.tracks.length };
}

async function refreshStoredTrack(track: Track): Promise<LocalTrack> {
  const localTrack = track as LocalTrack;
  if (localTrack.meta_scanned && localTrack.meta_version === LOCAL_META_VERSION) {
    return { ...localTrack, disabled: false };
  }

  const filePath = localPathFromTrack(track);
  if (!filePath) {
    return {
      ...localTrack,
      disabled: false,
      meta_scanned: true,
      meta_version: LOCAL_META_VERSION,
    };
  }

  const refreshed = await trackFromPath(filePath);
  return {
    ...localTrack,
    ...refreshed,
    disabled: false,
    sound_url: undefined,
  };
}

function updatePlaylistCover(pl: Playlist) {
  if (!pl.info.cover_img_url) {
    pl.info.cover_img_url = pl.tracks.find((track) => track.img_url)?.img_url ?? "";
  }
}

function updateStoredLocalTrack(trackId: string, refreshed: Track) {
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
      const refreshed = await refreshStoredTrack(track);
      refreshedTracks.push(refreshed);
      if (refreshed !== track) changed = true;
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

  async lyric(url: string): Promise<LyricResult> {
    const trackId = getParameterByName("track_id", url);
    const playlist = lsGet<Playlist>(LOCAL_PLAYLIST_ID);
    let track = playlist?.tracks.find((item) => item.id === trackId);

    if (!track) {
      try {
        const queue = JSON.parse(localStorage.getItem("current-playing") ?? "[]") as Track[];
        track = queue.find((item) => item.id === trackId);
      } catch {}
    }

    if (!track) return { lyric: "", tlyric: "" };

    const refreshed = await refreshStoredTrack(track);
    updateStoredLocalTrack(trackId, refreshed);
    return { lyric: refreshed.lyric ?? "", tlyric: "" };
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
