import { get } from "../tauri";
import { getParameterByName } from "./utils";
import type { SearchResult, Playlist, PlaylistInfo, LyricResult, UrlResult, PlaylistFilter, UserStatus } from "./types";
import type { Track } from "../stores/player";

function convertSong(s: Record<string, unknown>): Track {
  const singers = s.singers as Array<{ name: string; id: string }> ?? [];
  const singerList = s.singerList as Array<{ name: string; id: string }> ?? [];
  const artists = s.artists as Array<{ name: string; id: string }> ?? [];
  const album = s.album as { name: string; id: string } | undefined;
  const albumImgs = s.albumImgs as Array<{ img: string }> | undefined;
  const imgItems = s.imgItems as Array<{ imgUrl?: string; img?: string }> | undefined;
  const ext = s.ext as { lrcUrl?: string; trcUrl?: string } | undefined;
  const copyrightId = s.copyrightId ?? s.id;
  const artistName = singers[0]?.name ?? singerList[0]?.name ?? artists[0]?.name ?? s.singer ?? "";
  const artistId = singers[0]?.id ?? singerList[0]?.id ?? artists[0]?.id ?? s.singerId ?? "";
  const albumName = album?.name ?? s.album ?? "";
  const albumId = album?.id ?? s.albumId ?? "";
  const image = imgItems?.[0]?.imgUrl?.replace("{size}", "300")
    ?? imgItems?.[0]?.img
    ?? albumImgs?.[1]?.img
    ?? albumImgs?.[0]?.img
    ?? s.img1
    ?? "";
  return {
    id: `mgtrack_${copyrightId}`,
    title: (s.name ?? s.songName ?? "") as string,
    artist: String(artistName),
    artist_id: `mgartist_${artistId}`,
    album: String(albumName),
    album_id: `mgalbum_${albumId}`,
    img_url: String(image),
    source: "migu",
    source_url: `https://music.migu.cn/v3/music/song/${copyrightId}`,
    url: (s.vipFlag === 1 || s.vip === 1) ? "" : undefined,
    lyric_url: String(s.lrcUrl ?? ext?.lrcUrl ?? ""),
    tlyric_url: String(s.trcUrl ?? ext?.trcUrl ?? ""),
    quality: String(s.toneControl ?? ""),
    song_id: s.songId as string | number | undefined,
    content_id: s.contentId as string | number | undefined,
  };
}

const MG_HEADERS = {
  "channel": "0H00",
  "Origin": "https://m.music.migu.cn",
};

