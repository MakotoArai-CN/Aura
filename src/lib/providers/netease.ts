import { get, postForm, setBackendCookie } from "../tauri";
import { getParameterByName, qs } from "./utils";
import { weapi, eapi } from "./crypto";
import type { SearchResult, Playlist, PlaylistInfo, LyricResult, UrlResult, PlaylistFilter, UserStatus, UserPlaylistResult } from "./types";
import type { Track } from "../stores/player";

type NeteaseArtist = { name?: string; id?: string | number };
type NeteaseAlbum = { name?: string; id?: string | number; picUrl?: string; picUrlStr?: string; pic_str?: string; pic?: string | number };
type NeteaseTrackId = { id?: string | number };

function convertSong(s: Record<string, unknown>): Track {
  const artists = (Array.isArray(s.artists) ? s.artists : s.ar ?? []) as NeteaseArtist[];
  const album = (s.album ?? s.al ?? {}) as NeteaseAlbum;
  const artist = artists[0] ?? {};
  const pic =
    album?.picUrl ??
    album?.picUrlStr ??
    album?.pic_str ??
    (album?.pic != null ? `https://p1.music.126.net/${album.pic}.jpg` : "");
  return {
    id: `netrack_${s.id}`,
    title: String(s.name ?? ""),
    artist: artist.name ?? "",
    artist_id: artist.id != null ? `neartist_${artist.id}` : "",
    album: album?.name ?? "",
    album_id: album?.id != null ? `nealbum_${album.id}` : "",
    source: "netease",
    source_url: `https://music.163.com/#/song?id=${s.id}`,
    img_url: String(pic || ""),
    disabled: false,
  };
}

function splitArray<T>(items: T[], size: number): T[][] {
  const result: T[][] = [];
  for (let i = 0; i < items.length; i += size) {
    result.push(items.slice(i, i + size));
  }
  return result;
}

async function getSongDetails(trackIds: NeteaseTrackId[]): Promise<Track[]> {
  const ids = trackIds.map((item) => item.id).filter((id): id is string | number => id !== undefined && id !== null);
  if (ids.length === 0) return [];
  const batches = splitArray(ids, 1000);
  const responses = await Promise.all(batches.map((batch) => {
    const data = weapi({
      c: `[${batch.map((id) => `{"id":${id}}`).join(",")}]`,
      ids: `[${batch.join(",")}]`,
    });
    return postForm<{ songs?: Record<string, unknown>[] }>(
      "https://music.163.com/weapi/v3/song/detail",
      data as Record<string, string>
    ).catch(() => null);
  }));
  return responses.flatMap((resp) => (resp?.data.songs ?? []).map(convertSong));
}

async function hydrateSearchSongs(songs: Record<string, unknown>[]): Promise<Track[]> {
  const base = songs.map(convertSong).filter((track) => track.id !== "netrack_");
  const missingCover = base.some((track) => !track.img_url);
  if (!missingCover) return base;
  const ids = base.map((track) => ({ id: track.id.replace("netrack_", "") }));
  const details = await getSongDetails(ids).catch(() => []);
  const detailsById = new Map(details.map((track) => [track.id, track]));
  return base.map((track) => ({ ...track, ...(detailsById.get(track.id) ?? {}) }));
}

