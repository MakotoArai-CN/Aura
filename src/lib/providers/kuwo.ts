import { get, getBackendCookie, type HttpResponse } from "../tauri";
import { getParameterByName } from "./utils";
import type { SearchResult, Playlist, PlaylistInfo, LyricResult, UrlResult, PlaylistFilter } from "./types";
import type { Track } from "../stores/player";

const KUWO_TOKEN_COOKIE = "Hm_Iuvt_cdb524f42f23cer9b268564v7y735ewrq2324";

function htmlDecode(value: unknown): string {
  const text = String(value ?? "");
  if (typeof document === "undefined") return text;
  const textarea = document.createElement("textarea");
  textarea.innerHTML = text;
  return textarea.value;
}

function convertSong(s: Record<string, unknown>): Track {
  const rid = String(s.musicrid ?? s.rid ?? s.DC_TARGETID ?? "").replace(/^MUSIC_/, "");
  return {
    id: `kwtrack_${rid}`,
    title: htmlDecode(s.name ?? s.NAME),
    artist: htmlDecode(s.artist ?? s.ARTIST),
    artist_id: `kwartist_${s.artistid ?? s.ARTISTID ?? ""}`,
    album: htmlDecode(s.album ?? s.ALBUM),
    album_id: `kwalbum_${s.albumid ?? s.ALBUMID ?? ""}`,
    img_url: String(s.pic ?? s.albumpic ?? (s.web_albumpic_short ? `https://img2.kuwo.cn/star/albumcover/${s.web_albumpic_short}` : "")),
    source: "kuwo",
    source_url: `https://www.kuwo.cn/play_detail/${rid}`,
    lyric_url: rid,
  };
}

function kuwoSecret(message: string, password: string): string {
  if (!password) return "";
  let seed = "";
  for (let i = 0; i < password.length; i++) seed += password.charCodeAt(i).toString();
  const step = Math.floor(seed.length / 5);
  const multiplier = parseInt(seed.charAt(step) + seed.charAt(2 * step) + seed.charAt(3 * step) + seed.charAt(4 * step) + seed.charAt(5 * step), 10);
  const increment = Math.ceil(password.length / 2);
  const modulo = Math.pow(2, 31) - 1;
  if (multiplier < 2) return "";
  let salt = Math.round(1e9 * Math.random()) % 1e8;
  seed += salt;
  while (seed.length > 10) seed = (parseInt(seed.substring(0, 10), 10) + parseInt(seed.substring(10), 10)).toString();
  let state = (multiplier * parseInt(seed, 10) + increment) % modulo;
  let encrypted = "";
  for (let i = 0; i < message.length; i++) {
    const code = message.charCodeAt(i) ^ Math.floor((state / modulo) * 255);
    encrypted += code < 16 ? `0${code.toString(16)}` : code.toString(16);
    state = (multiplier * state + increment) % modulo;
  }
  let saltHex = salt.toString(16);
  while (saltHex.length < 8) saltHex = `0${saltHex}`;
  return encrypted + saltHex;
}

async function kuwoHeaders(retry = false): Promise<Record<string, string>> {
  let token = await getBackendCookie("https://www.kuwo.cn", KUWO_TOKEN_COOKIE).catch(() => null);
  if (!token && !retry) {
    await get("https://www.kuwo.cn/").catch(() => null);
    token = await getBackendCookie("https://www.kuwo.cn", KUWO_TOKEN_COOKIE).catch(() => null);
  }
  const secret = token ? kuwoSecret(token, KUWO_TOKEN_COOKIE) : "";
  return secret ? { Secret: secret, "Secret-Version": "1" } : {};
}

async function kuwoGet<T>(url: string): Promise<HttpResponse<T>> {
  let resp = await get<T>(url, await kuwoHeaders());
  if ((resp.data as { success?: boolean })?.success === false) {
    await get("https://www.kuwo.cn/").catch(() => null);
    resp = await get<T>(url, await kuwoHeaders(true));
  }
  return resp;
}

