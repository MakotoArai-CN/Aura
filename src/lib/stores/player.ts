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

/**
 * 高频播放时钟：position/duration 与 playerState 解耦。
 * 进度条、时间文本、歌词同步只订阅此 store，避免每次 tick
 * 让全应用所有 $playerState 订阅者失效重算。
 */
function createPlaybackClock() {
  const { subscribe, set } = writable<{ position: number; duration: number }>({ position: 0, duration: 0 });
  let lastPosition = -1;
  let lastDuration = -1;
  return {
    subscribe,
    update(position: number, duration: number) {
      // 数值未变时不发通知，暂停/缓冲期间零开销。
      if (position === lastPosition && duration === lastDuration) return;
      lastPosition = position;
      lastDuration = duration;
      set({ position, duration });
    },
  };
}

export const playbackClock = createPlaybackClock();

export const progressPercent = derived(playbackClock, ($c) =>
  $c.duration > 0 ? Math.max(0, Math.min(100, ($c.position / $c.duration) * 100)) : 0
);

export const positionFormatted = derived(playbackClock, ($c) => {
  const s = Math.floor($c.position);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
});

export const durationFormatted = derived(playbackClock, ($c) => {
  const s = Math.floor($c.duration);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
});
