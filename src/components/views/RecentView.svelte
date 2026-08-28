<script lang="ts">
  import { player } from "../../lib/player";
  import SongRow from "../ui/SongRow.svelte";
  import { recentPlayed } from "../../lib/stores/recent";
  import { toast } from "../../lib/stores/toast";

  let { navigate }: { navigate: (v: unknown) => void } = $props();

  let query = $state("");
  let confirmingClear = $state(false);

  let filtered = $derived.by(() => {
    const q = query.trim().toLowerCase();
    if (!q) return $recentPlayed;
    return $recentPlayed.filter(({ track }) =>
      `${track.title} ${track.artist} ${track.album ?? ""}`.toLowerCase().includes(q),
    );
  });

  /** 分组用的日期标签：今天 / 昨天 / N 天前 / M 月 D 日。 */
  function dayLabel(ts: number): string {
    const midnight = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
    const then = new Date(ts);
    const days = Math.round((midnight(new Date()) - midnight(then)) / 86400000);
    if (days <= 0) return "今天";
    if (days === 1) return "昨天";
    if (days < 7) return `${days} 天前`;
    return `${then.getMonth() + 1} 月 ${then.getDate()} 日`;
  }

  /**
   * 按天切成一段一段，段与段之间插一条日期分隔。
   *
   * 刻意不做成「每行右边贴一个时间」——SongRow 的根节点就是 <li>，想在它旁边放东西
   * 就得再套一层 <li>，而 <li> 里不能直接放 <li>，解析器会把外层提前闭合，布局直接散架。
   * 分隔行本身是 <li>，跟 SongRow 平级，合法且更好读。
   */
  let groups = $derived.by(() => {
    const out: Array<{ label: string; entries: typeof filtered }> = [];
    for (const entry of filtered) {
      const label = dayLabel(entry.playedAt);
      const tail = out[out.length - 1];
      if (tail?.label === label) tail.entries.push(entry);
      else out.push({ label, entries: [entry] });
    }
    return out;
  });

  function playFrom(id: string) {
    // 用当前筛选结果当播放队列，符合「看到的就是接下来会播的」
    player.setPlaylist(filtered.map((e) => e.track));
    player.playById(id);
  }

  function playAll() {
    if (!filtered.length) return;
    player.setPlaylist(filtered.map((e) => e.track));
    player.loadByIndex(0);
  }

  function clearAll() {
    recentPlayed.clear();
    confirmingClear = false;
    toast.success("已清空最近播放");
  }
</script>

<div class="recentbox">
  <div class="recent-head">
    <h2>最近播放</h2>
    <p class="recent-sub">
      {#if $recentPlayed.length}
        共 {$recentPlayed.length} 首，最近听的在最前面
      {:else}
        还没有播放记录
      {/if}
    </p>

    {#if $recentPlayed.length}
      <div class="recent-actions">
        <button class="recent-btn primary" onclick={playAll}>
          <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor" style="stroke:none;margin-right:4px">
            <path d="M8 5v14l11-7z"/>
          </svg>
          播放全部
        </button>
        {#if confirmingClear}
          <button class="recent-btn danger" onclick={clearAll}>确认清空</button>
          <button class="recent-btn" onclick={() => (confirmingClear = false)}>取消</button>
        {:else}
          <button class="recent-btn" onclick={() => (confirmingClear = true)}>清空</button>
        {/if}
        <div class="recent-search">
          <svg width="15" height="15" viewBox="0 0 24 24" style="opacity:0.5;flex-shrink:0;margin-right:6px">
            <circle cx="11" cy="11" r="8"/><path d="m21 21-4.35-4.35"/>
          </svg>
          <input type="text" placeholder="在最近播放中查找" bind:value={query} />
        </div>
      </div>
    {/if}
  </div>

  {#if !$recentPlayed.length}
    <div class="empty-state">
      听过的歌会自动出现在这里
      <button class="recent-btn primary" style="margin-top:14px" onclick={() => navigate({ type: "discover", source: "netease" })}>
        去发现音乐
      </button>
    </div>
  {:else if !filtered.length}
    <div class="empty-state">没有匹配「{query}」的记录</div>
  {:else}
    <ul class="detail-songlist recent-songlist">
      {#each groups as group (group.label)}
        <li class="day-sep">{group.label}</li>
        {#each group.entries as entry (entry.track.id)}
          <SongRow
            track={entry.track}
            showSourceBadge
            showRemove
            onPlay={() => playFrom(entry.track.id)}
            onRemove={() => recentPlayed.remove(entry.track.id)}
          />
        {/each}
      {/each}
    </ul>
  {/if}
</div>

<style>
  .recentbox { padding-bottom: 120px; }

  .recent-head { padding: 24px 25px 8px; max-width: 900px; }
  .recent-head h2 { margin: 0; font-size: 24px; font-weight: 700; }
  .recent-sub { margin: 6px 0 0; font-size: 12px; color: var(--link-default-color); }

  .recent-actions {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    margin-top: 14px;
  }

  .recent-btn {
    display: flex;
    align-items: center;
    background: var(--button-background-color);
    border: none;
    border-radius: 8px;
    padding: 6px 14px;
    font-size: 13px;
    font-weight: 600;
    color: var(--text-default-color);
    cursor: pointer;
    min-width: unset; min-height: unset;
    transition: transform 0.2s, background-color 0.2s, box-shadow 0.2s;
  }

  .recent-btn:hover { transform: scale(1.05); background: var(--button-hover-background-color); }
  .recent-btn.primary { background: var(--accent); color: #fff; }
  .recent-btn.primary:hover { box-shadow: 0 6px 16px -2px var(--theme-color-glow); }
  .recent-btn.danger { background: #d9534f; color: #fff; }

  .recent-search {
    display: flex;
    align-items: center;
    flex: 1 1 180px;
    min-width: 140px;
    max-width: 260px;
    background: var(--button-background-color);
    border-radius: 8px;
    padding: 6px 12px;
  }

  .recent-search input {
    flex: 1;
    min-width: 0;
    background: transparent;
    border: none;
    outline: none;
    font-size: 13px;
    color: var(--text-default-color);
  }

  .recent-search input::placeholder { color: var(--link-default-color); }

  ul.detail-songlist.recent-songlist {
    padding: 0 25px;
    margin: 0;
    list-style: none;
    max-width: 900px;
  }

  /* SongRow 自己就是 <li>，直接当 ul 的子节点用，中间穿插日期分隔行。 */
  .day-sep {
    list-style: none;
    padding: 16px 4px 6px;
    font-size: 12px;
    font-weight: 600;
    color: var(--link-default-color);
  }

  .day-sep:first-child { padding-top: 4px; }

  ul.detail-songlist.recent-songlist > :global(li) {
    animation: fadeInUp var(--dur-slow) var(--ease-out-quart) both;
  }

  .empty-state {
    display: flex; flex-direction: column;
    align-items: center; justify-content: center;
    padding: 80px; color: var(--link-default-color); font-size: 14px;
    animation: fadeIn var(--dur-slow) var(--ease-out-quart);
  }
</style>
