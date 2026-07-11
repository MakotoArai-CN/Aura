import { netease } from "./netease";
import { qq } from "./qq";
import { kugou } from "./kugou";
import { kuwo } from "./kuwo";
import { bilibili } from "./bilibili";
import { migu } from "./migu";
import { taihe } from "./taihe";
import { localmusic } from "./localmusic";
import { clearLoginCookies } from "../tauri";
import { normalizeLyricText } from "../lyrics";
import type { SearchResult, Playlist, PlaylistInfo, LyricResult, UrlResult, PlaylistFilter, LoginProvider, UserStatus, UserPlaylistResult } from "./types";
import type { Track } from "../stores/player";

export type ProviderName = "netease" | "qq" | "kugou" | "kuwo" | "bilibili" | "migu" | "taihe" | "localmusic";

const PROVIDERS: Record<ProviderName, {
  search(url: string): Promise<SearchResult>;
  show_playlist(url: string): Promise<{ result: PlaylistInfo[] }>;
  get_playlist(url: string): Promise<Playlist>;
  get_playlist_full?: (url: string) => Promise<Playlist>;
  get_playlist_filters(): PlaylistFilter[];
  lyric(url: string): Promise<LyricResult>;
  get_song_url(trackId: string, trackHint?: Track): Promise<UrlResult | null>;
  parse_url(url: string): Promise<PlaylistInfo | null>;
  get_user?: () => Promise<UserStatus>;
  get_user_created_playlist?: (url: string) => Promise<UserPlaylistResult>;
  get_user_favorite_playlist?: (url: string) => Promise<UserPlaylistResult>;
}> = { netease, qq, kugou, kuwo, bilibili, migu, taihe, localmusic };

const TRACK_PREFIX_MAP: Record<string, ProviderName> = {
  netrack: "netease", neplaylist: "netease", nealbum: "netease", neartist: "netease",
  qqtrack: "qq", qqplaylist: "qq", qqalbum: "qq", qqartist: "qq", qqtoplist: "qq",
  kgtrack: "kugou", kgplaylist: "kugou", kgalbum: "kugou", kgartist: "kugou",
  kwtrack: "kuwo", kwplaylist: "kuwo", kwalbum: "kuwo", kwartist: "kuwo", kwtoplist: "kuwo",
  bitrack: "bilibili", biplaylist: "bilibili", bifav: "bilibili", biartist: "bilibili", bialbum: "bilibili",
  mgtrack: "migu", mgplaylist: "migu", mgalbum: "migu", mgartist: "migu", mgtoplist: "migu",
  thtrack: "taihe", thplaylist: "taihe", thalbum: "taihe", thartist: "taihe",
  lmtrack: "localmusic", lmplaylist: "localmusic",
  myplaylist: "localmusic",
};