function kuwoTimestamp(value: unknown) {
  const time = Number(value);
  if (!Number.isFinite(time) || time < 0) return "00:00.00";
  const minutes = Math.floor(time / 60);
  const seconds = Math.floor(time - minutes * 60);
  const centiseconds = Math.floor((time - minutes * 60 - seconds) * 100);
  return `${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}.${String(centiseconds).padStart(2, "0")}`;
}

function kuwoLyricFromList(list: Array<{ time?: unknown; lineLyric?: unknown }> | null | undefined) {
  if (!Array.isArray(list)) return "";
  return list
    .filter((item) => item && item.lineLyric !== "//")
    .map((item) => `[${kuwoTimestamp(item.time)}]${htmlDecode(item.lineLyric)}`)
    .join("\n");
}

export const kuwo = {
  async search(url: string): Promise<SearchResult> {
    const keyword = getParameterByName("keywords", url);
    const curpage = Number(getParameterByName("curpage", url));
    const searchType = getParameterByName("type", url);
    if (searchType === "1") {
      const resp = await kuwoGet<{ data?: { list?: Array<{ id: number; name: string; img: string; uname?: string; total?: number }>; total?: number } }>(
        `https://www.kuwo.cn/api/www/search/searchPlayListBykeyWord?key=${encodeURIComponent(keyword)}&pn=${curpage}&rn=30`
      );
      return {
        result: (resp.data.data?.list ?? []).map((item) => ({
          id: `kwplaylist_${item.id}`,
          title: htmlDecode(item.name),
          artist: htmlDecode(item.uname ?? ""),
          source: "kuwo",
          source_url: `https://www.kuwo.cn/playlist_detail/${item.id}`,
          img_url: item.img,
          url: `kwplaylist_${item.id}`,
        })),
        total: resp.data.data?.total ?? 0,
        type: "playlist",
      };
    }
    const pn = Math.max(0, curpage - 1);
    const resp = await kuwoGet<{ abslist?: Record<string, unknown>[]; HIT?: string | number; data?: { list?: Record<string, unknown>[] }; total?: number }>(
      `https://www.kuwo.cn/search/searchMusicBykeyWord?vipver=1&client=kt&ft=music&cluster=0&strategy=2012&encoding=utf8&rformat=json&mobi=1&issubtitle=1&show_copyright_off=1&pn=${pn}&rn=20&all=${encodeURIComponent(keyword)}`
    );
    const list = resp.data.abslist ?? resp.data.data?.list ?? [];
    return { result: list.map(convertSong), total: Number(resp.data.HIT ?? resp.data.total ?? 0), type: "song" };
  },

  async show_playlist(url: string): Promise<{ result: PlaylistInfo[] }> {
    const offset = Number(getParameterByName("offset", url)) || 0;
    const filterId = getParameterByName("filter_id", url) || "";
    const pageSize = 25;
    const page = Math.floor(offset / pageSize) + 1;
    const targetUrl = filterId === "toplist"
      ? `https://www.kuwo.cn/api/pc/bang/list?pn=${page}&rn=${pageSize}&httpsStatus=1`
      : filterId
        ? `https://www.kuwo.cn/api/pc/classify/playlist/getTagPlayList?pn=${page}&rn=${pageSize}&id=${filterId}&httpsStatus=1`
        : `https://www.kuwo.cn/api/pc/classify/playlist/getRcmPlayList?pn=${page}&rn=${pageSize}&order=hot&httpsStatus=1`;

    const resp = await get<{ data?: { data?: Array<{ img?: string; pic?: string; pic300?: string; name: string; id: number | string }> } }>(targetUrl);
    const result = (resp.data.data?.data ?? []).map((item) => ({
      cover_img_url: item.img ?? item.pic ?? item.pic300 ?? "",
      title: item.name,
      id: filterId === "toplist" ? `kwtoplist_${item.id}` : `kwplaylist_${item.id}`,
      source_url: filterId === "toplist" ? `https://www.kuwo.cn/rankList` : `https://www.kuwo.cn/playlist_detail/${item.id}`,
    }));
    return { result };
  },

  async get_playlist(url: string): Promise<Playlist> {
    const listId = getParameterByName("list_id", url);
    const prefix = listId.split("_")[0];

    if (prefix === "kwalbum") return this._getAlbum(listId);
    if (prefix === "kwartist") return this._getArtist(listId);
    if (prefix === "kwtoplist") return this._getToplist(listId);
    return this._getPlaylist(listId);
  },

  async _getPlaylist(listId: string): Promise<Playlist> {
    const id = listId.split("_").pop();
    const infoResp = await kuwoGet<{ data: { info: { pic300: string; name: string; total?: number }; musicList: Record<string, unknown>[] } }>(
      `https://www.kuwo.cn/api/www/playlist/playListInfo?pid=${id}&pn=1&rn=1000&mobi=1`,
    );
    const data = infoResp.data.data;
    const info = data?.info;
    return {
      info: { id: `kwplaylist_${id}`, title: info?.name ?? "酷我歌单", cover_img_url: info?.pic300 ?? "" },
      tracks: (data?.musicList ?? []).map(convertSong),
    };
  },

  async get_playlist_full(url: string): Promise<Playlist> {
    const listId = getParameterByName("list_id", url);
    const prefix = listId.split("_")[0];
    if (prefix !== "kwplaylist" && prefix !== "kwalbum") return this.get_playlist(url);
    const id = listId.split("_").pop();
    if (prefix === "kwplaylist") {
      const infoResp = await get<{ pic?: string; title?: string; id?: string | number; total?: number }>(
        `https://nplserver.kuwo.cn/pl.svc?op=getlistinfo&pn=0&rn=0&encode=utf-8&keyset=pl2012&pcmp4=1&pid=${id}&vipver=MUSIC_9.0.2.0_W1&newver=1`
      ).catch(() => null);
      const total = Number(infoResp?.data.total ?? 0);
      const pageSize = 100;
      const pageCount = Math.max(1, total ? Math.ceil(total / pageSize) : 1);
      const pages = await Promise.all(Array.from({ length: pageCount }, (_, i) =>
        kuwoGet<{ data?: { musicList?: Record<string, unknown>[]; info?: { name?: string; pic300?: string } } }>(
          `https://www.kuwo.cn/api/www/playlist/playListInfo?pid=${id}&pn=${i + 1}&rn=${pageSize}&httpsStatus=1`
        ).catch(() => null)
      ));
      const firstInfo = pages[0]?.data.data?.info;
      return {
        info: {
          id: `kwplaylist_${id}`,
          title: infoResp?.data.title ?? firstInfo?.name ?? "酷我歌单",
          cover_img_url: (infoResp?.data.pic ?? firstInfo?.pic300 ?? "").replace("_150.jpg", "_400.jpg"),
          source_url: `https://www.kuwo.cn/playlist_detail/${id}`,
        },
        tracks: pages.flatMap((resp) => (resp?.data.data?.musicList ?? []).map(convertSong)),
      };
    }
    const resp = await kuwoGet<{ data: { albumInfo: { pic300: string; name: string }; musicList: Record<string, unknown>[] } }>(
      `https://www.kuwo.cn/api/www/album/albumInfo?albumId=${id}&pn=1&rn=500&httpsStatus=1`
    );
    return {
      info: { id: `kwalbum_${id}`, title: resp.data.data.albumInfo.name, cover_img_url: resp.data.data.albumInfo.pic300 },
      tracks: (resp.data.data.musicList ?? []).map(convertSong),
    };
  },

  async _getToplist(listId: string): Promise<Playlist> {
    const id = listId.split("_").pop();
    const resp = await kuwoGet<{ data: { musicList: Record<string, unknown>[]; name: string; pic300: string } }>(
      `https://www.kuwo.cn/api/www/bang/bang/musicList?bangId=${id}&pn=1&rn=200&mobi=1`,
    );
    return {
      info: { id: `kwtoplist_${id}`, title: resp.data.data.name, cover_img_url: resp.data.data.pic300 },
      tracks: (resp.data.data.musicList ?? []).map(convertSong),
    };
  },

  async _getAlbum(listId: string): Promise<Playlist> {
    const id = listId.split("_").pop();
    const resp = await kuwoGet<{ data: { albumInfo: { pic300: string; name: string }; musicList: Record<string, unknown>[] } }>(
      `https://www.kuwo.cn/api/www/album/albumInfo?albumId=${id}&pn=1&rn=200&mobi=1`,
    );
    return {
      info: { id: `kwalbum_${id}`, title: resp.data.data.albumInfo.name, cover_img_url: resp.data.data.albumInfo.pic300 },
      tracks: (resp.data.data.musicList ?? []).map(convertSong),
    };
  },

  async _getArtist(listId: string): Promise<Playlist> {
    const id = listId.split("_").pop();
    const resp = await kuwoGet<{ data: { artistInfo: { pic300: string; name: string }; list: Record<string, unknown>[] } }>(
      `https://www.kuwo.cn/api/www/artist/artistMusic?artistid=${id}&pn=1&rn=100&mobi=1`,
    );
    return {
      info: { id: `kwartist_${id}`, title: resp.data.data.artistInfo.name, cover_img_url: resp.data.data.artistInfo.pic300 },
      tracks: (resp.data.data.list ?? []).map(convertSong),
    };
  },

  async get_song_url(trackId: string): Promise<UrlResult | null> {
    const rid = trackId.replace("kwtrack_", "");
    const resp = await kuwoGet<{ data?: { url?: string }; url?: string }>(
      `https://www.kuwo.cn/api/v1/www/music/playUrl?mid=${rid}&type=music&httpsStatus=1&reqId=&plat=web_www&from=`
    );
    const url = resp.data.data?.url ?? resp.data.url ?? "";
    if (!url) return null;
    return { url, platform: "kuwo" };
  },

  async lyric(url: string): Promise<LyricResult> {
    const trackId = (getParameterByName("lyric_url", url) || getParameterByName("track_id", url)).replace("kwtrack_", "");
    if (!trackId) return { lyric: "", tlyric: "" };
    const resp = await kuwoGet<{
      status?: number;
      data?: string | {
        lrclist?: Array<{ time?: unknown; lineLyric?: unknown }>;
        lrc?: string;
        lyric?: string;
      };
    }>(`https://m.kuwo.cn/newh5/singles/songinfoandlrc?musicId=${trackId}`);
    const data = resp.data.data;
    if (typeof data === "string") return { lyric: data, tlyric: "" };
    return { lyric: kuwoLyricFromList(data?.lrclist) || data?.lrc || data?.lyric || "", tlyric: "" };
  },

  async parse_url(url: string): Promise<PlaylistInfo | null> {
    const normalized = url
      .replace(/kuwo\.cn\/(h5app|newh5(?:app)?)\//, "kuwo.cn/")
      .replace(/kuwo\.cn\/(album\/|\?albumid=)/, "kuwo.cn/album_detail/")
      .replace(/kuwo\.cn\/(artist|singers)\//, "kuwo.cn/singer_detail/")
      .replace(/kuwo\.cn\/playlist\//, "kuwo.cn/playlist_detail/");
    if (normalized.includes("kuwo.cn/playlist_detail/")) {
      const match = /\/playlist_detail\/(\d+)/.exec(normalized);
      if (match) return { id: `kwplaylist_${match[1]}`, title: "" };
    }
    if (normalized.includes("kuwo.cn/singer_detail/")) {
      const match = /\/singer_detail\/(\d+)/.exec(normalized);
      if (match) return { id: `kwartist_${match[1]}`, title: "" };
    }
    if (normalized.includes("kuwo.cn/album_detail/")) {
      const match = /\/album_detail\/(\d+)/.exec(normalized);
      if (match) return { id: `kwalbum_${match[1]}`, title: "" };
    }
    return null;
  },

  get_playlist_filters(): PlaylistFilter[] {
    return [
      {
        id: "kuwo",
        name: "酷我",
        items: [
          { id: "", name: "全部" },
          { id: "toplist", name: "排行榜" },
          { id: "37", name: "华语" },
          { id: "35", name: "欧美" },
          { id: "1877", name: "二次元" },
          { id: "393", name: "流行" },
          { id: "389", name: "摇滚" },
          { id: "577", name: "纯音乐" },
          { id: "180", name: "影视" },
        ],
      },
    ];
  },
};
