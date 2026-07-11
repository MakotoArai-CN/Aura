import { writable, derived } from "svelte/store";

export type LoopMode = 0 | 1 | 2; // 0=순서 1=단곡반복 2=랜덤

export interface Track {
  id: string;
  title: string;
  artist: string;
  artist_id?: string;
  album?: string;
  album_id?: string;
  img_url?: string;
  source: string;
  source_url?: string;
  url?: string;
  sound_url?: string;
  lyric_url?: string;
  tlyric_url?: string;
  lyric?: string;
  disabled?: boolean;
  platform?: string;
  bitrate?: string;
  quality?: string;
  duration?: number;
  song_id?: string | number;
  content_id?: string | number;
}

export interface PlayerState {
  playing: boolean;
  loading: boolean;
  volume: number;
  muted: boolean;
  loopMode: LoopMode;
  currentIndex: number;
  playlist: Track[];
  currentTrack: Track | null;
  position: number;
  duration: number;
  playedFrom: number;
}

const initial: PlayerState = {
  playing: false,
  loading: false,
  volume: 90,
  muted: false,
  loopMode: 0,
  currentIndex: -1,
  playlist: [],
  currentTrack: null,
  position: 0,
  duration: 0,
  playedFrom: 0,
};

function createPlayerStore() {
  const { subscribe, set, update } = writable<PlayerState>(initial);

  return {
    subscribe,
    set,
    update,
    patch(partial: Partial<PlayerState>) {
      update((s) => ({ ...s, ...partial }));
    },
  };
}

export const playerState = createPlayerStore();

export const progressPercent = derived(playerState, ($s) =>
  $s.duration > 0 ? ($s.position / $s.duration) * 100 : 0
);

export const positionFormatted = derived(playerState, ($s) => {
  const s = Math.floor($s.position);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
});

export const durationFormatted = derived(playerState, ($s) => {
  const s = Math.floor($s.duration);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
});