export const migu = {
  async search(url: string): Promise<SearchResult> {
    const keyword = getParameterByName("keywords", url);
    const curpage = Number(getParameterByName("curpage", url));
    const resp = await get<Record<string, unknown>[]>(
      `https://app.u.nf.migu.cn/pc/resource/song/item/search/v1.0?text=${encodeURIComponent(keyword)}&pageNo=${curpage}&pageSize=20`,
      MG_HEADERS
    );
    return {
      result: (Array.isArray(resp.data) ? resp.data : []).map(convertSong),
      total: 1000,
      type: "song",
    };
  },

  async show_playlist(url: string): Promise<{ result: PlaylistInfo[] }> {
    const offset = Number(getParameterByName("offset", url)) || 0;
    const filterId = getParameterByName("filter_id", url) || "";

    if (filterId === "toplist") {
      if (offset > 0) return { result: [] };
      const resp = await get<{ data: Array<{ picUrl: string; columnName: string; columnId: string }> }>(
        "https://m.music.migu.cn/migu/remoting/special_index_tag?type=3&pageSize=20",
        MG_HEADERS
      );
      return {
        result: (resp.data.data ?? []).map((item) => ({
          cover_img_url: item.picUrl,
          title: item.columnName,
          id: `mgtoplist_${item.columnId}`,
          source_url: `https://music.migu.cn/v3/music/top/${item.columnId}`,
        })),
      };
    }

    const page = Math.floor(offset / 30) + 1;
    if (!filterId) {
      const resp = await get<{ data?: { contentItemList?: Array<{ itemList?: Array<{ imageUrl: string; title: string; actionUrl?: string }> }> } }>(
        `https://app.c.nf.migu.cn/MIGUM2.0/v2.0/content/getMusicData.do?count=30&start=${page}&templateVersion=5&type=1`,
        MG_HEADERS
      );
      const items = resp.data.data?.contentItemList?.[0]?.itemList ?? [];
      return {
        result: items.map((item) => {
          const id = /id=(\d+)&?/.exec(item.actionUrl ?? "")?.[1] ?? "";
          return {
            cover_img_url: item.imageUrl,
            title: item.title,
            id: `mgplaylist_${id}`,
            source_url: `https://music.migu.cn/v3/music/playlist/${id}`,
          };
        }).filter((item) => item.id !== "mgplaylist_"),
      };
    }

    const resp = await get<{ data?: { contentItemList?: { itemList?: Array<{ imageUrl: string; title: string; actionUrl?: string }> } } }>(
      `https://app.c.nf.migu.cn/MIGUM3.0/v1.0/template/musiclistplaza-listbytag?pageNumber=${page}&tagId=${filterId}&templateVersion=1`,
      MG_HEADERS
    );
    const items = resp.data.data?.contentItemList?.itemList ?? [];
    return {
      result: items.map((item) => {
        const id = /id=(\d+)&?/.exec(item.actionUrl ?? "")?.[1] ?? "";
        return {
        cover_img_url: item.imageUrl,
        title: item.title,
        id: `mgplaylist_${id}`,
        source_url: `https://music.migu.cn/v3/music/playlist/${id}`,
      };
      }).filter((item) => item.id !== "mgplaylist_"),
    };
  },

  async get_playlist(url: string): Promise<Playlist> {
    const listId = getParameterByName("list_id", url);
    const prefix = listId.split("_")[0];

    if (prefix === "mgalbum") return this._getAlbum(listId);
    if (prefix === "mgartist") return this._getArtist(listId);
    if (prefix === "mgtoplist") return this._getToplist(listId);
    return this._getPlaylist(listId);
  },

  async _getPlaylist(listId: string): Promise<Playlist> {
    const id = listId.split("_").pop();
    const infoResp = await get<{ resource?: Array<{ imgItem?: { img?: string }; imgItems?: Array<{ img?: string }>; title?: string; musicNum?: number }> }>(
      `https://app.c.nf.migu.cn/MIGUM2.0/v1.0/content/resourceinfo.do?needSimple=00&resourceType=2021&resourceId=${id}`,
      MG_HEADERS
    );
    const trackResp = await get<{ list?: Record<string, unknown>[] }>(
      `https://app.c.nf.migu.cn/MIGUM2.0/v1.0/user/queryMusicListSongs.do?musicListId=${id}&pageNo=1&pageSize=50`,
      MG_HEADERS
    );
    const pl = infoResp.data.resource?.[0];
    return {
      info: { id: `mgplaylist_${id}`, title: pl?.title ?? "咪咕歌单", cover_img_url: pl?.imgItem?.img ?? pl?.imgItems?.[0]?.img ?? "" },
      tracks: (trackResp.data.list ?? []).map(convertSong),
    };
  },

  async get_playlist_full(url: string): Promise<Playlist> {
    const listId = getParameterByName("list_id", url);
    const prefix = listId.split("_")[0];
    if (prefix !== "mgplaylist" && prefix !== "mgalbum") return this.get_playlist(url);

    const id = listId.split("_").pop();
    const isAlbum = prefix === "mgalbum";
    const resourceType = isAlbum ? "2003" : "2021";
    const infoResp = await get<{ resource?: Array<{ imgItem?: { img?: string }; imgItems?: Array<{ img?: string }>; title?: string; musicNum?: number; totalCount?: number }> }>(
      `https://app.c.nf.migu.cn/MIGUM2.0/v1.0/content/resourceinfo.do?needSimple=00&resourceType=${resourceType}&resourceId=${id}`,
      MG_HEADERS
    );
    const info = infoResp.data.resource?.[0];
    const total = Number(info?.musicNum ?? info?.totalCount ?? 0);
    const pageSize = 50;
    const pageCount = Math.max(1, total ? Math.ceil(total / pageSize) : 1);
    const pages = await Promise.all(Array.from({ length: pageCount }, (_, i) => {
      const target = isAlbum
        ? `https://app.c.nf.migu.cn/MIGUM2.0/v1.0/content/queryAlbumSong?albumId=${id}&pageNo=${i + 1}&pageSize=${pageSize}`
        : `https://app.c.nf.migu.cn/MIGUM2.0/v1.0/user/queryMusicListSongs.do?musicListId=${id}&pageNo=${i + 1}&pageSize=${pageSize}`;
      return get<{ list?: Record<string, unknown>[]; data?: { songList?: Record<string, unknown>[] } }>(target, MG_HEADERS).catch(() => null);
    }));
    return {
      info: {
        id: `${prefix}_${id}`,
        title: info?.title ?? (isAlbum ? "咪咕专辑" : "咪咕歌单"),
        cover_img_url: info?.imgItem?.img ?? info?.imgItems?.[1]?.img ?? info?.imgItems?.[0]?.img ?? "",
        source_url: isAlbum ? `https://music.migu.cn/v3/music/album/${id}` : `https://music.migu.cn/v3/music/playlist/${id}`,
      },
      tracks: pages.flatMap((resp) => (resp?.data.list ?? resp?.data.data?.songList ?? []).map(convertSong)),
    };
  },

  async _getToplist(listId: string): Promise<Playlist> {
    const id = listId.split("_").pop();
    const resp = await get<{ data: { columnName: string; picUrl: string; rankList: Array<{ musics: Record<string, unknown>[] }> } }>(
      `https://m.music.migu.cn/migu/remoting/cms_rank_list_song_tag?columnId=${id}&pageNo=1&pageSize=100`,
      MG_HEADERS
    );
    const tracks = resp.data.data?.rankList?.flatMap((r) => r.musics.map(convertSong)) ?? [];
    return {
      info: { id: `mgtoplist_${id}`, title: resp.data.data.columnName, cover_img_url: resp.data.data.picUrl },
      tracks,
    };
  },

  async _getAlbum(listId: string): Promise<Playlist> {
    const id = listId.split("_").pop();
    const resp = await get<{ resource: { picUrl: string; name: string; id: string; songs: Record<string, unknown>[] } }>(
      `https://m.music.migu.cn/migu/remoting/cms_album_detail_tag?albumId=${id}`,
      MG_HEADERS
    );
    return {
      info: { id: `mgalbum_${id}`, title: resp.data.resource.name, cover_img_url: resp.data.resource.picUrl },
      tracks: (resp.data.resource.songs ?? []).map(convertSong),
    };
  },

  async _getArtist(listId: string): Promise<Playlist> {
    const id = listId.split("_").pop();
    const resp = await get<{ data: { artistInfo: { picUrl: string; name: string }; list: Record<string, unknown>[] } }>(
      `https://m.music.migu.cn/migu/remoting/cms_artist_songs_tag?artistId=${id}&pageNo=1&pageSize=100`,
      MG_HEADERS
    );
    return {
      info: { id: `mgartist_${id}`, title: resp.data.data.artistInfo.name, cover_img_url: resp.data.data.artistInfo.picUrl },
      tracks: (resp.data.data.list ?? []).map(convertSong),
    };
  },

  async get_song_url(trackId: string, trackHint?: Track): Promise<UrlResult | null> {
    const id = trackId.replace("mgtrack_", "");
    const contentId = trackHint?.content_id ? String(trackHint.content_id) : id;
    const resp = await get<{ data?: { url?: string; playUrl?: string } }>(
      `https://app.c.nf.migu.cn/MIGUM3.0/strategy/pc/listen/v1.0?scene=&netType=01&resourceType=2&copyrightId=${encodeURIComponent(id)}&contentId=${encodeURIComponent(contentId)}&toneFlag=HQ`,
      { channel: "0146951", uid: "1234" }
    );
    let url = resp.data.data?.url ?? resp.data.data?.playUrl ?? "";
    if (url.startsWith("//")) url = `https:${url}`;
    url = url.replace(/\+/g, "%2B");
    if (!url) return null;
    return { url, platform: "migu", bitrate: "320kbps" };
  },

  async lyric(url: string): Promise<LyricResult> {
    const trackId = getParameterByName("track_id", url).replace("mgtrack_", "");
    const lyricsUrlResp = getParameterByName("lyric_url", url);
    const tlyricsUrl = getParameterByName("tlyric_url", url);

    let lyric = "";
    let tlyric = "";

    if (lyricsUrlResp) {
      const resp = await get<string>(lyricsUrlResp);
      lyric = resp.text ?? "";
    }
    if (tlyricsUrl) {
      const resp = await get<string>(tlyricsUrl);
      tlyric = resp.text ?? "";
    }

    return { lyric, tlyric };
  },

  async get_user(): Promise<UserStatus> {
    const ts = Date.now();
    const resp = await get<{
      success?: boolean;
      user?: {
        uid?: string;
        mobile?: string;
        nickname?: string;
        avatar?: { midAvatar?: string };
      };
    }>(`https://music.migu.cn/v3/api/user/getUserInfo?_=${ts}`);
    if (!resp.data.success || !resp.data.user) {
      return { status: "fail", data: { is_login: false, platform: "migu" } };
    }
    return {
      status: "success",
      data: {
        is_login: true,
        user_id: resp.data.user.uid,
        user_name: resp.data.user.mobile,
        nickname: resp.data.user.nickname ?? "咪咕用户",
        avatar: resp.data.user.avatar?.midAvatar ?? "",
        platform: "migu",
        data: resp.data,
      },
    };
  },

  async parse_url(url: string): Promise<PlaylistInfo | null> {
    const normalized = url.replace("music.migu.cn/v3/my/playlist/", "music.migu.cn/v3/music/playlist/");
    if (normalized.includes("music.migu.cn/v3/music/playlist/")) {
      const match = /\/playlist\/(\d+)/.exec(normalized);
      if (match) return { id: `mgplaylist_${match[1]}`, title: "" };
    }
    return null;
  },

  get_playlist_filters(): PlaylistFilter[] {
    return [
      {
        id: "migu",
        name: "咪咕",
        items: [
          { id: "", name: "全部" },
          { id: "toplist", name: "排行榜" },
          { id: "3101", name: "华语" },
          { id: "3102", name: "欧美" },
          { id: "3103", name: "日韩" },
          { id: "3104", name: "港台" },
          { id: "3105", name: "流行" },
          { id: "3106", name: "摇滚" },
          { id: "3107", name: "古典" },
          { id: "3108", name: "民谣" },
        ],
      },
    ];
  },
};
