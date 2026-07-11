import { get } from "../tauri";
import { getParameterByName } from "./utils";
import type { SearchResult, Playlist, PlaylistInfo, LyricResult, UrlResult, PlaylistFilter } from "./types";
import type { Track } from "../stores/player";

function convertSong(s: Record<string, unknown>): Track {
  const hash = String(s.FileHash ?? s.hash ?? s.Hash ?? "");
  const albumId = String(s.AlbumID ?? s.album_id ?? s.AlbumID ?? "");
  const singerId = Array.isArray(s.SingerId) ? s.SingerId[0] : s.singerid ?? s.SingerId ?? "";
  const singerName = Array.isArray(s.SingerName) ? s.SingerName[0] : s.singername ?? s.SingerName ?? "";
  const image = String(s.imgurl ?? s.ImgUrl ?? s.Image ?? "").replace("{size}", "400");
  return {
    id: `kgtrack_${hash}`,
    title: (s.songname ?? s.SongName ?? s.FileName ?? "") as string,
    artist: String(singerName),
    artist_id: `kgartist_${singerId}`,
    album: (s.album_name ?? s.AlbumName ?? "") as string,
    album_id: `kgalbum_${albumId}`,
    img_url: image,
    source: "kugou",
    source_url: `https://www.kugou.com/song/#hash=${hash}&album_id=${albumId}`,
    lyric_url: hash,
  };
}

