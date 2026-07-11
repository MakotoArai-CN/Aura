import { get } from "../tauri";
import { getParameterByName } from "./utils";
import forge from "node-forge";
import type { SearchResult, Playlist, PlaylistInfo, LyricResult, UrlResult, PlaylistFilter } from "./types";
import type { Track } from "../stores/player";

const TAIHE_BASE = "https://music.taihe.com/v1";
const TAIHE_APPID = "16073360";
const TAIHE_SECRET = "0b50b02fd0d73a9c4c8c3a781c30845f";

function taiheSign(params: Record<string, string | number>): string {
  const p = { ...params };
  const q = new URLSearchParams(Object.fromEntries(Object.entries(p).map(([k, v]) => [k, String(v)])));
  q.sort();
  const signStr = decodeURIComponent(`${q.toString()}${TAIHE_SECRET}`);
  return forge.md5.create().update(forge.util.encodeUtf8(signStr)).digest().toHex();
}

function taiheUrl(path: string, params: Record<string, string | number> = {}): string {
  const p: Record<string, string | number> = {
    ...params,
    timestamp: Math.round(Date.now() / 1000),
    appid: TAIHE_APPID,
  };
  p.sign = taiheSign(p);
  const q = new URLSearchParams(Object.fromEntries(Object.entries(p).map(([k, v]) => [k, String(v)])));
  return `${TAIHE_BASE}${path}?${q.toString()}`;
}

function convertSong(s: Record<string, unknown>): Track {
  const artists = s.artist as Array<{ name: string; artistCode: string }> ?? [];
  return {
    id: `thtrack_${s.id}`,
    title: (s.title ?? "") as string,
    artist: artists[0]?.name ?? "",
    artist_id: `thartist_${artists[0]?.artistCode ?? ""}`,
    album: (s.albumTitle ?? "") as string,
    album_id: `thalbum_${s.albumAssetCode ?? ""}`,
    img_url: (s.pic ?? "") as string,
    source: "taihe",
    source_url: `https://music.taihe.com/song/${s.id}`,
    lyric_url: (s.lyric ?? "") as string,
  };
}

