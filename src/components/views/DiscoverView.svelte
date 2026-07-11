<script lang="ts">
  import { MediaService } from "../../lib/providers/index";
  import { cssImageUrl, proxyResourceUrl } from "../../lib/resourceUrl";
  import { runOnActionKey } from "../../lib/keyboard";
  import { infiniteScroll } from "../../lib/infiniteScroll";
  import type { PlaylistFilter, PlaylistInfo } from "../../lib/providers/types";

  let { navigate, source = "netease" }: { navigate: (v: unknown) => void; source?: string } = $props();

  const SOURCES = [
    { id: "netease", name: "网易云" }, { id: "qq", name: "QQ音乐" },
    { id: "kugou", name: "酷狗" }, { id: "kuwo", name: "酷我" },
    { id: "bilibili", name: "哔哩哔哩" }, { id: "migu", name: "咪咕" },
    { id: "taihe", name: "千千" },
  ];

  // Non-reactive bookkeeping
  let activeSource = "netease";
  let activeFilter = "";
  let currentOffset = 0;
  let currentHasMore = true;
  let busy = false;

  // Reactive UI only
  let filters = $state<PlaylistFilter[]>([]);
  let playlists = $state<PlaylistInfo[]>([]);
  let loading = $state(false);
  let hasMore = $state(true);
  let activeSrcUI = $state("netease");
  let activeFilterUI = $state("");
  let loadError = $state("");
  let requestSeq = 0;
  let appliedSourceProp = "";

  function applySource(src: string) {
    activeSource = src;
    activeSrcUI = src;
    const nextFilters = MediaService.getPlaylistFilters(src)[0]?.items ?? [];
    const nextFilterId = nextFilters[0]?.id ?? "";
    filters = nextFilters;
    activeFilter = nextFilterId;
    activeFilterUI = activeFilter;
    currentOffset = 0;
    currentHasMore = true;
    fetchPlaylists(true);
  }

  function applyFilter(id: string) {
    activeFilter = id;
    activeFilterUI = id;
    currentOffset = 0;
    currentHasMore = true;
    fetchPlaylists(true);
  }

  async function fetchPlaylists(reset: boolean) {
    if (!reset && (busy || !currentHasMore)) return;
    const seq = ++requestSeq;
    const sourceForRequest = activeSource;
    const filterForRequest = activeFilter;
    const offsetForRequest = reset ? 0 : currentOffset;
    busy = true;
    loading = true;
    if (reset) { playlists = []; hasMore = true; currentOffset = 0; }
    try {
      const res = await MediaService.showPlaylistArray(sourceForRequest, offsetForRequest, filterForRequest);
      if (seq !== requestSeq || sourceForRequest !== activeSource || filterForRequest !== activeFilter) return;
      const items = res.result;
      loadError = "";
      playlists = reset ? items : [...playlists, ...items];
      currentHasMore = items.length >= 20;
      hasMore = currentHasMore;
      currentOffset += items.length;
    } catch (err) {
      if (seq !== requestSeq || sourceForRequest !== activeSource || filterForRequest !== activeFilter) return;
      console.error("[DiscoverView] playlist load failed", err);
      loadError = "加载失败";
      currentHasMore = false;
      hasMore = false;
    } finally {
      if (seq === requestSeq) {
        loading = false;
        busy = false;
      }
    }
  }

  $effect(() => {
    const nextSource = source;
    if (nextSource === appliedSourceProp) return;
    appliedSourceProp = nextSource;
    queueMicrotask(() => applySource(nextSource));
  });
</script>