export const kugou = {
  async search(url: string): Promise<SearchResult> {
    const keyword = getParameterByName("keywords", url);
    const curpage = Number(getParameterByName("curpage", url));
    const searchType = getParameterByName("type", url);
    if (searchType === "1") {
      const resp = await get<{ data?: { info?: Array<{ specialid: number; specialname: string; imgurl?: string; nickname?: string; songcount?: number }>; total?: number } }>(
        `http://mobilecdnbj.kugou.com/api/v3/search/special?keyword=${encodeURIComponent(keyword)}&pagesize=20&filter=0&page=${curpage}`
      );
      return {
        result: (resp.data.data?.info ?? []).map((item) => ({
          id: `kgplaylist_${item.specialid}`,
          title: item.specialname,
          artist: item.nickname ?? "",
          source: "kugou",
          source_url: `https://www.kugou.com/yy/special/single/${item.specialid}.html`,
          img_url: item.imgurl ? item.imgurl.replace("{size}", "400") : "",
          url: `kgplaylist_${item.specialid}`,
        })),
        total: resp.data.data?.total ?? 0,
        type: "playlist",
      };
    }
    const targetUrl =
      `https://songsearch.kugou.com/song_search_v2?keyword=${encodeURIComponent(keyword)}&page=${curpage}&pagesize=20&showtype=1`;
    const resp = await get<{ data?: { lists?: Record<string, unknown>[]; total?: number } }>(targetUrl);
    const baseTracks = (resp.data.data?.lists ?? []).map(convertSong).filter((track) => track.id !== "kgtrack_");
    const result = await Promise.all(baseTracks.map(async (track) => {
      const detail = await get<{ data?: { img?: string } }>(
        `https://www.kugou.com/yy/index.php?r=play/getdata&hash=${encodeURIComponent(track.lyric_url ?? track.id.replace("kgtrack_", ""))}`
      ).catch(() => null);
      return { ...track, img_url: detail?.data.data?.img ?? track.img_url };
    }));
    return { result, total: resp.data.data?.total ?? result.length, type: "song" };
  },

  async show_playlist(url: string): Promise<{ result: PlaylistInfo[] }> {
    const offset = Number(getParameterByName("offset", url)) || 0;
    const filterId = getParameterByName("filter_id", url) || "0";
    const targetUrl =
      `https://m.kugou.com/plist/index?json=true&page=${Math.floor(offset / 30) + 1}&tagid=${filterId}`;
    const resp = await get<{ plist: { list: { info: Array<{ imgurl: string; specialname: string; specialid: number }> } } }>(targetUrl);
    const result = (resp.data.plist?.list?.info ?? []).map((item) => ({
      cover_img_url: item.imgurl?.replace("{size}", "400"),
      title: item.specialname,
      id: `kgplaylist_${item.specialid}`,
      source_url: `https://www.kugou.com/yy/special/single/${item.specialid}.html`,
    }));
    return { result };
  },

  async get_playlist(url: string): Promise<Playlist> {
    const listId = getParameterByName("list_id", url);
    const prefix = listId.split("_")[0];

    if (prefix === "kgalbum") return this._getAlbum(listId);
    if (prefix === "kgartist") return this._getArtist(listId);
    return this._getPlaylist(listId);
  },

  async _getPlaylist(listId: string): Promise<Playlist> {
    const id = listId.split("_").pop();
    const resp = await get<{ data: { info: { specialname: string; imgurl: string }; list: { info: Record<string, unknown>[] } } }>(
      `https://m.kugou.com/plist/list/${id}?json=true`
    );
    const data = resp.data.data;
    const info = data?.info;
    return {
      info: {
        id: `kgplaylist_${id}`,
        title: info?.specialname ?? "酷狗歌单",
        cover_img_url: info?.imgurl?.replace("{size}", "400") ?? "",
        source_url: `https://www.kugou.com/yy/special/single/${id}.html`,
      },
      tracks: (data?.list?.info ?? []).map(convertSong),
    };
  },

  async _getAlbum(listId: string): Promise<Playlist> {
    const id = listId.split("_").pop();
    const resp = await get<{ data: { info: { album_name: string; imgurl: string }; list: Record<string, unknown>[] } }>(
      `https://mobilecdn.kugou.com/api/v3/album/song?albumid=${id}&page=1&pagesize=200&format=json`
    );
    return {
      info: { id: `kgalbum_${id}`, title: resp.data.data.info.album_name, cover_img_url: resp.data.data.info.imgurl },
      tracks: (resp.data.data.list ?? []).map(convertSong),
    };
  },

  async _getArtist(listId: string): Promise<Playlist> {
    const id = listId.split("_").pop();
    const resp = await get<{ data: { info: { singername: string; imgurl: string }; tab: number; list: Record<string, unknown>[] } }>(
      `https://mobilecdn.kugou.com/api/v3/singer/song?singerid=${id}&page=1&pagesize=50&format=json`
    );
    return {
      info: { id: `kgartist_${id}`, title: resp.data.data.info.singername, cover_img_url: resp.data.data.info.imgurl },
      tracks: (resp.data.data.list ?? []).map(convertSong),
    };
  },

  async get_song_url(trackId: string): Promise<UrlResult | null> {
    const hash = trackId.replace("kgtrack_", "");
    const resp = await get<{ url?: string; bitRate?: number }>(
      `https://m.kugou.com/app/i/getSongInfo.php?cmd=playInfo&hash=${encodeURIComponent(hash)}`
    );
    if (!resp.data.url) return null;
    return { url: resp.data.url, bitrate: resp.data.bitRate ? `${resp.data.bitRate}kbps` : undefined, platform: "kugou" };
  },

  async lyric(url: string): Promise<LyricResult> {
    const trackId = getParameterByName("track_id", url).replace("kgtrack_", "");
    const albumId = getParameterByName("album_id", url).replace("kgalbum_", "");
    const resp = await get<string>(
      `https://wwwapi.kugou.com/yy/index.php?r=play/getdata&callback=jQuery&mid=1&hash=${encodeURIComponent(trackId)}&platid=4&album_id=${encodeURIComponent(albumId)}&_=${Date.now()}`
    );
    const text = resp.text ?? String(resp.data ?? "");
    const json = text.replace(/^jQuery\(/, "").replace(/\);?\s*$/, "");
    const data = JSON.parse(json) as { data?: { lyrics?: string } };
    return { lyric: data.data?.lyrics ?? "", tlyric: "" };
  },

  async parse_url(url: string): Promise<PlaylistInfo | null> {
    if (url.includes("kugou.com/yy/special/single/")) {
      const match = /\/single\/(\d+)\.html/.exec(url);
      if (match) return { id: `kgplaylist_${match[1]}`, title: "" };
    }
    return null;
  },

  get_playlist_filters(): PlaylistFilter[] {
    return [
      {
        id: "kugou",
        name: "酷狗",
        items: [
          { id: "0", name: "全部" },
          { id: "6", name: "华语" },
          { id: "16", name: "欧美" },
          { id: "8", name: "日韩" },
          { id: "2", name: "古典" },
          { id: "3", name: "流行" },
          { id: "40", name: "摇滚" },
          { id: "20", name: "影视" },
          { id: "32", name: "游戏" },
        ],
      },
    ];
  },
};