export const SEARCHABLE_SOURCES: ProviderName[] = ["netease", "qq", "kugou", "kuwo", "bilibili", "migu", "taihe"];
export const DEFAULT_FAILOVER_SOURCES: ProviderName[] = ["kuwo", "qq", "migu"];
const LOGIN_PROVIDERS: LoginProvider[] = [
  { id: "netease", name: "网易云音乐" },
  { id: "qq", name: "QQ音乐" },
  { id: "bilibili", name: "哔哩哔哩" },
  { id: "migu", name: "咪咕音乐" },
];
const LOGIN_URLS: Record<string, string> = {
  netease: "https://music.163.com/#/login",
  qq: "https://y.qq.com/portal/profile.html",
  kugou: "https://www.kugou.com",
  kuwo: "https://www.kuwo.cn",
  bilibili: "https://passport.bilibili.com/login",
  migu: "https://music.migu.cn",
  taihe: "https://music.taihe.com",
};
const LOGIN_COOKIE_CLEAR_TARGETS: Record<string, Array<{ url: string; names: string[] }>> = {
  netease: [
    { url: "https://music.163.com", names: ["MUSIC_U", "__csrf", "NMTID", "MUSIC_A"] },
    { url: "https://interface3.music.163.com", names: ["MUSIC_U", "__csrf", "NMTID", "MUSIC_A"] },
  ],
  qq: [
    { url: "https://y.qq.com", names: ["uin", "wxuin", "qm_keyst", "qqmusic_key", "p_uin", "p_skey", "skey"] },
    { url: "https://u.y.qq.com", names: ["uin", "wxuin", "qm_keyst", "qqmusic_key", "p_uin", "p_skey", "skey"] },
    { url: "https://c.y.qq.com", names: ["uin", "wxuin", "qm_keyst", "qqmusic_key", "p_uin", "p_skey", "skey"] },
    { url: "https://i.y.qq.com", names: ["uin", "wxuin", "qm_keyst", "qqmusic_key", "p_uin", "p_skey", "skey"] },
    { url: "https://qq.com", names: ["uin", "wxuin", "qm_keyst", "qqmusic_key", "p_uin", "p_skey", "skey"] },
    { url: "https://www.qq.com", names: ["uin", "wxuin", "qm_keyst", "qqmusic_key", "p_uin", "p_skey", "skey"] },
  ],
  bilibili: [
    { url: "https://www.bilibili.com", names: ["SESSDATA", "bili_jct", "DedeUserID", "DedeUserID__ckMd5", "sid", "buvid3", "buvid4", "b_nut"] },
    { url: "https://api.bilibili.com", names: ["SESSDATA", "bili_jct", "DedeUserID", "DedeUserID__ckMd5", "sid", "buvid3", "buvid4", "b_nut"] },
    { url: "https://passport.bilibili.com", names: ["SESSDATA", "bili_jct", "DedeUserID", "DedeUserID__ckMd5", "sid", "buvid3", "buvid4", "b_nut"] },
    { url: "https://space.bilibili.com", names: ["SESSDATA", "bili_jct", "DedeUserID", "DedeUserID__ckMd5", "sid", "buvid3", "buvid4", "b_nut"] },
  ],
  migu: [
    {
      url: "https://music.migu.cn",
      names: [
        "migu_music_sid", "migu_music_platinum", "migu_music_level", "migu_music_nickname",
        "migu_music_avatar", "migu_music_uid", "migu_music_credit_level", "migu_music_passid",
        "migu_music_email", "migu_music_msisdn", "migu_music_status",
      ],
    },
    { url: "https://passport.migu.cn", names: ["USessionID", "LTToken"] },
  ],
};

function isProviderName(value: string): value is ProviderName {
  return value in PROVIDERS;
}

function getAutoChooseSourceList(): ProviderName[] {
  const raw = getSettingsValue<string[]>("autoChooseSourceList", "auto_choose_source_list", DEFAULT_FAILOVER_SOURCES);
  const normalized = raw.filter((source): source is ProviderName =>
    isProviderName(source) && SEARCHABLE_SOURCES.includes(source as ProviderName)
  );
  return normalized.length > 0 ? normalized : DEFAULT_FAILOVER_SOURCES;
}

function getProviderByItemId(itemId: string) {
  const prefix = itemId.split("_")[0];
  const name = TRACK_PREFIX_MAP[prefix];
  return name ? { name, provider: PROVIDERS[name] } : null;
}

export function getProviderByName(name: string) {
  return PROVIDERS[name as ProviderName] ?? null;
}

export function getProviderNameByItemId(itemId: string): ProviderName | null {
  const prefix = itemId.split("_")[0];
  return TRACK_PREFIX_MAP[prefix] ?? null;
}

const playlistCache = new Map<string, { data: Playlist; ts: number }>();
const CACHE_TTL = 60 * 60 * 1000;
const ALLMUSIC_SEARCH_TIMEOUT_MS = 6500;
const SINGLE_SOURCE_SEARCH_TIMEOUT_MS = 10000;

function qs(params: Record<string, string | number | boolean>): string {
  return new URLSearchParams(
    Object.fromEntries(Object.entries(params).map(([k, v]) => [k, String(v)]))
  ).toString();
}

function lsGet<T>(key: string): T | null {
  try { const v = localStorage.getItem(key); return v ? JSON.parse(v) : null; } catch { return null; }
}
function lsSet(key: string, value: unknown) {
  localStorage.setItem(key, JSON.stringify(value));
}

function getSettingsValue<T>(settingKey: string, legacyKey: string, fallback: T): T {
  const settings = lsGet<Record<string, unknown>>("listen1_settings");
  const value = settings?.[settingKey];
  if (value !== undefined) return value as T;
  const legacyValue = lsGet<T>(legacyKey);
  return legacyValue === null ? fallback : legacyValue;
}

function normalizeMatchText(value: string) {
  return value
    .toLowerCase()
    .replace(/[（(].*?[）)]/g, "")
    .replace(/\s+/g, "")
    .trim();
}