export const taihe = {
  async search(url: string): Promise<SearchResult> {
    const keyword = getParameterByName("keywords", url);
    const curpage = getParameterByName("curpage", url);
    const searchType = getParameterByName("type", url);
    if (searchType === "1") return { result: [], total: 0, type: "song" };

    const resp = await get<{ data: { typeTrack: Record<string, unknown>[]; total: number } }>(
      taiheUrl("/search", { word: keyword, pageNo: Number(curpage) || 1, type: 1 })
    );
    return {
      result: (resp.data.data?.typeTrack ?? []).map(convertSong),
      total: resp.data.data?.total ?? 0,
      type: "song",
    };
  },

  async show_playlist(url: string): Promise<{ result: PlaylistInfo[] }> {
    const offset = Number(getParameterByName("offset", url)) || 0;
    const filterId = getParameterByName("filter_id", url);
    const page = Math.floor(offset / 20) + 1;
    const params: Record<string, string | number> = { pageNo: page, pageSize: 20 };
    if (filterId) params.subCateId = filterId;

    const resp = await get<{ data: { result?: Array<{ pic?: string; coverImageUrl?: string; title: string; id?: string | number; listId?: string | number }> } }>(
      taiheUrl("/tracklist/list", params)
    );
    return {
      result: (resp.data.data?.result ?? []).map((item) => ({
        cover_img_url: item.pic ?? item.coverImageUrl ?? "",
        title: item.title,
        id: `thplaylist_${item.id ?? item.listId}`,
        source_url: `https://music.taihe.com/songlist/${item.id ?? item.listId}`,
      })),
    };
  },

  async get_playlist(url: string): Promise<Playlist> {
    const listId = getParameterByName("list_id", url);
    const prefix = listId.split("_")[0];

    if (prefix === "thalbum") return this._getAlbum(listId);
    if (prefix === "thartist") return this._getArtist(listId);
    return this._getPlaylist(listId);
  },

  async _getPlaylist(listId: string): Promise<Playlist> {
    const id = listId.split("_").pop();
    const infoResp = await get<{ data: { coverImageUrl: string; title: string; id: string } }>(
      taiheUrl("/tracklist/info", { id: id! })
    );
    const tracksResp = await get<{ data: { trackList: Record<string, unknown>[] } }>(
      taiheUrl("/tracklist/info", { id: id!, pageNo: 1, pageSize: 500 })
    );
    return {
      info: { id: `thplaylist_${id}`, title: infoResp.data.data.title, cover_img_url: infoResp.data.data.coverImageUrl },
      tracks: (tracksResp.data.data?.trackList ?? []).map(convertSong),
    };
  },

  async get_playlist_full(url: string): Promise<Playlist> {
    const listId = getParameterByName("list_id", url);
    const prefix = listId.split("_")[0];
    if (prefix !== "thplaylist") return this.get_playlist(url);

    const id = listId.split("_").pop();
    const infoResp = await get<{ data: { pic?: string; coverImageUrl?: string; title: string; id: string; trackCount?: number } }>(
      taiheUrl("/tracklist/info", { id: id! })
    );
    const infoData = infoResp.data.data;
    const total = Number(infoData.trackCount ?? 0);
    const pageSize = 100;
    const pageCount = Math.max(1, total ? Math.ceil(total / pageSize) : 1);
    const pages = await Promise.all(Array.from({ length: pageCount }, (_, i) =>
      get<{ data: { trackList?: Record<string, unknown>[] } }>(
        taiheUrl("/tracklist/info", { id: id!, pageNo: i + 1, pageSize })
      ).catch(() => null)
    ));
    return {
      info: {
        id: `thplaylist_${id}`,
        title: infoData.title,
        cover_img_url: infoData.coverImageUrl ?? infoData.pic ?? "",
        source_url: `https://music.taihe.com/songlist/${id}`,
      },
      tracks: pages.flatMap((resp) => (resp?.data.data?.trackList ?? []).map(convertSong)),
    };
  },

  async _getAlbum(listId: string): Promise<Playlist> {
    const id = listId.split("_").pop();
    const resp = await get<{ data: { album: { coverImageUrl: string; title: string }; trackList: Record<string, unknown>[] } }>(
      taiheUrl("/album/info", { albumAssetCode: id! })
    );
    return {
      info: { id: `thalbum_${id}`, title: resp.data.data.album.title, cover_img_url: resp.data.data.album.coverImageUrl },
      tracks: (resp.data.data.trackList ?? []).map(convertSong),
    };
  },

  async _getArtist(listId: string): Promise<Playlist> {
    const id = listId.split("_").pop();
    const resp = await get<{ data: { artist?: { pic: string; name: string }; result?: Record<string, unknown>[]; trackList?: Record<string, unknown>[] } }>(
      taiheUrl("/artist/song", { artistCode: id!, pageNo: 1, pageSize: 50 })
    );
    const infoResp = await get<{ data: { pic: string; name: string } }>(
      taiheUrl("/artist/info", { artistCode: id! })
    ).catch(() => null);
    return {
      info: { id: `thartist_${id}`, title: infoResp?.data.data.name ?? resp.data.data.artist?.name ?? "千千音乐人", cover_img_url: infoResp?.data.data.pic ?? resp.data.data.artist?.pic ?? "" },
      tracks: (resp.data.data.result ?? resp.data.data.trackList ?? []).map(convertSong),
    };
  },

  async get_song_url(trackId: string): Promise<UrlResult | null> {
    const id = trackId.replace("thtrack_", "");
    const resp = await get<{ data: { path: string } }>(
      taiheUrl("/song/tracklink", { TSID: id, quality: "HQ" })
    );
    const url = resp.data.data?.path;
    if (!url) return null;
    return { url, platform: "taihe" };
  },

  async lyric(url: string): Promise<LyricResult> {
    const lyricUrl = getParameterByName("lyric_url", url);
    if (!lyricUrl) return { lyric: "", tlyric: "" };
    const resp = await get<string>(lyricUrl);
    return { lyric: resp.text ?? "", tlyric: "" };
  },

  async parse_url(url: string): Promise<PlaylistInfo | null> {
    if (url.includes("music.taihe.com/songlist/")) {
      const match = /\/songlist\/(\w+)/.exec(url);
      if (match) return { id: `thplaylist_${match[1]}`, title: "" };
    }
    return null;
  },

  get_playlist_filters(): PlaylistFilter[] {
    return [
      {
        id: "taihe",
        name: "千千",
        items: [
          { id: "", name: "全部" },
          { id: "华语", name: "华语" },
          { id: "欧美", name: "欧美" },
          { id: "日语", name: "日语" },
          { id: "韩语", name: "韩语" },
          { id: "流行", name: "流行" },
        ],
      },
    ];
  },
};
