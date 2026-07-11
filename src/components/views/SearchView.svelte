<script lang="ts">
  import { MediaService } from "../../lib/providers/index";
  import { player } from "../../lib/player";
  import { proxyResourceUrl } from "../../lib/resourceUrl";
  import SongRow from "../ui/SongRow.svelte";
  import type { Track } from "../../lib/stores/player";
  import { toast } from "../../lib/stores/toast";
  import { runOnActionKey } from "../../lib/keyboard";
  import { infiniteScroll } from "../../lib/infiniteScroll";

  let { navigate, query: initialQuery = "" }: { navigate: (v: unknown) => void; query?: string } = $props();

  let searchQuery = $state("");
  let lastAutoQuery = $state("");

  let source = $state("allmusic");
  let searchType = $state("0");
  let results = $state<Track[]>([]);
  let loading = $state(false);
  let page = $state(1);
  let total = $state(0);
  let searchMessage = $state("");
  let searchSeq = 0;

  const SOURCES = [
    { id: "allmusic", name: "全部" }, { id: "netease", name: "网易云" },
    { id: "qq", name: "QQ音乐" }, { id: "kugou", name: "酷狗" },
    { id: "kuwo", name: "酷我" }, { id: "bilibili", name: "哔哩哔哩" },
    { id: "migu", name: "咪咕" }, { id: "taihe", name: "千千" },
    { id: "localmusic", name: "本地音乐" },
  ];

  async function doSearch(reset = true) {
    if (!searchQuery.trim()) return;
    const seq = ++searchSeq;
    let settled = false;
    if (reset) { page = 1; results = []; total = 0; }
    searchMessage = "";
    loading = true;
    const timeout = setTimeout(() => {
      if (searchSeq !== seq || settled) return;
      loading = false;
      searchMessage = source === "allmusic"
        ? "部分音源响应超时，可切换具体音源重试"
        : "当前音源响应超时，请稍后重试";
      toast.warn(searchMessage);
    }, 8000);
    try {
      const res = await MediaService.search(source, { keywords: searchQuery, curpage: page, type: searchType });
      settled = true;
      if (searchSeq !== seq) return;
      results = reset ? res.result : [...results, ...res.result];
      total = res.total;
      searchMessage = "";
    } catch (error) {
      settled = true;
      if (searchSeq !== seq) return;
      console.error("[SearchView] search failed", { source, searchType, searchQuery, error });
      searchMessage = "搜索失败，请检查网络或音源";
      toast.error(searchMessage);
    } finally {
      clearTimeout(timeout);
      if (searchSeq === seq) loading = false;
    }
  }

  $effect(() => {
    const query = initialQuery.trim();
    if (!query || query === lastAutoQuery) return;
    lastAutoQuery = query;
    searchQuery = query;
    void doSearch();
  });

  function playAll() {
    if (!results.length) return;
    player.setPlaylist(results);
    player.loadByIndex(0);
  }
</script>

