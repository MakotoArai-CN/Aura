import { get } from "../tauri";
import { getParameterByName } from "./utils";
import forge from "node-forge";
import type { SearchResult, Playlist, PlaylistInfo, LyricResult, UrlResult, PlaylistFilter, UserPlaylistResult, UserStatus } from "./types";
import type { Track } from "../stores/player";

let wbiKey: { img_key: string; sub_key: string } | null = null;

type BiliView = {
  title?: string;
  pic?: string;
  desc?: string;
  owner?: { name?: string; mid?: number | string };
  pages?: Array<{ cid?: number | string; part?: string; page?: number; first_frame?: string }>;
};

type BiliFavoriteMedia = {
  id?: number | string;
  bvid?: string;
  title?: string;
  cover?: string;
  intro?: string;
  type?: number;
  upper?: { name?: string; mid?: number | string };
};

type BiliFavoritePage = {
  info?: {
    id?: number | string;
    title?: string;
    cover?: string;
    media_count?: number;
    upper?: { name?: string; mid?: number | string };
  };
  medias?: BiliFavoriteMedia[];
  has_more?: boolean;
};

type BiliNavData = {
  isLogin?: boolean;
  mid?: number | string;
  uname?: string;
  face?: string;
};

type BiliFavoriteFolder = {
  id?: number | string;
  fid?: number | string;
  title?: string;
  media_count?: number;
  cover?: string;
};

async function fetchWbiKey() {
  const resp = await get<{ data?: { wbi_img?: { img_url?: string; sub_url?: string } } }>(
    "https://api.bilibili.com/x/web-interface/nav"
  );
  const imgUrl = resp.data?.data?.wbi_img?.img_url ?? "";
  const subUrl = resp.data?.data?.wbi_img?.sub_url ?? "";
  return {
    img_key: imgUrl.slice(imgUrl.lastIndexOf("/") + 1, imgUrl.lastIndexOf(".")),
    sub_key: subUrl.slice(subUrl.lastIndexOf("/") + 1, subUrl.lastIndexOf(".")),
  };
}

async function getWbiKey() {
  if (wbiKey) return wbiKey;
  wbiKey = await fetchWbiKey();
  return wbiKey;
}