function isLikelySameTrack(candidate: Track, target: { title: string; artist: string }) {
  const candidateTitle = normalizeMatchText(candidate.title);
  const targetTitle = normalizeMatchText(target.title);
  const candidateArtist = normalizeMatchText(candidate.artist);
  const targetArtist = normalizeMatchText(target.artist);

  if (!candidateTitle || !targetTitle) return false;
  const titleMatches = candidateTitle === targetTitle ||
    candidateTitle.includes(targetTitle) ||
    targetTitle.includes(candidateTitle);
  if (!titleMatches) return false;
  if (!targetArtist || !candidateArtist) return true;
  return candidateArtist.includes(targetArtist) || targetArtist.includes(candidateArtist);
}

async function resolveSearchPlayable(
  sourceName: ProviderName,
  target: { id?: string; title: string; artist: string },
  excludeTrackIds: Set<string> = new Set()
): Promise<UrlResult | null> {
  const provider = PROVIDERS[sourceName];
  const keyword = `${target.title} ${target.artist}`;
  const data = await provider.search(`/search?keywords=${encodeURIComponent(keyword)}&curpage=1&type=0`);
  const candidates = data.result.filter((candidate) =>
    !candidate.disabled &&
    !excludeTrackIds.has(candidate.id) &&
    isLikelySameTrack(candidate, target)
  );

  for (const candidate of candidates) {
    const url = await provider.get_song_url(candidate.id, candidate).catch(() => null);
    if (url?.url) return { ...url, platform: sourceName, track: candidate };
  }
  return null;
}

function dedupeTracks(tracks: Track[]): Track[] {
  const seen = new Set<string>();
  const result: Track[] = [];
  for (const track of tracks) {
    if (!track.id || seen.has(track.id)) continue;
    seen.add(track.id);
    result.push(track);
  }
  return result;
}

function withTimeout<T>(promise: Promise<T>, ms: number, label: string): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | null = null;
  const timeout = new Promise<T>((_, reject) => {
    timer = setTimeout(() => reject(new Error(`${label} timed out after ${ms}ms`)), ms);
  });
  return Promise.race([promise, timeout]).finally(() => {
    if (timer) clearTimeout(timer);
  });
}

function getDefaultPlaylistFilterId(source: ProviderName): string {
  const group = PROVIDERS[source].get_playlist_filters()[0];
  return group?.items?.[0]?.id ?? "";
}