export const netease = {
  async show_playlist(url: string): Promise<{ result: PlaylistInfo[] }> {
    const offset = getParameterByName("offset", url);
    const filterId = getParameterByName("filter_id", url);

    if (filterId === "toplist") {
      if (Number(offset) > 0) return { result: [] };
      const data = weapi({});
      const resp = await postForm<{ list: Array<{ coverImgUrl: string; id: number; name: string }> }>(
        "https://music.163.com/weapi/toplist/detail",
        data as Record<string, string>
      );
      const result = (resp.data.list ?? []).map((item) => ({
        cover_img_url: item.coverImgUrl,
        id: `neplaylist_${item.id}`,
        source_url: `https://music.163.com/#/playlist?id=${item.id}`,
        title: item.name,
      }));
      return { result };
    }

    let filter = filterId ? `&cat=${filterId}` : "";
    const targetUrl = `https://music.163.com/discover/playlist/?order=hot${filter}&limit=35${offset ? `&offset=${offset}` : ""}`;
    const resp = await get<string>(targetUrl);
    const html = resp.text ?? "";
    const parser = new DOMParser();
    const doc = parser.parseFromString(html, "text/html");
    const items = Array.from(doc.getElementsByClassName("m-cvrlst")[0]?.children ?? []);
    const result = items.map((item) => {
      const a = item.getElementsByTagName("div")[0]?.getElementsByTagName("a")[0];
      const img = item.getElementsByTagName("img")[0];
      const id = getParameterByName("id", a?.href ?? "");
      return {
        cover_img_url: img?.src.replace("140y140", "512y512") ?? "",
        title: a?.title ?? "",
        id: `neplaylist_${id}`,
        source_url: `https://music.163.com/#/playlist?id=${id}`,
      };
    });
    return { result };
  },

  async search(url: string): Promise<SearchResult> {
    const keyword = getParameterByName("keywords", url);
    const curpage = getParameterByName("curpage", url);
    const searchType = getParameterByName("type", url);
    const neType = searchType === "1" ? "1000" : "1";

    const offset = String(20 * (Number(curpage) - 1));
    const resp = await postForm<{ result: Record<string, unknown> }>(
      "https://music.163.com/weapi/search/get?csrf_token=",
      weapi({ s: keyword, offset, limit: 20, type: Number(neType), csrf_token: "" }) as Record<string, string>
    );

    const data = resp.data;
    let result: Track[] = [];
    let total = 0;

    const searchResult = (data?.result ?? {}) as {
      songs?: Record<string, unknown>[];
      songCount?: number;
      playlists?: Array<{ id: number; name: string; coverImgUrl: string; creator: { nickname: string }; trackCount: number }>;
      playlistCount?: number;
    };

    if (searchType === "0") {
      const songs = searchResult.songs ?? [];
      result = await hydrateSearchSongs(songs);
      total = searchResult.songCount ?? songs.length;
    } else if (searchType === "1") {
      const playlists = searchResult.playlists ?? [];
      result = playlists.map((info) => ({
        id: `neplaylist_${info.id}`,
        title: info.name,
        artist: info.creator?.nickname ?? "",
        source: "netease",
        source_url: `https://music.163.com/#/playlist?id=${info.id}`,
        img_url: info.coverImgUrl,
        url: `neplaylist_${info.id}`,
      }));
      total = searchResult.playlistCount ?? playlists.length;
    }

    return { result, total, type: searchType === "1" ? "playlist" : "song" };
  },

  async get_playlist(url: string): Promise<Playlist> {
    const listId = getParameterByName("list_id", url);
    const prefix = listId.split("_")[0];

    if (prefix === "nealbum") {
      return this._getAlbum(listId);
    } else if (prefix === "neartist") {
      return this._getArtist(listId);
    }
    return this._getPlaylist(listId);
  },

  async _getPlaylist(listId: string): Promise<Playlist> {
    const id = listId.split("_").pop();
    const data = weapi({ id, offset: 0, total: true, limit: 1000, n: 1000, s: 8, csrf_token: "" });
    const resp = await postForm<{ playlist: { coverImgUrl: string; name: string; id: number; tracks?: Record<string, unknown>[]; trackIds?: NeteaseTrackId[] } }>(
      "https://music.163.com/weapi/v3/playlist/detail?csrf_token=",
      data as Record<string, string>
    );
    const pl = resp.data.playlist;
    const trackIds = pl?.trackIds ?? [];
    const tracks = trackIds.length > 0 ? await getSongDetails(trackIds) : (pl?.tracks ?? []).map(convertSong);
    return {
      info: {
        id: `neplaylist_${pl?.id ?? id}`,
        title: pl?.name ?? "网易云歌单",
        cover_img_url: pl?.coverImgUrl ?? "",
        source_url: `https://music.163.com/#/playlist?id=${pl?.id ?? id}`,
      },
      tracks,
    };
  },

  async get_playlist_full(url: string): Promise<Playlist> {
    return this.get_playlist(url);
  },

  async _getAlbum(listId: string): Promise<Playlist> {
    const id = listId.split("_").pop();
    const resp = await get<{ album: { picUrl: string; name: string; id: number }; songs: Record<string, unknown>[] }>(
      `https://music.163.com/api/album/${id}`
    );
    const album = resp.data.album;
    return {
      info: {
        id: `nealbum_${album.id}`,
        title: album.name,
        cover_img_url: album.picUrl,
        source_url: `https://music.163.com/#/album?id=${album.id}`,
      },
      tracks: resp.data.songs.map(convertSong),
    };
  },

  async _getArtist(listId: string): Promise<Playlist> {
    const id = listId.split("_").pop();
    const resp = await get<{ artist: { picUrl: string; name: string; id: number }; hotSongs: Record<string, unknown>[] }>(
      `https://music.163.com/api/artist/${id}`
    );
    const artist = resp.data.artist;
    return {
      info: {
        id: `neartist_${artist.id}`,
        title: artist.name,
        cover_img_url: artist.picUrl,
        source_url: `https://music.163.com/#/artist?id=${artist.id}`,
      },
      tracks: resp.data.hotSongs.map(convertSong),
    };
  },

  async get_song_url(trackId: string): Promise<UrlResult | null> {
    const songId = trackId.replace("netrack_", "");
    const d = eapi("/api/song/enhance/player/url", { ids: `[${songId}]`, br: 999000 });
    await setBackendCookie("https://interface3.music.163.com", "os=pc; Path=/; Max-Age=3153600000").catch(() => undefined);
    const resp = await postForm<{ data: Array<{ url: string | null; br: number }> }>(
      "https://interface3.music.163.com/eapi/song/enhance/player/url",
      d as Record<string, string>
    );
    const item = resp.data.data?.[0];
    if (!item?.url) return null;
    return {
      url: item.url,
      bitrate: `${(item.br / 1000).toFixed(0)}kbps`,
      platform: "netease",
    };
  },

  async lyric(url: string): Promise<LyricResult> {
    const trackId = getParameterByName("track_id", url).split("_").pop();
    const data = weapi({ id: trackId, lv: -1, tv: -1, csrf_token: "" });
    const resp = await postForm<{ lrc?: { lyric: string }; tlyric?: { lyric: string } }>(
      "https://music.163.com/weapi/song/lyric?csrf_token=",
      data as Record<string, string>
    );
    return {
      lyric: resp.data.lrc?.lyric ?? "",
      tlyric: resp.data.tlyric?.lyric ?? "",
    };
  },

  async get_user(): Promise<UserStatus> {
    const data = weapi({});
    const resp = await postForm<{
      account?: { id?: number; userName?: string } | null;
      profile?: { nickname?: string; avatarUrl?: string };
    }>(
      "https://music.163.com/api/nuser/account/get",
      data as Record<string, string>
    );
    if (!resp.data.account) return { status: "fail", data: { is_login: false } };
    return {
      status: "success",
      data: {
        is_login: true,
        user_id: resp.data.account.id,
        user_name: resp.data.account.userName,
        nickname: resp.data.profile?.nickname ?? resp.data.account.userName ?? "网易云用户",
        avatar: resp.data.profile?.avatarUrl ?? "",
        platform: "netease",
        data: resp.data,
      },
    };
  },

  async get_user_playlist(url: string, playlistType: "created" | "favorite"): Promise<UserPlaylistResult> {
    const userId = getParameterByName("user_id", url);
    if (!userId) return { status: "fail", data: { playlists: [] } };
    const resp = await postForm<{ playlist?: Array<{ subscribed?: boolean; coverImgUrl?: string; id: number; name?: string }> }>(
      "https://music.163.com/api/user/playlist",
      {
        uid: userId,
        limit: "1000",
        offset: "0",
        includeVideo: "true",
      }
    );
    const playlists = (resp.data.playlist ?? [])
      .filter((item) => playlistType === "created" ? item.subscribed === false : item.subscribed === true)
      .map((item) => ({
        cover_img_url: item.coverImgUrl ?? "",
        id: `neplaylist_${item.id}`,
        source_url: `https://music.163.com/#/playlist?id=${item.id}`,
        title: item.name ?? "",
      }));
    return { status: "success", data: { playlists } };
  },

  async get_user_created_playlist(url: string): Promise<UserPlaylistResult> {
    return this.get_user_playlist(url, "created");
  },

  async get_user_favorite_playlist(url: string): Promise<UserPlaylistResult> {
    return this.get_user_playlist(url, "favorite");
  },

  async parse_url(url: string): Promise<PlaylistInfo | null> {
    let normalized = url
      .replace("music.163.com/#/discover/toplist?", "music.163.com/#/playlist?")
      .replace("music.163.com/#/my/m/music/", "music.163.com/")
      .replace("music.163.com/#/m/", "music.163.com/")
      .replace("music.163.com/#/", "music.163.com/");

    if (normalized.includes("//music.163.com/playlist")) {
      const pathMatch = /\/\/music\.163\.com\/playlist\/([0-9]+)/.exec(normalized);
      const queryId = getParameterByName("id", normalized);
      const id = queryId || pathMatch?.[1] || "";
      if (!id) return null;
      return { id: `neplaylist_${id}`, title: "" };
    } else if (normalized.includes("//music.163.com/artist")) {
      return { id: `neartist_${getParameterByName("id", normalized)}`, title: "" };
    } else if (normalized.includes("//music.163.com/album")) {
      const match = /\/\/music.163.com\/album\/([0-9]+)/.exec(normalized);
      const id = match ? match[1] : getParameterByName("id", normalized);
      return { id: `nealbum_${id}`, title: "" };
    }
    return null;
  },

  get_playlist_filters(): PlaylistFilter[] {
    return [
      {
        id: "netease",
        name: "网易云",
        items: [
          { id: "", name: "全部" },
          { id: "toplist", name: "排行榜" },
          { id: "流行", name: "流行" },
          { id: "民谣", name: "民谣" },
          { id: "电子", name: "电子" },
          { id: "舞曲", name: "舞曲" },
          { id: "说唱", name: "说唱" },
          { id: "轻音乐", name: "轻音乐" },
          { id: "爵士", name: "爵士" },
          { id: "乡村", name: "乡村" },
          { id: "R&B/Soul", name: "R&B" },
          { id: "摇滚", name: "摇滚" },
          { id: "古典", name: "古典" },
          { id: "古风", name: "古风" },
          { id: "影视原声", name: "影视" },
          { id: "ACG", name: "ACG" },
          { id: "儿童", name: "儿童" },
          { id: "游戏", name: "游戏" },
        ],
      },
    ];
  },
};