async function encWbi(params: Record<string, string | number>): Promise<string> {
  const { img_key, sub_key } = await getWbiKey();
  const mixinKeyEncTab = [46,47,18,2,53,8,23,32,15,50,10,31,58,3,45,35,27,43,5,49,33,9,42,19,29,28,14,39,12,38,41,13,37,48,7,16,24,55,40,61,26,17,0,1,60,51,30,4,22,25,54,21,56,59,6,63,57,62,11,36,20,34,44,52];
  const original = img_key + sub_key;
  let mixinKey = "";
  mixinKeyEncTab.forEach((n) => { mixinKey += original[n] ?? ""; });
  mixinKey = mixinKey.slice(0, 32);

  const currTime = Math.round(Date.now() / 1000);
  const chrFilter = /[!'()*]/g;
  const p: Record<string, string | number> = { ...params, wts: currTime };
  const query = Object.keys(p).sort().map((k) =>
    `${encodeURIComponent(k)}=${encodeURIComponent(String(p[k]).replace(chrFilter, ""))}`
  ).join("&");
  const wbiSign = forge.md5.create().update(forge.util.encodeUtf8(query + mixinKey)).digest().toHex();
  return `${query}&w_rid=${wbiSign}`;
}

function stringValue(value: unknown): string {
  return typeof value === "string" || typeof value === "number" ? String(value) : "";
}

function stripHtml(value: unknown): string {
  const textarea = typeof document !== "undefined" ? document.createElement("textarea") : null;
  const stripped = stringValue(value).replace(/<[^>]+>/g, "").trim();
  if (!textarea) return stripped;
  textarea.innerHTML = stripped;
  return textarea.value;
}

function normalizeImageUrl(value: unknown): string {
  const url = stringValue(value).trim();
  if (!url) return "";
  return url.startsWith("//") ? `https:${url}` : url;
}

function extractBvid(value: unknown): string {
  const text = stringValue(value);
  const match = /(BV[0-9A-Za-z]+)/.exec(text);
  return match?.[1] ?? "";
}

function extractAid(value: unknown): string {
  const text = stringValue(value);
  if (!text) return "";
  const match = /(?:^|[?&/])(?:av|aid=)?(\d{4,})(?:$|[/?&#])?/i.exec(text);
  return match?.[1] ?? (/^\d+$/.test(text) ? text : "");
}

function videoIdentity(s: Record<string, unknown>, fallbackBvid = "") {
  const bvid = extractBvid(s.bvid) || extractBvid(s.arcurl) || fallbackBvid;
  const aid = extractAid(s.aid) || extractAid(s.id) || extractAid(s.arcurl);
  if (bvid) return { id: bvid, bvid, aid };
  if (aid) return { id: `av${aid}`, bvid: "", aid };
  return { id: "", bvid: "", aid: "" };
}

function convertAudioSong(s: Record<string, unknown>): Track {
  const id = stringValue(s.id);
  return {
    id: `bitrack_${id}`,
    title: stripHtml(s.title),
    artist: stringValue(s.uname),
    artist_id: `biartist_${stringValue(s.uid)}`,
    album: "",
    album_id: "",
    source: "bilibili",
    source_url: `https://www.bilibili.com/audio/au${id}`,
    img_url: normalizeImageUrl(s.cover),
    lyric_url: stringValue(s.lyric),
  };
}

function convertVideoSong(s: Record<string, unknown>, bvid?: string): Track {
  const identity = videoIdentity(s, bvid);
  const title = stripHtml(s.title ?? s.part);
  const owner = s.owner as { name?: string; mid?: number | string } | undefined;
  const author = stringValue(s.author) || stringValue(owner?.name);
  const mid = stringValue(owner?.mid) || stringValue(s.mid);
  return {
    id: identity.id ? `bitrack_v_${identity.id}` : "bitrack_v_",
    title,
    artist: author,
    artist_id: mid ? `biartist_v_${mid}` : "",
    album: title,
    album_id: identity.id ? `bialbum_${identity.id}` : "",
    img_url: normalizeImageUrl(s.pic ?? s.pic_url),
    source: "bilibili",
    source_url: identity.bvid
      ? `https://www.bilibili.com/video/${identity.bvid}`
      : identity.aid
        ? `https://www.bilibili.com/video/av${identity.aid}`
        : undefined,
  };
}

function convertFavoriteMedia(media: BiliFavoriteMedia): Track | null {
  const bvid = extractBvid(media.bvid);
  const aid = extractAid(media.id);
  if (!bvid && !aid) return null;
  return convertVideoSong({
    bvid,
    aid,
    title: media.title,
    pic: media.cover,
    desc: media.intro,
    owner: media.upper,
  }, bvid);
}

function convertVideoPage(page: NonNullable<BiliView["pages"]>[number], view: BiliView, videoId: string): Track {
  const cid = stringValue(page.cid);
  const title = (view.pages?.length ?? 0) === 1 ? stringValue(view.title) : stringValue(page.part);
  return {
    id: `bitrack_v_${videoId}-${cid}`,
    title: title || stringValue(view.title),
    artist: stringValue(view.owner?.name),
    artist_id: view.owner?.mid != null ? `biartist_v_${view.owner.mid}` : "",
    album: stringValue(view.title),
    album_id: `bialbum_${videoId}`,
    img_url: normalizeImageUrl(page.first_frame || view.pic),
    source: "bilibili",
    source_url: `https://www.bilibili.com/video/${videoId}${page.page ? `?p=${page.page}` : ""}`,
  };
}

function parseVideoTrackId(trackId: string): { videoId: string; cid: string } | null {
  if (trackId.startsWith("bitrack_v_")) {
    const raw = trackId.slice("bitrack_v_".length);
    const [videoId, cid = ""] = raw.split("-");
    return videoId ? { videoId, cid } : null;
  }

  if (!trackId.startsWith("bitrack_")) return null;
  const raw = trackId.slice("bitrack_".length);
  if (/^\d+$/.test(raw)) return null;
  const [videoId, cid = ""] = raw.split("_");
  return videoId ? { videoId, cid } : null;
}

async function getVideoView(videoId: string): Promise<BiliView | null> {
  const params = videoId.startsWith("av") ? `aid=${videoId.slice(2)}` : `bvid=${videoId}`;
  const resp = await get<{ data?: BiliView | { View?: BiliView } }>(
    `https://api.bilibili.com/x/web-interface/view?${params}`
  );
  const data: BiliView | { View?: BiliView } | undefined = resp.data?.data;
  if (!data) return null;
  if ("View" in data) return data.View ?? null;
  return data as BiliView;
}

function isMcdnUrl(url: string): boolean {
  return /\.mcdn\.bilivideo\.cn(?::8082)?\//i.test(url);
}

function getDashAudioUrl(data: unknown): string {
  const dash = (data as {
    dash?: {
      audio?: Array<{
        baseUrl?: string;
        base_url?: string;
        backupUrl?: string[];
        backup_url?: string[];
        bandwidth?: number;
      }>;
    };
  })?.dash;
  const audio = [...(dash?.audio ?? [])].sort((a, b) => (b.bandwidth ?? 0) - (a.bandwidth ?? 0));
  const candidates = audio.flatMap((item) => [
    item.baseUrl ?? item.base_url ?? "",
    ...(item.backupUrl ?? item.backup_url ?? []),
  ]).filter(Boolean);
  return candidates.find((url) => !isMcdnUrl(url)) ?? candidates[0] ?? "";
}

async function getVideoAudioUrl(videoId: string, cid: string): Promise<string> {
  const isAid = videoId.startsWith("av");
  const idParam = isAid ? `avid=${videoId.slice(2)}` : `bvid=${videoId}`;

  const legacyResp = await get<{ data?: unknown }>(
    `http://api.bilibili.com/x/player/playurl?fnval=16&${idParam}&cid=${cid}`
  );
  const legacyUrl = getDashAudioUrl(legacyResp.data?.data);
  if (legacyUrl) return legacyUrl;

  try {
    const query = await encWbi({
      [isAid ? "avid" : "bvid"]: isAid ? videoId.slice(2) : videoId,
      cid,
      fnver: 0,
      fnval: 4048,
      fourk: 1,
      otype: "json",
    });
    const resp = await get<{ data?: unknown }>(
      `https://api.bilibili.com/x/player/wbi/playurl?${query}`
    );
    const url = getDashAudioUrl(resp.data?.data);
    if (url) return url;
  } catch {
    wbiKey = null;
  }
  return "";
}

function parseFavoriteListId(listId: string): { mid: string; fid: string } {
  const raw = listId.replace("bifav_", "");
  const [mid = "", fid = ""] = raw.split("_");
  return { mid, fid: fid || mid };
}

async function getFavoritePage(fid: string, page: number, pageSize: number): Promise<BiliFavoritePage> {
  const resp = await get<{ data?: BiliFavoritePage }>(
    `https://api.bilibili.com/x/v3/fav/resource/list?media_id=${encodeURIComponent(fid)}&pn=${page}&ps=${pageSize}&keyword=&order=mtime&type=0&tid=0&platform=web`
  );
  return resp.data?.data ?? {};
}

export const bilibili = {
  async search(url: string): Promise<SearchResult> {
    const keyword = getParameterByName("keywords", url);
    const curpage = Number(getParameterByName("curpage", url)) || 1;
    const searchType = getParameterByName("type", url);

    if (searchType === "1") {
      const playlists = await this.show_playlist(`/show_playlist?offset=${(curpage - 1) * 20}`);
      return {
        result: playlists.result.map((item) => ({
          id: item.id,
          title: item.title,
          artist: "",
          source: "bilibili",
          source_url: item.source_url,
          img_url: item.cover_img_url,
          url: item.id,
        })),
        total: playlists.result.length,
        type: "playlist",
      };
    }

    const query = await encWbi({ keyword, page: curpage, page_size: 20, search_type: "video" });
    const resp = await get<{ data?: { result?: Record<string, unknown>[]; numResults?: number } }>(
      `https://api.bilibili.com/x/web-interface/wbi/search/type?${query}`
    );
    const result = (resp.data?.data?.result ?? [])
      .map((s) => convertVideoSong(s))
      .filter((track) => track.id !== "bitrack_v_");
    return { result, total: resp.data?.data?.numResults ?? 1000, type: "song" };
  },

  async show_playlist(url: string): Promise<{ result: PlaylistInfo[] }> {
    const offset = Number(getParameterByName("offset", url)) || 0;
    const page = Math.floor(offset / 20) + 1;
    const resp = await get<{ data?: { data?: Array<Record<string, unknown>> } }>(
      `https://www.bilibili.com/audio/music-service-c/web/menu/hit?ps=20&pn=${page}`
    );
    const result = (resp.data?.data?.data ?? []).map((item) => {
      const id = stringValue(item.menuId);
      return {
        id: `biplaylist_${id}`,
        title: stringValue(item.title),
        cover_img_url: normalizeImageUrl(item.cover),
        source_url: `https://www.bilibili.com/audio/am${id}`,
      };
    }).filter((item) => item.id !== "biplaylist_");
    return { result };
  },

  async get_playlist(url: string): Promise<Playlist> {
    const listId = getParameterByName("list_id", url);
    const prefix = listId.split("_")[0];

    if (prefix === "biartist") return this._getArtist(listId);
    if (prefix === "biplaylist") return this._getAudioPlaylist(listId);
    if (prefix === "bifav") return this._getFavoritePlaylist(listId, false);
    return this._getVideoPlaylist(listId);
  },

  async get_playlist_full(url: string): Promise<Playlist> {
    const listId = getParameterByName("list_id", url);
    if (listId.split("_")[0] === "bifav") return this._getFavoritePlaylist(listId, true);
    return this.get_playlist(url);
  },

  async _getAudioPlaylist(listId: string): Promise<Playlist> {
    const id = listId.split("_").pop() ?? "";
    const infoResp = await get<{ data?: { title?: string; cover?: string; intro?: string } }>(
      `https://www.bilibili.com/audio/music-service-c/web/menu/info?sid=${id}`
    );
    const trackResp = await get<{ data?: { data?: Record<string, unknown>[] } }>(
      `https://www.bilibili.com/audio/music-service-c/web/song/of-menu?pn=1&ps=100&sid=${id}`
    );
    const info = infoResp.data?.data ?? {};
    return {
      info: {
        id: `biplaylist_${id}`,
        title: info.title ?? "哔哩哔哩歌单",
        cover_img_url: normalizeImageUrl(info.cover),
        description: info.intro ?? "",
        source_url: `https://www.bilibili.com/audio/am${id}`,
      },
      tracks: (trackResp.data?.data?.data ?? []).map(convertAudioSong),
    };
  },

  async _getVideoPlaylist(listId: string): Promise<Playlist> {
    const parsed = parseVideoTrackId(listId);
    const videoId = parsed?.videoId ?? listId.replace("bitrack_", "");
    const view = await getVideoView(videoId);
    if (!view) {
      return {
        info: {
          id: listId,
          title: "哔哩哔哩视频",
          cover_img_url: "",
          source_url: `https://www.bilibili.com/video/${videoId}`,
        },
        tracks: [],
      };
    }

    const pages = view.pages ?? [];
    return {
      info: {
        id: `bitrack_v_${videoId}`,
        title: stringValue(view.title) || "哔哩哔哩视频",
        cover_img_url: normalizeImageUrl(view.pic),
        description: stringValue(view.desc),
        source_url: `https://www.bilibili.com/video/${videoId}`,
      },
      tracks: pages.map((page) => convertVideoPage(page, view, videoId)).filter((track) => track.id !== `bitrack_v_${videoId}-`),
    };
  },

  async _getFavoritePlaylist(listId: string, full = true): Promise<Playlist> {
    const { mid, fid } = parseFavoriteListId(listId);
    const pageSize = 20;
    const first = await getFavoritePage(fid, 1, pageSize);
    const pages: BiliFavoritePage[] = [first];

    if (full) {
      const total = Number(first.info?.media_count ?? 0);
      const knownPageCount = total > 0 ? Math.ceil(total / pageSize) : 0;
      let page = 2;
      let hasMore = first.has_more === true || (first.medias?.length ?? 0) >= pageSize;

      while (hasMore && page <= Math.max(knownPageCount, page) && page <= 100) {
        const next = await getFavoritePage(fid, page, pageSize).catch(() => null);
        if (!next) break;
        const count = next.medias?.length ?? 0;
        if (count === 0) break;
        pages.push(next);
        hasMore = next.has_more === true || (knownPageCount > 0 && page < knownPageCount) || count >= pageSize;
        page += 1;
      }
    }

    const tracks = pages
      .flatMap((page) => page.medias ?? [])
      .map(convertFavoriteMedia)
      .filter((track): track is Track => track !== null)
      .filter((track) => track.id !== "bitrack_v_");
    return {
      info: {
        id: `bifav_${mid || first.info?.upper?.mid || 0}_${fid}`,
        title: first.info?.title ?? "哔哩哔哩收藏夹",
        cover_img_url: normalizeImageUrl(first.info?.cover),
        source_url: mid ? `https://space.bilibili.com/${mid}/favlist?fid=${fid}&ftype=create` : `https://space.bilibili.com/favlist?fid=${fid}`,
      },
      tracks,
    };
  },

  async _getArtist(listId: string): Promise<Playlist> {
    const mid = listId.split("_").pop() ?? "";
    const infoResp = await get<{ data?: { card?: { name?: string; face?: string } } }>(
      `https://api.bilibili.com/x/web-interface/card?mid=${mid}`
    );
    const query = await encWbi({ mid, ps: 30, pn: 1, order: "pubdate" });
    const resp = await get<{ data?: { list?: { vlist?: Record<string, unknown>[] } } }>(
      `https://api.bilibili.com/x/space/wbi/arc/search?${query}`
    );
    const card = infoResp.data?.data?.card ?? {};
    return {
      info: {
        id: `biartist_${mid}`,
        title: card.name ?? "哔哩哔哩用户",
        cover_img_url: normalizeImageUrl(card.face),
        source_url: `https://space.bilibili.com/${mid}`,
      },
      tracks: (resp.data?.data?.list?.vlist ?? []).map((s) => convertVideoSong(s)),
    };
  },

  async get_song_url(trackId: string): Promise<UrlResult | null> {
    const video = parseVideoTrackId(trackId);
    if (video) {
      let cid = video.cid;
      if (!cid) {
        const view = await getVideoView(video.videoId);
        cid = stringValue(view?.pages?.[0]?.cid);
      }
      if (!cid) return null;
      const url = await getVideoAudioUrl(video.videoId, cid);
      return url ? { url, platform: "bilibili" } : null;
    }

    const songId = trackId.replace("bitrack_", "");
    if (!/^\d+$/.test(songId)) return null;
    const resp = await get<{ data?: { cdns?: string[] } }>(
      `https://www.bilibili.com/audio/music-service-c/web/url?sid=${songId}`
    );
    const url = resp.data?.data?.cdns?.[0] ?? "";
    return url ? { url, platform: "bilibili" } : null;
  },

  async lyric(): Promise<LyricResult> {
    return { lyric: "", tlyric: "" };
  },

  async get_user(): Promise<UserStatus> {
    const resp = await get<{ data?: BiliNavData }>("https://api.bilibili.com/x/web-interface/nav");
    const data = resp.data?.data;
    if (!data?.isLogin || data.mid == null) {
      return { status: "fail", data: { is_login: false, platform: "bilibili" } };
    }
    return {
      status: "success",
      data: {
        is_login: true,
        user_id: data.mid,
        user_name: String(data.mid),
        nickname: data.uname ?? "哔哩哔哩用户",
        avatar: normalizeImageUrl(data.face),
        platform: "bilibili",
        data,
      },
    };
  },

  async get_user_created_playlist(url: string): Promise<UserPlaylistResult> {
    const mid = getParameterByName("user_id", url);
    if (!mid) return { status: "fail", data: { playlists: [] } };
    const resp = await get<{ data?: { list?: BiliFavoriteFolder[] } }>(
      `https://api.bilibili.com/x/v3/fav/folder/created/list-all?up_mid=${encodeURIComponent(mid)}`
    );
    const playlists = (resp.data?.data?.list ?? []).map((item) => {
      const fid = stringValue(item.id ?? item.fid);
      return {
        id: `bifav_${mid}_${fid}`,
        title: item.title ?? "",
        cover_img_url: normalizeImageUrl(item.cover),
        source_url: `https://space.bilibili.com/${mid}/favlist?fid=${fid}&ftype=create`,
      };
    }).filter((item) => item.id !== `bifav_${mid}_`);
    return { status: "success", data: { playlists } };
  },

  async get_user_favorite_playlist(url: string): Promise<UserPlaylistResult> {
    return this.get_user_created_playlist(url);
  },

  async parse_url(url: string): Promise<PlaylistInfo | null> {
    try {
      const favUrl = new URL(url, "https://www.bilibili.com");
      if (/space\.bilibili\.com$/.test(favUrl.hostname) && favUrl.pathname.includes("/favlist")) {
        const mid = /\/(\d+)\/favlist/.exec(favUrl.pathname)?.[1] ?? "0";
        const fid = favUrl.searchParams.get("fid") ?? favUrl.searchParams.get("media_id") ?? "";
        if (fid) return { id: `bifav_${mid}_${fid}`, title: "" };
      }
    } catch {
      // Continue with the legacy regex parsers below.
    }

    const audioPlaylistMatch = /bilibili\.com\/audio\/am([0-9]+)/.exec(url);
    if (audioPlaylistMatch) return { id: `biplaylist_${audioPlaylistMatch[1]}`, title: "" };

    const audioTrackMatch = /bilibili\.com\/audio\/au([0-9]+)/.exec(url);
    if (audioTrackMatch) return { id: `bitrack_${audioTrackMatch[1]}`, title: "" };

    const videoMatch = /bilibili\.com\/(?:video\/)?(BV[0-9A-Za-z]+)/.exec(url);
    if (videoMatch) return { id: `bitrack_v_${videoMatch[1]}`, title: "" };
    return null;
  },

  get_playlist_filters(): PlaylistFilter[] {
    return [];
  },
};