// My playlist management (localStorage-backed, key-compatible with original)
const myplaylistLib = {
  guid(): string {
    const s4 = () => Math.floor((1 + Math.random()) * 0x10000).toString(16).substring(1);
    return `${s4() + s4()}-${s4()}-${s4()}-${s4()}-${s4()}${s4()}${s4()}`;
  },
  getKey(type: "my" | "favorite"): string {
    return type === "my" ? "playerlists" : "favoriteplayerlists";
  },
  show(type: "my" | "favorite"): Playlist[] {
    const key = this.getKey(type);
    const ids: string[] = lsGet(key) ?? [];
    return ids.map((id) => lsGet<Playlist>(id)).filter(Boolean) as Playlist[];
  },
  get(id: string): Playlist | null {
    return lsGet<Playlist>(id);
  },
  create(type: "my" | "favorite", title: string, track?: Track): string {
    const id = `myplaylist_${this.guid()}`;
    const pl: Playlist = { info: { id, title, cover_img_url: "" }, tracks: track ? [track] : [] };
    lsSet(id, pl);
    const key = this.getKey(type);
    const ids: string[] = lsGet(key) ?? [];
    ids.push(id);
    lsSet(key, ids);
    return id;
  },
  edit(id: string, title: string, coverImgUrl: string) {
    const pl = lsGet<Playlist>(id);
    if (!pl) return;
    pl.info.title = title;
    pl.info.cover_img_url = coverImgUrl;
    lsSet(id, pl);
  },
  remove(type: "my" | "favorite", id: string) {
    const key = this.getKey(type);
    const ids = (lsGet<string[]>(key) ?? []).filter((i) => i !== id);
    lsSet(key, ids);
    if (type === "my") localStorage.removeItem(id);
  },
  addTrack(id: string, track: Track): Playlist | null {
    const pl = lsGet<Playlist>(id);
    if (!pl) return null;
    if (!pl.tracks.find((t) => t.id === track.id)) pl.tracks.push({ ...track });
    lsSet(id, pl);
    return pl;
  },
  removeTrack(id: string, trackId: string): Playlist | null {
    const pl = lsGet<Playlist>(id);
    if (!pl) return null;
    pl.tracks = pl.tracks.filter((t) => t.id !== trackId);
    lsSet(id, pl);
    return pl;
  },
  reorderTrack(id: string, trackId: string, toTrackId: string, direction: "top" | "bottom") {
    const pl = lsGet<Playlist>(id);
    if (!pl) return;
    const from = pl.tracks.findIndex((t) => t.id === trackId);
    let to = pl.tracks.findIndex((t) => t.id === toTrackId);
    if (from === -1 || to === -1 || from === to) return;
    if (to > from) to--;
    const offset = direction === "top" ? 0 : 1;
    const [item] = pl.tracks.splice(from, 1);
    pl.tracks.splice(to + offset, 0, item);
    lsSet(id, pl);
  },
  saveExternalPlaylist(type: "my" | "favorite", pl: Playlist): string {
    const id = `myplaylist_${this.guid()}`;
    const saved: Playlist = { info: { ...pl.info, id }, tracks: pl.tracks.map((t) => ({ ...t, url: undefined, sound_url: undefined, disabled: false })) };
    lsSet(id, saved);
    const key = this.getKey(type);
    const ids: string[] = lsGet(key) ?? [];
    ids.push(id);
    lsSet(key, ids);
    return id;
  },
  merge(srcId: string, targetId: string) {
    const src = lsGet<Playlist>(srcId);
    const target = lsGet<Playlist>(targetId);
    if (!src || !target) return;
    const existing = new Set(src.tracks.map((t) => t.id));
    target.tracks.forEach((t) => { if (!existing.has(t.id)) src.tracks.push({ ...t }); });
    lsSet(srcId, src);
  },
  localAdd(id: string, tracks: Track[]): Playlist {
    const existing = lsGet<Playlist>(id) ?? { info: { id, title: "本地音乐" }, tracks: [] };
    const existingIds = new Set(existing.tracks.map((t) => t.id));
    tracks.filter((t) => !existingIds.has(t.id)).forEach((t) => existing.tracks.push(t));
    lsSet(id, existing);
    return existing;
  },
  isFavorite(id: string): boolean {
    const ids: string[] = lsGet("favoriteplayerlists") ?? [];
    // Check if any favorite playlist was cloned from this id (stored as source_url)
    return ids.some((fid) => {
      const pl = lsGet<Playlist>(fid);
      return pl?.info?.source_url === id || fid === id;
    });
  },
};

export { myplaylistLib };

