import { writable } from "svelte/store";

export interface PlaylistInfo {
  id: string;
  title: string;
  cover_img_url?: string;
  source_url?: string;
  description?: string;
}

export interface Playlist {
  info: PlaylistInfo;
  tracks: import("./player").Track[];
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

function createMyPlaylistStore() {
  function load(): string[] {
    return lsGet<string[]>("playerlists") ?? [];
  }

  const { subscribe, set, update } = writable<string[]>(load());

  return {
    subscribe,
    reload() {
      set(load());
    },
    getPlaylist(id: string): Playlist | null {
      return lsGet<Playlist>(id);
    },
    getAllPlaylists(): Playlist[] {
      const ids = load();
      return ids.map((id) => lsGet<Playlist>(id)).filter(Boolean) as Playlist[];
    },
    createPlaylist(title: string): Playlist {
      const id = `myplaylist_${Date.now()}_${Math.random().toString(36).slice(2)}`;
      const playlist: Playlist = {
        info: { id, title, cover_img_url: "" },
        tracks: [],
      };
      lsSet(id, playlist);
      const ids = load();
      ids.push(id);
      lsSet("playerlists", ids);
      set(ids);
      return playlist;
    },
    updatePlaylist(id: string, updates: Partial<Playlist>) {
      const existing = lsGet<Playlist>(id);
      if (!existing) return;
      const updated = { ...existing, ...updates };
      if (updates.info) updated.info = { ...existing.info, ...updates.info };
      lsSet(id, updated);
    },
    deletePlaylist(id: string) {
      const ids = load().filter((i) => i !== id);
      lsSet("playerlists", ids);
      localStorage.removeItem(id);
      set(ids);
    },
    addTracksToPlaylist(id: string, tracks: import("./player").Track[]) {
      const pl = lsGet<Playlist>(id);
      if (!pl) return;
      const existing = new Set(pl.tracks.map((t) => t.id));
      const newTracks = tracks.filter((t) => !existing.has(t.id));
      pl.tracks.push(...newTracks);
      lsSet(id, pl);
    },
    removeTrackFromPlaylist(id: string, trackIndex: number) {
      const pl = lsGet<Playlist>(id);
      if (!pl) return;
      pl.tracks.splice(trackIndex, 1);
      lsSet(id, pl);
    },
  };
}

export const myPlaylists = createMyPlaylistStore();

function createFavoriteStore() {
  function load(): string[] {
    return lsGet<string[]>("favoriteplayerlists") ?? [];
  }

  const { subscribe, set } = writable<string[]>(load());

  return {
    subscribe,
    reload() {
      set(load());
    },
    add(id: string) {
      const ids = load();
      if (!ids.includes(id)) {
        ids.push(id);
        lsSet("favoriteplayerlists", ids);
        set(ids);
      }
    },
    remove(id: string) {
      const ids = load().filter((i) => i !== id);
      lsSet("favoriteplayerlists", ids);
      set(ids);
    },
    has(id: string): boolean {
      return load().includes(id);
    },
  };
}

export const favoritePlaylists = createFavoriteStore();
