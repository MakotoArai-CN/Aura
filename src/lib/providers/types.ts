import type { Track } from "../stores/player";

export interface SearchResult {
  result: Track[];
  total: number;
  type: "song" | "playlist" | "artist" | "album";
}

export interface PlaylistInfo {
  id: string;
  title: string;
  cover_img_url?: string;
  source_url?: string;
  description?: string;
}

export interface Playlist {
  info: PlaylistInfo;
  tracks: Track[];
}

export interface PlaylistFilter {
  id: string;
  name: string;
  items?: Array<{ id: string; name: string }>;
}

export interface LyricResult {
  lyric: string;
  tlyric?: string;
}

export interface UrlResult {
  url: string;
  bitrate?: string;
  platform?: string;
  track?: Track;
}

export interface LoginProvider {
  id: string;
  name: string;
}

export interface UserInfo {
  is_login: boolean;
  user_id?: string | number;
  user_name?: string;
  nickname?: string;
  avatar?: string;
  platform?: string;
  data?: unknown;
}

export interface UserStatus {
  status: "success" | "fail";
  data: UserInfo;
}

export interface UserPlaylistResult {
  status: "success" | "fail";
  data: {
    playlists: PlaylistInfo[];
  };
}

export interface Provider {
  search(url: string): Promise<SearchResult>;
  show_playlist(url: string): Promise<{ result: PlaylistInfo[] }>;
  get_playlist(url: string): Promise<Playlist>;
  get_playlist_full?: (url: string) => Promise<Playlist>;
  get_playlist_filters(): PlaylistFilter[];
  lyric(url: string): Promise<LyricResult>;
  get_song_url(trackId: string, trackHint?: Track, quality?: string): Promise<UrlResult | null>;
  parse_url(url: string): Promise<PlaylistInfo | null>;
  get_user?: () => Promise<UserStatus>;
  get_user_created_playlist?: (url: string) => Promise<UserPlaylistResult>;
  get_user_favorite_playlist?: (url: string) => Promise<UserPlaylistResult>;
}