export const MediaService = {
  async search(source: string, options: Record<string, string | number>): Promise<SearchResult> {
    const url = `/search?${qs(options)}`;
    if (source === "allmusic") {
      const results = await Promise.allSettled(
        SEARCHABLE_SOURCES.map((name) =>
          withTimeout(PROVIDERS[name].search(url), ALLMUSIC_SEARCH_TIMEOUT_MS, `[MediaService] ${name} search`)
            .catch((error) => {
              console.error(`[MediaService] ${name} search failed`, error);
              return { result: [], total: 0, type: "song" as const };
            })
        )
      );
      const merged: Track[] = [];
      const fulfilled = results.filter((r): r is PromiseFulfilledResult<SearchResult> => r.status === "fulfilled").map((r) => r.value.result);
      const maxLen = Math.max(0, ...fulfilled.map((r) => r.length));
      for (let i = 0; i < maxLen; i++) {
        fulfilled.forEach((r) => { if (i < r.length) merged.push(r[i]); });
      }
      return { result: merged, total: 1000, type: "song" };
    }
    const provider = getProviderByName(source);
    if (!provider) return { result: [], total: 0, type: "song" };
    return withTimeout(provider.search(url), SINGLE_SOURCE_SEARCH_TIMEOUT_MS, `[MediaService] ${source} search`);
  },

  async showPlaylistArray(source: string, offset: number, filter_id: string): Promise<{ result: PlaylistInfo[]; source?: ProviderName; filter_id?: string }> {
    const requested = (getProviderByName(source) ? source : "netease") as ProviderName;
    const provider = PROVIDERS[requested];
    const effectiveFilterId = filter_id || getDefaultPlaylistFilterId(requested);
    try {
      const res = await provider.show_playlist(`/show_playlist?${qs({ offset, filter_id: effectiveFilterId })}`);
      return { ...res, source: requested, filter_id: effectiveFilterId };
    } catch (err) {
      console.error(`[MediaService] ${requested} playlist failed`, err);
      return { result: [], source: requested, filter_id: effectiveFilterId };
    }
  },

  getPlaylistFilters(source: string): PlaylistFilter[] {
    return getProviderByName(source)?.get_playlist_filters() ?? [];
  },

  async getPlaylist(listId: string, useCache = true): Promise<Playlist | null> {
    const prefix = listId.split("_")[0];
    if (prefix === "myplaylist") {
      return myplaylistLib.get(listId) ?? null;
    }
    if (prefix === "lmplaylist") {
      return localmusic.get_playlist();
    }
    if (useCache) {
      const cached = playlistCache.get(listId);
      if (cached && Date.now() - cached.ts < CACHE_TTL && cached.data.tracks.length > 0) {
        return cached.data;
      }
    }
    const info = getProviderByItemId(listId);
    if (!info) return null;
    const loader = !useCache && info.provider.get_playlist_full ? info.provider.get_playlist_full : info.provider.get_playlist;
    const data = await loader.call(info.provider, `/playlist?list_id=${listId}`);
    data.tracks = dedupeTracks(data.tracks);
    if (data.tracks.length > 0) playlistCache.set(listId, { data, ts: Date.now() });
    return data;
  },

  async getLyric(trackId: string, albumId: string, lyricUrl?: string, tlyricUrl?: string): Promise<LyricResult> {
    const info = getProviderByItemId(trackId);
    if (!info) return { lyric: "", tlyric: "" };
    const result = await info.provider.lyric(`/lyric?${qs({ track_id: trackId, album_id: albumId ?? "", lyric_url: lyricUrl ?? "", tlyric_url: tlyricUrl ?? "" })}`);
    return {
      lyric: normalizeLyricText((result as { lyric?: unknown }).lyric),
      tlyric: normalizeLyricText((result as { tlyric?: unknown }).tlyric),
    };
  },

  // Core: bootstrap with auto-failover across platforms
  async bootstrapTrack(
    track: Track,
    onSuccess: (result: UrlResult) => void,
    onFail: () => void
  ): Promise<void> {
    const info = getProviderByItemId(track.id);
    if (!info) { onFail(); return; }

    let result: UrlResult | null = null;
    try {
      result = await info.provider.get_song_url(track.id, track);
    } catch {}

    if (result?.url) {
      onSuccess(result);
      return;
    }

    const sameSourceFallback = await resolveSearchPlayable(info.name, track, new Set([track.id])).catch(() => null);
    if (sameSourceFallback?.url) {
      onSuccess(sameSourceFallback);
      return;
    }

    // Failover: auto-choose from other sources
    const autoChoose = getSettingsValue<boolean>("enableAutoChooseSource", "enable_auto_choose_source", true);
    if (!autoChoose) { onFail(); return; }

    const sourceList = getAutoChooseSourceList();
    const trackPlatform = info.name;
    const failoverList = sourceList.filter((s) => s !== trackPlatform && s !== track.source);

    const searchPromises = failoverList.map((sourceName) =>
      resolveSearchPlayable(sourceName, track).catch(() => null)
    );

    // Return first successful result
    const found = await Promise.any(
      searchPromises.map((p) => p.then((r) => { if (!r) throw new Error("no result"); return r; }))
    ).catch(() => null);

    if (found?.url) {
      onSuccess(found);
    } else {
      onFail();
    }
  },

  async parseURL(url: string): Promise<PlaylistInfo | null> {
    const results = await Promise.allSettled(Object.values(PROVIDERS).map((p) => p.parse_url(url)));
    for (const r of results) {
      if (r.status === "fulfilled" && r.value) return r.value;
    }
    return null;
  },

  showMyPlaylist(): Playlist[] {
    return myplaylistLib.show("my");
  },

  showFavPlaylist(): Playlist[] {
    return myplaylistLib.show("favorite");
  },

  getLoginProviders(): LoginProvider[] {
    return [...LOGIN_PROVIDERS];
  },

  getLoginUrl(source: string): string {
    return LOGIN_URLS[source] ?? "";
  },

  async getUser(source: string): Promise<UserStatus> {
    try {
      const provider = getProviderByName(source);
      if (provider?.get_user) return await provider.get_user();
    } catch (error) {
      console.error(`[MediaService] ${source} get user failed`, error);
    }
    return { status: "fail", data: { is_login: false } };
  },

  async getUserCreatedPlaylist(source: string, userId: string | number): Promise<PlaylistInfo[]> {
    try {
      const provider = getProviderByName(source);
      if (!provider?.get_user_created_playlist) return [];
      const response = await provider.get_user_created_playlist(`/get_user_create_playlist?${qs({ user_id: userId })}`);
      return response.status === "success" ? response.data.playlists : [];
    } catch (error) {
      console.error(`[MediaService] ${source} get user created playlist failed`, error);
      return [];
    }
  },

  async getUserFavoritePlaylist(source: string, userId: string | number): Promise<PlaylistInfo[]> {
    try {
      const provider = getProviderByName(source);
      if (!provider?.get_user_favorite_playlist) return [];
      const response = await provider.get_user_favorite_playlist(`/get_user_favorite_playlist?${qs({ user_id: userId })}`);
      return response.status === "success" ? response.data.playlists : [];
    } catch (error) {
      console.error(`[MediaService] ${source} get user favorite playlist failed`, error);
      return [];
    }
  },

  async logout(source: string): Promise<void> {
    localStorage.removeItem(`listen1_auth_${source}`);
    const targets = LOGIN_COOKIE_CLEAR_TARGETS[source] ?? [];
    await Promise.all(targets.map((target) => clearLoginCookies(target.url, target.names).catch(() => undefined)));
  },

  createMyPlaylist(title: string, track?: Track): string {
    return myplaylistLib.create("my", title, track);
  },

  editMyPlaylist(id: string, title: string, coverImgUrl: string) {
    myplaylistLib.edit(id, title, coverImgUrl);
    playlistCache.delete(id);
  },

  removeMyPlaylist(id: string, type: "my" | "favorite" = "my") {
    myplaylistLib.remove(type, id);
    playlistCache.delete(id);
  },

  addTrackToMyPlaylist(id: string, track: Track): Playlist | null {
    const result = myplaylistLib.addTrack(id, track);
    playlistCache.delete(id);
    return result;
  },

  removeTrackFromMyPlaylist(id: string, trackId: string) {
    myplaylistLib.removeTrack(id, trackId);
    playlistCache.delete(id);
  },

  async clonePlaylist(listId: string, type: "my" | "favorite"): Promise<void> {
    const pl = await this.getPlaylist(listId, false);
    if (pl) myplaylistLib.saveExternalPlaylist(type, pl);
  },

  saveExternalPlaylist(type: "my" | "favorite", playlist: Playlist): string {
    const id = myplaylistLib.saveExternalPlaylist(type, playlist);
    playlistCache.delete(id);
    return id;
  },

  mergePlaylist(srcId: string, targetId: string) {
    myplaylistLib.merge(srcId, targetId);
    playlistCache.delete(srcId);
  },

  isFavorite(listId: string): boolean {
    return myplaylistLib.isFavorite(listId);
  },

  async getUrl(trackId: string, trackHint?: Track, forceFailover = false, excludeSources: string[] = []): Promise<UrlResult | null> {
    const info = getProviderByItemId(trackId);
    if (!info) return null;

    let result: UrlResult | null = null;
    if (!forceFailover) {
      try {
        result = await info.provider.get_song_url(trackId, trackHint);
      } catch {}

      if (result?.url) return result;
    }

    // Need the track info for keyword search — look in current playing or cache
    const stored = localStorage.getItem("current-playing");
    let tracks: Array<{ id: string; title: string; artist: string }> = [];
    try { if (stored) tracks = JSON.parse(stored); } catch {}
    const trackInfo = trackHint ?? tracks.find((t) => t.id === trackId);
    if (!trackInfo) return null;

    if (!forceFailover) {
      const sameSourceFallback = await resolveSearchPlayable(info.name, { ...trackInfo, id: trackId }, new Set([trackId])).catch(() => null);
      if (sameSourceFallback?.url) return sameSourceFallback;
    }

    // Failover
    const autoChoose = getSettingsValue<boolean>("enableAutoChooseSource", "enable_auto_choose_source", true);
    if (!autoChoose) return null;

    const sourceList = getAutoChooseSourceList();
    const excluded = new Set<ProviderName>([
      info.name,
      ...excludeSources.filter((source): source is ProviderName => Boolean(getProviderByName(source))),
    ]);
    const failoverList = sourceList.filter((s) => !excluded.has(s));

    const searchPromises = failoverList.map((sourceName) =>
      resolveSearchPlayable(sourceName, trackInfo).catch(() => null)
    );

    const found = await Promise.any(
      searchPromises.map((p) => p.then((r) => { if (!r) throw new Error("no result"); return r; }))
    ).catch(() => null);

    return found;
  },

  getSources(): ProviderName[] {
    return [...SEARCHABLE_SOURCES];
  },
};