<div class="searchbox">
  <!-- Source list — matches original .source-list style -->
  <div class="source-list">
    {#each SOURCES as s}
      <div class="source-button" class:active={activeSrcUI === s.id}
        onclick={() => applySource(s.id)} role="button" tabindex="0" onkeydown={(e) => runOnActionKey(e, () => applySource(s.id))}>
        <span class="buttontext" class:active={activeSrcUI === s.id}>{s.name}</span>
      </div>
    {/each}
  </div>

  <!-- Playlist filter — matches original .playlist-filter style -->
  {#if filters.length > 0}
    <div class="playlist-filter">
      {#each filters as f}
        <div class="filter-item" class:active={activeFilterUI === f.id}
          onclick={() => applyFilter(f.id)} role="button" tabindex="0" onkeydown={(e) => runOnActionKey(e, () => applyFilter(f.id))}>
          {f.name}
        </div>
      {/each}
    </div>
  {/if}

  <!-- Grid -->
  <ul class="playlist-covers">
    {#each playlists as pl, i (`${pl.id}-${i}`)}
      {@const coverUrl = proxyResourceUrl(pl.cover_img_url)}
      <li>
        <div class="u-cover" onclick={() => navigate({ type: "playlist", id: pl.id })}
          role="button" tabindex="0" onkeydown={(e) => runOnActionKey(e, () => navigate({ type: "playlist", id: pl.id }))}>
          {#if coverUrl}
            <div class="covershadow" style:background-image={cssImageUrl(pl.cover_img_url)}></div>
            <img src={coverUrl} alt={pl.title}
              onerror={(e) => { (e.currentTarget as HTMLImageElement).style.display = "none"; }} />
          {:else}
            <div class="cover-placeholder"></div>
          {/if}
          <div class="bottom">
            <svg viewBox="0 0 24 24" fill="rgba(200,200,200,0.7)" style="width:30px;height:30px;stroke:#fff">
              <path d="M8 5v14l11-7z"/>
            </svg>
          </div>
        </div>
        <div class="desc">
          <div class="title">{pl.title}</div>
        </div>
      </li>
    {/each}
    {#if loading}
      {#each Array(10) as _, i (i)}
        <li>
          <div class="u-cover"><div class="skeleton"></div></div>
          <div class="desc"><div class="title-skeleton"></div></div>
        </li>
      {/each}
    {/if}
  </ul>

  {#if !loading && playlists.length === 0 && !hasMore}
    <div class="empty-tip">{loadError || "暂无内容"}</div>
  {/if}

  {#if hasMore && playlists.length > 0}
    <div
      class="loading_bottom"
      use:infiniteScroll={{ onLoadMore: () => fetchPlaylists(false), disabled: loading }}
    >
      <button class="btn-load-more" onclick={() => fetchPlaylists(false)} disabled={loading}>
        {loading ? "加载中..." : "加载更多"}
      </button>
    </div>
  {/if}
</div>

<style>
  .source-list {
    margin: 10px 26px 24px;
    user-select: none;
    display: flex;
    align-items: center;
    flex-wrap: nowrap;
    overflow-x: auto;
  }

  .source-button {
    display: inline-block;
    cursor: pointer;
    transition: 0.1s;
    flex-shrink: 0;
  }

  .source-button:hover,
  .source-button.active {
    transition: 0.2s;
    padding: 0;
  }

  .source-button .buttontext {
    overflow: hidden;
    text-overflow: ellipsis;
    display: -webkit-box;
    -webkit-line-clamp: 1;
    line-clamp: 1;
    -webkit-box-orient: vertical;
    font-size: 14px;
    margin: 4px 10px;
    color: var(--text-subtitle-color);
    transition: 0.1s;
  }

  .source-button:hover .buttontext,
  .source-button.active .buttontext,
  .source-button .buttontext.active {
    color: var(--text-default-color);
    -webkit-app-region: no-drag;
    font-size: 24px;
    font-weight: 700;
    text-decoration: none;
    border-radius: 10px;
    padding: 6px 10px;
    transition: all 0.2s, background 0.3s;
    -webkit-user-drag: none;
    margin: 0 12px;
    white-space: nowrap;
  }

  .source-button.active .buttontext,
  .source-button .buttontext.active {
    border-bottom: solid 2px var(--text-default-color);
  }

  .source-button:hover .buttontext {
    background-color: var(--button-background-color);
  }

  .playlist-filter {
    display: flex;
    flex-wrap: wrap;
    line-height: 38px;
    margin: 0 26px 0;
  }

  .filter-item {
    display: flex;
    justify-content: center;
    align-items: center;
    line-height: 20px;
    padding: 8px 16px;
    margin: 10px 16px 6px 0;
    color: var(--black--white);
    font-weight: 600;
    font-size: 16px;
    border-radius: 10px;
    transition: all 0.2s;
    cursor: pointer;
  }

  .filter-item:hover,
  .filter-item.active {
    background: var(--theme-color-hover);
    color: var(--theme-color);
    transition: all 0.2s;
  }

  .playlist-covers {
    padding-right: 2vw;
    padding-top: 30px;
    display: flex;
    flex-flow: row wrap;
    position: relative;
    margin: 0 14px;
    list-style: none;
    gap: 40px 0;
  }

  .playlist-covers li {
    color: var(--text-default-color);
    margin: 0 12px;
  }

  @media screen and (max-width: 1000px) {
    .playlist-covers li { flex: 0 1 calc(25% - 26px); }
  }

  @media screen and (min-width: 1000px) and (max-width: 1480px) {
    .playlist-covers li { flex: 0 1 calc(20% - 26px); }
  }

  @media screen and (min-width: 1480px) {
    .playlist-covers li { flex: 0 1 calc(16.66% - 26px); }
  }

  .u-cover {
    position: relative;
    display: flex;
    justify-content: center;
    align-items: center;
    user-select: none;
  }

  .u-cover .covershadow {
    transition: all 0.4s;
    opacity: 0;
    position: absolute;
    top: 12px;
    height: 100%;
    width: 100%;
    filter: blur(16px) opacity(0.6);
    transform: scale(0.92, 0.96);
    z-index: 0;
    background-size: cover;
    border-radius: 0.75em;
  }

  .u-cover img {
    display: block;
    width: 100%;
    aspect-ratio: 1;
    object-fit: cover;
    border: solid 1px rgba(0, 0, 0, 0.04);
    border-radius: 0.75em;
    margin-bottom: 2px;
    cursor: pointer;
    z-index: 1;
    transition: all 0.1s ease-in-out 0s;
  }

  .u-cover:hover img {
    margin-top: -10px;
    margin-bottom: 10px;
    padding-bottom: 0;
  }

  .u-cover:hover .covershadow {
    display: block;
    opacity: 1;
  }

  .u-cover .bottom {
    position: absolute;
    z-index: 2;
    cursor: pointer;
    opacity: 0;
    transition: all 0.2s ease 0s;
    display: flex;
    align-items: center;
    justify-content: center;
    color: #fff;
    backdrop-filter: blur(8px);
    background: hsla(0, 0%, 100%, 0.14);
    border: 1px solid hsla(0, 0%, 100%, 0.08);
    height: 22%;
    width: 22%;
    border-radius: 50%;
  }

  .u-cover .bottom svg {
    height: 30px;
    width: 30px;
    fill: rgba(200, 200, 200, 0.5);
    stroke-width: 1;
    stroke: #fff;
  }

  .u-cover:hover .bottom { opacity: 1; }
  .u-cover:hover .bottom:hover { background: hsla(0, 0%, 100%, 0.28); }

  .cover-placeholder {
    width: 100%;
    aspect-ratio: 1;
    background: var(--button-background-color);
    border-radius: 0.75em;
  }

  .desc {
    cursor: default;
    padding-top: 8px;
    height: 65px;
  }

  .desc .title {
    word-break: break-all;
    font-size: 16px;
    font-weight: 600;
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
    line-height: 20px;
    color: var(--text-default-color);
    margin: 0 0 5px;
    z-index: 1;
  }

  .skeleton {
    width: 100%; height: 100%;
    background: linear-gradient(90deg, var(--button-background-color) 25%, var(--button-hover-background-color) 50%, var(--button-background-color) 75%);
    background-size: 200% 100%;
    animation: shimmer 1.5s infinite;
  }

  .title-skeleton {
    height: 28px;
    border-radius: 4px;
    background: linear-gradient(90deg, var(--button-background-color) 25%, var(--button-hover-background-color) 50%, var(--button-background-color) 75%);
    background-size: 200% 100%;
    animation: shimmer 1.5s 0.2s infinite;
    margin-top: 6px;
  }

  @keyframes shimmer {
    0% { background-position: 200% 0; }
    100% { background-position: -200% 0; }
  }

  /* Load more */
  .loading_bottom { text-align: center; padding: 32px; }

  .btn-load-more {
    padding: 10px 28px;
    font-size: 14px;
    font-weight: 600;
    background: var(--button-background-color);
    border: 1px solid var(--button-border-color);
    border-radius: 999px;
    color: var(--text-default-color);
    cursor: pointer;
    min-width: unset; min-height: unset;
    transition: transform var(--dur-fast) var(--ease-spring),
                background var(--dur-fast) var(--ease-soft),
                box-shadow var(--dur-fast) var(--ease-soft);
    opacity: 0.9;
  }

  .btn-load-more:hover {
    transform: translateY(-2px);
    background: var(--button-hover-background-color);
    box-shadow: var(--shadow-sm);
    opacity: 1;
  }
  .btn-load-more:active { transform: translateY(0); }

  .empty-tip { text-align: center; padding: 80px; color: var(--text-subtitle-color); }

  /* Searchbox — matches original: margin-bottom accounts for floating player */
  .searchbox {
    margin-bottom: 150px;
    transition: 0.3s;
  }
</style>
