import { get, post, getBackendCookie, getLoginCookies } from "../tauri";
import { getParameterByName } from "./utils";
import type { SearchResult, Playlist, PlaylistInfo, LyricResult, UrlResult, PlaylistFilter, UserStatus, UserPlaylistResult } from "./types";
import type { Track } from "../stores/player";

function htmlDecode(value: string): string {
  const parser = new DOMParser();
  return parser.parseFromString(value, "text/html").body.textContent ?? value;
}

function unicodeToAscii(value: string): string {
  return value.replace(/&#(\d+);/g, (_, code) => String.fromCharCode(Number(code)));
}

function getImageUrl(mid: string | null | undefined, imgType: "artist" | "album"): string {
  if (!mid) return "";
  const cat = imgType === "artist" ? "T001R300x300M000" : "T002R300x300M000";
  return `https://y.gtimg.cn/music/photo_new/${cat}${mid}.jpg`;
}

function isPlayable(song: { switch: number }): boolean {
  const flags = song.switch.toString(2).split("").reverse();
  return flags[0] === "1";
}

function normalizeUin(value: string): string {
  const raw = value.trim();
  if (!raw) return "";
  if (/^o\d+$/i.test(raw)) return raw.slice(1);
  if (/^u\d+$/i.test(raw)) return raw.slice(1);
  return raw.replace(/\D/g, "");
}

function convertSong(song: Record<string, unknown>): Track {
  const singers = (song.singer ?? []) as Array<{ name?: string; mid?: string }>;
  const album = (song.album ?? {}) as { name?: string; mid?: string };
  const mid = String(song.songmid ?? song.mid ?? "");
  const albumMid = String(song.albummid ?? album?.mid ?? "");
  return {
    id: `qqtrack_${mid}`,
    title: htmlDecode(String(song.songname ?? song.name ?? "")),
    artist: htmlDecode(singers[0]?.name ?? ""),
    artist_id: `qqartist_${singers[0]?.mid}`,
    album: htmlDecode(album?.name ?? ""),
    album_id: `qqalbum_${albumMid}`,
    img_url: getImageUrl(albumMid, "album"),
    source: "qq",
    source_url: `https://y.qq.com/#type=song&mid=${mid}`,
    url: song.switch !== undefined && !isPlayable(song as { switch: number }) ? "" : undefined,
  };
}

function convertSmartboxSong(song: Record<string, unknown>): Track {
  const mid = String(song.mid ?? "");
  const name = String(song.name ?? "");
  const singer = String(song.singer ?? "");
  return {
    id: `qqtrack_${mid}`,
    title: htmlDecode(name),
    artist: htmlDecode(singer),
    artist_id: "",
    album: "",
    album_id: "",
    img_url: "",
    source: "qq",
    source_url: `https://y.qq.com/#type=song&mid=${mid}`,
  };
}

export const qq = {
  async show_playlist(url: string): Promise<{ result: PlaylistInfo[] }> {
    const offset = Number(getParameterByName("offset", url)) || 0;
    let filterId = getParameterByName("filter_id", url) || "10000000";

    if (filterId === "toplist") {
      if (offset > 0) return { result: [] };
      const resp = await get<{ data: { topList: Array<{ picUrl: string; id: number; topTitle: string }> } }>(
        "https://c.y.qq.com/v8/fcg-bin/fcg_myqq_toplist.fcg?g_tk=5381&inCharset=utf-8&outCharset=utf-8&notice=0&format=json&uin=0&needNewCode=1&platform=h5"
      );
      return {
        result: resp.data.data.topList.map((item) => ({
          cover_img_url: item.picUrl,
          id: `qqtoplist_${item.id}`,
          source_url: `https://y.qq.com/n/yqq/toplist/${item.id}.html`,
          title: item.topTitle,
        })),
      };
    }

    const targetUrl =
      `https://c.y.qq.com/splcloud/fcgi-bin/fcg_get_diss_by_tag.fcg` +
      `?picmid=1&rnd=${Math.random()}&g_tk=732560869` +
      `&loginUin=0&hostUin=0&format=json&inCharset=utf8&outCharset=utf-8` +
      `&notice=0&platform=yqq.json&needNewCode=0` +
      `&categoryId=${filterId}&sortId=5&sin=${offset}&ein=${29 + offset}`;

    const resp = await get<{ data: { list: Array<{ imgurl: string; dissname: string; dissid: number }> } }>(targetUrl);
    return {
      result: resp.data.data.list.map((item) => ({
        cover_img_url: item.imgurl,
        title: htmlDecode(item.dissname),
        id: `qqplaylist_${item.dissid}`,
        source_url: `https://y.qq.com/n/ryqq/playlist/${item.dissid}`,
      })),
    };
  },

  async search(url: string): Promise<SearchResult> {
    const keyword = getParameterByName("keywords", url);
    const curpage = Number(getParameterByName("curpage", url));
    const searchType = getParameterByName("type", url);

    const typeMap: Record<string, number> = { "0": 0, "1": 3 };
    const limit = 50;
    const body = {
      comm: { ct: "19", cv: "1859", uin: "0" },
      req: {
        method: "DoSearchForQQMusicDesktop",
        module: "music.search.SearchCgiService",
        param: {
          grp: 1,
          query: keyword,
          num_per_page: limit,
          page_num: curpage,
          search_type: typeMap[searchType] ?? 0,
        },
      },
    };

    const resp = await post<{ req: { data: { body: { song?: { list?: Record<string, unknown>[] }; songlist?: { list?: Array<{ dissid: string | number; dissname: string; imgurl?: string; creator?: { name?: string }; song_count?: number }> } }; meta?: { sum?: number } } } }>(
      "https://u.y.qq.com/cgi-bin/musicu.fcg",
      body
    );

    const bodyData = resp.data.req?.data?.body;
    if (searchType === "1") {
      const playlists = bodyData?.songlist?.list ?? [];
      return {
        result: playlists.map((info) => ({
          id: `qqplaylist_${info.dissid}`,
          title: htmlDecode(info.dissname),
          artist: unicodeToAscii(info.creator?.name ?? ""),
          source: "qq",
          source_url: `https://y.qq.com/n/ryqq/playlist/${info.dissid}`,
          img_url: info.imgurl ?? "",
          url: `qqplaylist_${info.dissid}`,
        })),
        total: resp.data.req?.data?.meta?.sum ?? playlists.length,
        type: "playlist",
      };
    }
    let songs = bodyData?.song?.list ?? [];
    if (songs.length === 0) {
      const smartResp = await get<{ data?: { song?: { count?: number; itemlist?: Record<string, unknown>[] } } }>(
        `https://c6.y.qq.com/splcloud/fcgi-bin/smartbox_new.fcg?format=json&key=${encodeURIComponent(keyword)}&inCharset=utf8&outCharset=utf-8`
      ).catch(() => null);
      const smartSongs = smartResp?.data.data?.song?.itemlist ?? [];
      return {
        result: smartSongs.map(convertSmartboxSong).filter((track) => track.id !== "qqtrack_"),
        total: smartResp?.data.data?.song?.count ?? smartSongs.length,
        type: "song",
      };
    }
    return {
      result: songs.map((s) => convertSong(s)).filter((track) => track.id !== "qqtrack_"),
      total: resp.data.req?.data?.meta?.sum ?? songs.length,
      type: "song",
    };
  },

  async get_playlist(url: string): Promise<Playlist> {
    const listId = getParameterByName("list_id", url);
    const prefix = listId.split("_")[0];

    if (prefix === "qqalbum") return this._getAlbum(listId);
    if (prefix === "qqartist") return this._getArtist(listId);
    if (prefix === "qqtoplist") return this._getToplist(listId);
    return this._getPlaylist(listId);
  },

  async _getPlaylist(listId: string): Promise<Playlist> {
    const id = listId.split("_").pop();
    const targetUrl =
      `https://i.y.qq.com/qzone-music/fcg-bin/fcg_ucc_getcdinfo_byids_cp.fcg` +
      `?type=1&json=1&utf8=1&onlysong=0&nosign=1&disstid=${id}` +
      `&g_tk=5381&loginUin=0&hostUin=0&format=json&inCharset=GB2312&outCharset=utf-8&platform=yqq`;

    const resp = await get<{ cdlist?: Array<{ logo: string; dissname: string; songlist: Record<string, unknown>[] }> }>(targetUrl);
    const pl = resp.data.cdlist?.[0];
    return {
      info: { id: `qqplaylist_${id}`, title: pl?.dissname ?? "QQ音乐歌单", cover_img_url: pl?.logo ?? "", source_url: `https://y.qq.com/n/ryqq/playlist/${id}` },
      tracks: (pl?.songlist ?? []).map(convertSong),
    };
  },

  async _getAlbum(listId: string): Promise<Playlist> {
    const id = listId.split("_").pop();
    const resp = await get<{ data: { name: string; list: Record<string, unknown>[] } }>(
      `https://i.y.qq.com/v8/fcg-bin/fcg_v8_album_info_cp.fcg?platform=h5page&albummid=${id}&format=json&outCharset=utf-8`
    );
    return {
      info: { id: `qqalbum_${id}`, title: resp.data.data.name, cover_img_url: getImageUrl(id ?? "", "album"), source_url: `https://y.qq.com/#type=album&mid=${id}` },
      tracks: resp.data.data.list.map(convertSong),
    };
  },

  async _getArtist(listId: string): Promise<Playlist> {
    const id = listId.split("_").pop();
    const body = JSON.stringify({ comm: { ct: 24, cv: 0 }, singer: { method: "get_singer_detail_info", param: { sort: 5, singermid: id, sin: 0, num: 50 }, module: "music.web_singer_info_svr" } });
    const resp = await get<{ singer: { data: { singer_info: { name: string }; songlist: Record<string, unknown>[] } } }>(
      `https://u.y.qq.com/cgi-bin/musicu.fcg?format=json&data=${encodeURIComponent(body)}`
    );
    return {
      info: { id: `qqartist_${id}`, title: resp.data.singer.data.singer_info.name, cover_img_url: getImageUrl(id ?? "", "artist"), source_url: `https://y.qq.com/#type=singer&mid=${id}` },
      tracks: resp.data.singer.data.songlist.map(convertSong),
    };
  },

  async _getToplist(listId: string): Promise<Playlist> {
    const id = listId.split("_").pop();
    const body = JSON.stringify({ comm: { cv: 1602, ct: 20 }, toplist: { module: "musicToplist.ToplistInfoServer", method: "GetDetail", param: { topid: Number(id), num: 100 } } });
    const resp = await get<{ toplist: { data: { data: { frontPicUrl: string; title: string }; songInfoList: Record<string, unknown>[] } } }>(
      `https://u.y.qq.com/cgi-bin/musicu.fcg?format=json&data=${encodeURIComponent(body)}`
    );
    const tl = resp.data.toplist.data;
    return {
      info: { id: `qqtoplist_${id}`, title: tl.data.title, cover_img_url: tl.data.frontPicUrl, source_url: `https://y.qq.com/n/yqq/toplist/${id}.html` },
      tracks: tl.songInfoList.map(convertSong),
    };
  },

  async get_song_url(trackId: string): Promise<UrlResult | null> {
    const mid = trackId.replace("qqtrack_", "");
    const guid = "10000";
    const fileType = "128";
    const fileConfig: Record<string, { prefix: string; ext: string; bitrate: string }> = {
      m4a: { prefix: "C400", ext: ".m4a", bitrate: "M4A" },
      "128": { prefix: "M500", ext: ".mp3", bitrate: "128kbps" },
      "320": { prefix: "M800", ext: ".mp3", bitrate: "320kbps" },
      ape: { prefix: "A000", ext: ".ape", bitrate: "APE" },
      flac: { prefix: "F000", ext: ".flac", bitrate: "FLAC" },
    };
    const fileInfo = fileConfig[fileType];
    const filename = `${fileInfo.prefix}${mid}${mid}${fileInfo.ext}`;
    const body = {
      loginUin: "0",
      comm: { uin: "0", format: "json", ct: 24, cv: 0 },
      req_1: {
        module: "vkey.GetVkeyServer",
        method: "CgiGetVkey",
        param: { filename: [filename], guid, songmid: [mid], songtype: [0], uin: "0", loginflag: 1, platform: "20" },
      },
    };
    const resp = await post<{ req_1: { data: { midurlinfo: Array<{ purl: string }>; sip: string[] } } }>(
      "https://u.y.qq.com/cgi-bin/musicu.fcg",
      body
    );
    const purl = resp.data.req_1?.data?.midurlinfo[0]?.purl;
    if (!purl) return null;
    const sip = resp.data.req_1?.data?.sip[0] ?? "https://dl.stream.qqmusic.qq.com/";
    const prefix = purl.slice(0, 4);
    const bitrate = Object.values(fileConfig).find((item) => item.prefix === prefix)?.bitrate;
    return { url: sip + purl, platform: "qq", bitrate };
  },

  async lyric(url: string): Promise<LyricResult> {
    const trackId = getParameterByName("track_id", url).replace("qqtrack_", "");
    const targetUrl = `https://c.y.qq.com/lyric/fcgi-bin/fcg_query_lyric_new.fcg?songmid=${trackId}&g_tk=5381&format=json&inCharset=utf-8&outCharset=utf-8`;
    const resp = await get<{ lyric: string; trans: string }>(targetUrl);
    const decode = (s: string) => s ? atob(s) : "";
    return { lyric: decode(resp.data.lyric), tlyric: decode(resp.data.trans) };
  },

  async get_user_by_uin(uin: string): Promise<UserStatus> {
    const body = JSON.stringify({
      comm: { ct: 24, cv: 0 },
      vip: {
        module: "userInfo.VipQueryServer",
        method: "SRFVipQuery_V2",
        param: { uin_list: [uin] },
      },
      base: {
        module: "userInfo.BaseUserInfoServer",
        method: "get_user_baseinfo_v2",
        param: { vec_uin: [uin] },
      },
    });
    const resp = await get<{
      base?: {
        data?: {
          map_userinfo?: Record<string, { nick?: string; headurl?: string }>;
        };
      };
    }>(
      `https://u.y.qq.com/cgi-bin/musicu.fcg?format=json&loginUin=${uin}&hostUin=0&inCharset=utf8&outCharset=utf-8&platform=yqq.json&needNewCode=0&data=${encodeURIComponent(body)}`
    );
    const info = resp.data.base?.data?.map_userinfo?.[uin];
    if (!info) return { status: "fail", data: { is_login: false, platform: "qq" } };
    return {
      status: "success",
      data: {
        is_login: true,
        user_id: uin,
        user_name: uin,
        nickname: info.nick ?? uin,
        avatar: info.headurl ?? "",
        platform: "qq",
        data: resp.data,
      },
    };
  },

  async get_user(): Promise<UserStatus> {
    const cookieUrls = [
      "https://y.qq.com",
      "https://u.y.qq.com",
      "https://c.y.qq.com",
      "https://i.y.qq.com",
      "https://qq.com",
      "https://www.qq.com",
    ];
    const cookieNames = ["uin", "wxuin", "p_uin"];
    let uin = "";

    for (const url of cookieUrls) {
      const cookies: Record<string, string> = await getLoginCookies(url).catch(() => ({}));
      for (const name of cookieNames) {
        if (cookies[name]) {
          uin = normalizeUin(cookies[name]);
          break;
        }
      }
      if (uin) break;
    }

    if (!uin) {
      for (const url of cookieUrls) {
        for (const name of cookieNames) {
          const value = await getBackendCookie(url, name).catch(() => null);
          if (value) {
            uin = normalizeUin(value);
            break;
          }
        }
        if (uin) break;
      }
    }

    if (!uin) return { status: "fail", data: { is_login: false, platform: "qq" } };
    return this.get_user_by_uin(uin);
  },

  async get_user_created_playlist(url: string): Promise<UserPlaylistResult> {
    const userId = getParameterByName("user_id", url);
    if (!userId) return { status: "fail", data: { playlists: [] } };
    const size = 100;
    const targetUrl =
      `https://c.y.qq.com/rsc/fcgi-bin/fcg_user_created_diss` +
      `?cv=4747474&ct=24&format=json&inCharset=utf-8&outCharset=utf-8` +
      `&notice=0&platform=yqq.json&needNewCode=1&uin=${userId}&hostuin=${userId}&sin=0&size=${size}`;
    const resp = await get<{
      data?: {
        disslist?: Array<{
          dir_show?: number;
          tid?: number;
          diss_name?: string;
          diss_cover?: string;
        }>;
      };
    }>(targetUrl);
    const playlists = (resp.data.data?.disslist ?? [])
      .flatMap((item) => {
        if (item.dir_show === 0) {
          if (!item.tid || item.diss_name !== "我喜欢") return [];
          return [{
            cover_img_url: "https://y.gtimg.cn/mediastyle/y/img/cover_love_300.jpg",
            id: `qqplaylist_${item.tid}`,
            source_url: `https://y.qq.com/n/ryqq/playlist/${item.tid}`,
            title: item.diss_name,
          }];
        }
        if (!item.tid) return [];
        return [{
          cover_img_url: item.diss_cover ?? "",
          id: `qqplaylist_${item.tid}`,
          source_url: `https://y.qq.com/n/ryqq/playlist/${item.tid}`,
          title: item.diss_name ?? "",
        }];
      });
    return { status: "success", data: { playlists } };
  },

  async get_user_favorite_playlist(url: string): Promise<UserPlaylistResult> {
    const userId = getParameterByName("user_id", url);
    if (!userId) return { status: "fail", data: { playlists: [] } };
    const resp = await get<{
      data?: {
        cdlist?: Array<{
          dir_show?: number;
          logo?: string;
          dissid?: number;
          dissname?: string;
        }>;
      };
    }>(
      `https://c.y.qq.com/fav/fcgi-bin/fcg_get_profile_order_asset.fcg` +
      `?ct=20&cid=205360956&userid=${userId}&reqtype=3&sin=0&ein=100`
    );
    const playlists = (resp.data.data?.cdlist ?? [])
      .filter((item) => item.dir_show !== 0 && item.dissid)
      .map((item) => ({
        cover_img_url: item.logo ?? "",
        id: `qqplaylist_${item.dissid}`,
        source_url: `https://y.qq.com/n/ryqq/playlist/${item.dissid}`,
        title: item.dissname ?? "",
      }));
    return { status: "success", data: { playlists } };
  },

  async parse_url(url: string): Promise<PlaylistInfo | null> {
    if (url.includes("y.qq.com/n/ryqq/playlist/")) {
      const match = /\/playlist\/(\d+)/.exec(url);
      if (match) return { id: `qqplaylist_${match[1]}`, title: "" };
    }
    if (url.includes("y.qq.com/n/yqq/toplist/")) {
      const match = /\/toplist\/(\d+)/.exec(url);
      if (match) return { id: `qqtoplist_${match[1]}`, title: "" };
    }
    return null;
  },

  get_playlist_filters(): PlaylistFilter[] {
    return [
      {
        id: "qq",
        name: "QQ音乐",
        items: [
          { id: "10000000", name: "全部" },
          { id: "toplist", name: "排行榜" },
          { id: "10000002", name: "华语" },
          { id: "10000003", name: "欧美" },
          { id: "10000016", name: "粤语" },
          { id: "10000017", name: "日语" },
          { id: "10000018", name: "韩语" },
          { id: "10000019", name: "小语种" },
          { id: "10000020", name: "运动" },
          { id: "10000005", name: "影视" },
          { id: "10000006", name: "游戏" },
        ],
      },
    ];
  },
};
