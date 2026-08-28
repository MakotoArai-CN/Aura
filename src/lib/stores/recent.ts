import { writable } from "svelte/store";
import type { Track } from "./player";

/** 最近播放。列表头是最新的一首。 */
export interface RecentEntry {
  track: Track;
  /** 最后一次播放的时间戳（ms）。 */
  playedAt: number;
}

const KEY = "aura_recent_played";
/** 只留这么多首。够翻很久了，再多纯粹是拖慢 localStorage 的读写。 */
const LIMIT = 200;

/**
 * 落盘前把易失字段摘掉。
 *
 * `url` / `sound_url` 多数源是带签名和有效期的直链，隔一天就 403；`lyric` 是整段歌词
 * 文本，几百首攒下来能把这条记录顶到几 MB。这些都能在播放时重新解析出来，没有必要存。
 */
function slim(track: Track): Track {
  const { url: _url, sound_url: _soundUrl, lyric: _lyric, ...rest } = track;
  return rest;
}

function load(): RecentEntry[] {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    // 存过的数据也可能是坏的（手改、旧版本格式），逐条筛掉没有 id 的。
    // 时间戳用 Number.isFinite 而不是 typeof === "number"：NaN 也是 number，
    // 拿它算出来的是 Invalid Date，分组 key 变成 NaN，两条就能把整个视图撞成白屏。
    return parsed.filter(
      (e): e is RecentEntry => Boolean(e?.track?.id) && Number.isFinite(e.playedAt),
    );
  } catch {
    return [];
  }
}

function save(list: RecentEntry[]) {
  try {
    localStorage.setItem(KEY, JSON.stringify(list));
  } catch {
    // 配额满了就算了，最近播放丢了不影响听歌
  }
}

function createRecentStore() {
  const { subscribe, set, update } = writable<RecentEntry[]>(load());

  return {
    subscribe,
    /**
     * 记一首。同一首歌只保留一条，重播就把它提到最前面并刷新时间。
     */
    record(track: Track) {
      if (!track?.id) return;
      update((list) => {
        const next = [{ track: slim(track), playedAt: Date.now() }, ...list.filter((e) => e.track.id !== track.id)];
        if (next.length > LIMIT) next.length = LIMIT;
        save(next);
        return next;
      });
    },
    remove(id: string) {
      update((list) => {
        const next = list.filter((e) => e.track.id !== id);
        save(next);
        return next;
      });
    },
    clear() {
      save([]);
      set([]);
    },
  };
}

export const recentPlayed = createRecentStore();