<div class="searchbox">
  <!-- Search bar matches original .navigation .search style -->
  <div class="search-bar-area">
    <div class="search-bar-wrap">
      <svg width="18" height="18" viewBox="0 0 24 24" style="opacity:0.5;flex-shrink:0;margin-right:8px">
        <circle cx="11" cy="11" r="8"/><path d="m21 21-4.35-4.35"/>
      </svg>
      <input
        type="text"
        class="search-input"
        placeholder="搜索歌曲、歌手、专辑..."
        bind:value={searchQuery}
        onkeydown={(e) => e.key === "Enter" && doSearch()}
      />
      <button class="search-btn" onclick={() => doSearch()}>搜索</button>
    </div>

    <!-- Source filter -->
    <div class="source-filter">
      {#each SOURCES as s}
        <button class="filter-tag" class:active={source === s.id}
          onclick={() => { source = s.id; if (searchQuery) doSearch(); }}>
          {s.name}
        </button>
      {/each}
    </div>

    <!-- Type filter -->
    <div class="type-filter">
      <button class="filter-tag" class:active={searchType === "0"}
        onclick={() => { searchType = "0"; if (searchQuery) doSearch(); }}>歌曲</button>
      <button class="filter-tag" class:active={searchType === "1"}
        onclick={() => { searchType = "1"; if (searchQuery) doSearch(); }}>歌单</button>
    </div>
  </div>

  {#if results.length > 0}
    <div class="result-header">
      <span style="font-size:12px;opacity:0.6">共 {total} 个结果</span>
      <button class="search-action-btn" onclick={playAll}>
        <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor" style="stroke:none;margin-right:4px">
          <path d="M8 5v14l11-7z"/>
        </svg>
        播放全部
      </button>
    </div>

    {#if searchType === "0"}
      <ul class="detail-songlist isSearch">
        {#each results as track, i (`${track.id}-${i}`)}
          <SongRow {track} index={i + 1} showSourceBadge={source === "allmusic"} onPlay={() => {
            player.setPlaylist(results);
            player.playById(track.id);
          }} />
        {/each}
      </ul>
    {:else}
      <!-- Playlist grid (same as discover) -->
      <ul class="playlist-covers isSearch">
        {#each results as item, i (`${item.id}-${i}`)}
          {@const coverUrl = proxyResourceUrl(item.img_url)}
          <li>
            <div class="u-cover" onclick={() => navigate({ type: "playlist", id: item.id })}
              role="button" tabindex="0" onkeydown={(e) => runOnActionKey(e, () => navigate({ type: "playlist", id: item.id }))}>
              {#if coverUrl}
                <img src={coverUrl} alt={item.title} />
              {:else}
                <div class="cover-ph"></div>
              {/if}
              <div class="bottom">
                <svg viewBox="0 0 24 24" fill="rgba(200,200,200,0.6)" style="width:30px;height:30px;stroke:#fff">
                  <path d="M8 5v14l11-7z"/>
                </svg>
              </div>
            </div>
            <div class="desc"><div class="title">{item.title}</div></div>
          </li>
        {/each}
      </ul>
    {/if}

    {#if results.length < total}
      <div
        class="search-pagination"
        use:infiniteScroll={{ onLoadMore: () => { page++; doSearch(false); }, disabled: loading }}
      >
        <button class="btn-pagination" onclick={() => { page++; doSearch(false); }} disabled={loading}>
          {loading ? "加载中..." : "加载更多"}
        </button>
      </div>
    {/if}
  {:else if loading}
    <div class="search-spinner-area">
      <svg class="searchspinner" viewBox="0 0 24 24" fill="none" stroke="currentColor" style="animation:spin 1s linear infinite">
        <circle cx="12" cy="12" r="10" stroke-dasharray="60" stroke-dashoffset="20"/>
      </svg>
    </div>
  {:else if searchQuery}
    <div class="empty-state">{searchMessage || "没有找到相关结果"}</div>
  {:else}
    <div class="empty-state">
      <svg width="48" height="48" viewBox="0 0 24 24" style="opacity:0.2;margin-bottom:12px">
        <circle cx="11" cy="11" r="8"/><path d="m21 21-4.35-4.35"/>
      </svg>
      <p>搜索你喜欢的音乐</p>
    </div>
  {/if}
</div>

<style>
  .searchbox { padding-bottom: 120px; }

  .search-bar-area {
    padding: 16px 20px 8px;
    display: flex;
    flex-direction: column;
    gap: 10px;
    max-width: 900px;
  }

  .search-bar-wrap {
    display: flex;
    width: 100%;
    height: 44px;
    background: var(--search-input-background-color);
    border: 1px solid transparent;
    border-radius: 999px;
    padding: 0 6px 0 16px;
    align-items: center;
    transition: border-color var(--dur-fast) var(--ease-soft),
                box-shadow var(--dur-fast) var(--ease-soft);
  }

  .search-bar-wrap:focus-within {
    border-color: var(--theme-color);
    box-shadow: 0 0 0 4px var(--theme-color-hover);
  }

  .search-input {
    flex: 1;
    background: transparent;
    border: none;
    outline: none;
    font-size: 16px;
    font-weight: 600;
    color: var(--text-default-color);
  }

  .search-input::placeholder { color: var(--link-default-color); }

  .search-btn {
    background: var(--theme-color);
    color: #fff;
    border: none;
    border-radius: 999px;
    padding: 8px 18px;
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    min-width: unset; min-height: unset;
    transition: transform var(--dur-fast) var(--ease-spring),
                background var(--dur-fast) var(--ease-soft),
                box-shadow var(--dur-fast) var(--ease-soft);
    box-shadow: 0 4px 12px -2px var(--theme-color-glow);
  }

  .search-btn:hover { transform: translateY(-1px); background: var(--accent-hover); box-shadow: 0 6px 16px -2px var(--theme-color-glow); }
  .search-btn:active { transform: translateY(0); }

  .source-filter, .type-filter { display: flex; gap: 6px; flex-wrap: wrap; }

  .filter-tag {
    padding: 5px 12px;
    border-radius: 999px;
    font-size: 12px;
    color: var(--link-default-color);
    background: none;
    border: 1px solid transparent;
    cursor: pointer;
    min-width: unset; min-height: unset;
    transition: background var(--dur-fast) var(--ease-soft),
                color var(--dur-fast) var(--ease-soft),
                border-color var(--dur-fast) var(--ease-soft),
                transform var(--dur-fast) var(--ease-spring);
    font-weight: 500;
  }

  .filter-tag:hover {
    background: var(--songlist-hover-background-color);
    color: var(--text-default-color);
    transform: translateY(-1px);
  }
  .filter-tag.active {
    background: var(--theme-color);
    color: #fff;
    box-shadow: 0 4px 10px -2px var(--theme-color-glow);
  }

  .result-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 20px 4px;
    max-width: 900px;
  }

  .search-action-btn {
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
    transition: all 0.2s;
  }

  .search-action-btn:hover { transform: scale(1.05); background: var(--button-hover-background-color); }

  /* Song list */
  ul.detail-songlist.isSearch {
    padding: 0 25px;
    list-style: none;
    margin: 0;
  }

  /* Playlist grid (same as discover) */
  ul.playlist-covers.isSearch {
    padding: 0 13px;
    display: flex;
    flex-flow: row wrap;
    margin: 0 14px;
    gap: 12px 0;
    list-style: none;
  }

  ul.playlist-covers.isSearch li {
    width: calc(20% - 24px);
    min-height: 156px;
    margin: 0 12px;
    color: var(--text-default-color);
  }

  .u-cover {
    position: relative;
    display: block;
    width: 100%; aspect-ratio: 1;
    cursor: pointer; overflow: hidden; border-radius: 10px;
    box-shadow: 0 4px 12px -4px rgba(0,0,0,0.2);
    transition: transform var(--dur-med) var(--ease-spring),
                box-shadow var(--dur-med) var(--ease-soft);
    will-change: transform;
  }

  .u-cover:hover { transform: translateY(-2px) scale(1.01); box-shadow: var(--shadow-lift); }

  .u-cover img {
    display: block; width: 100%; height: 100%;
    object-fit: cover;
    border: 1px solid var(--line-default-color);
    border-radius: 10px;
    transition: transform var(--dur-slow) var(--ease-out-expo);
  }
  .u-cover:hover img { transform: scale(1.04); }

  .u-cover .bottom {
    position: absolute; right: 10px; bottom: 10px;
    height: 36px; width: 36px;
    opacity: 0;
    transform: translateY(6px) scale(0.85);
    transition: opacity var(--dur-med) var(--ease-out-quart),
                transform var(--dur-med) var(--ease-spring);
    display: flex; align-items: center; justify-content: center;
    background: var(--theme-color);
    border-radius: 50%;
    box-shadow: 0 6px 16px var(--theme-color-glow);
    z-index: 2;
  }
  .u-cover .bottom svg { fill: #fff !important; stroke: none !important; }

  .u-cover:hover .bottom { opacity: 1; transform: translateY(0) scale(1); }
  .cover-ph { width: 100%; height: 100%; background: var(--button-background-color); border-radius: 6px; }
  .desc { margin-top: 6px; cursor: pointer; }
  .desc .title { font-size: 12px; display: -webkit-box; line-clamp: 2; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; line-height: 1.4; opacity: 0.88; }

  /* Pagination */
  .search-pagination { text-align: center; padding: 32px; }

  .btn-pagination {
    padding: 8px 24px;
    font-size: 16px; font-weight: 600;
    background: var(--button-background-color);
    border: none; border-radius: var(--default-border-radius);
    color: var(--text-default-color);
    cursor: pointer;
    min-width: unset; min-height: unset;
    transition: all 0.2s;
    opacity: 0.78;
  }

  .btn-pagination:hover { transform: scale(1.05); background: var(--button-hover-background-color); }
  .btn-pagination:disabled { opacity: 0.4; cursor: default; transform: none; }

  .search-spinner-area { text-align: center; padding: 60px; }

  .empty-state {
    display: flex; flex-direction: column;
    align-items: center; justify-content: center;
    padding: 80px; color: var(--link-default-color); font-size: 14px;
    animation: fadeIn var(--dur-slow) var(--ease-out-quart);
  }

  ul.playlist-covers.isSearch li,
  ul.detail-songlist.isSearch > :global(li) {
    animation: fadeInUp var(--dur-slow) var(--ease-out-quart) both;
  }

  @keyframes spin { to { transform: rotate(360deg); } }
</style>
