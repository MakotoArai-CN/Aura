<script lang="ts">
  import { onMount } from "svelte";
  import { MediaService } from "../../lib/providers/index";
  import { localmusic } from "../../lib/providers/localmusic";
  import { player } from "../../lib/player";
  import { cssImageUrl, proxyResourceUrl } from "../../lib/resourceUrl";
  import SongRow from "../ui/SongRow.svelte";
  import { runOnActionKey } from "../../lib/keyboard";
  import { infiniteScroll } from "../../lib/infiniteScroll";
  import type { Playlist } from "../../lib/providers/types";
  import type { Track } from "../../lib/stores/player";

  let { navigate, listId }: { navigate: (v: unknown) => void; listId: string } = $props();

  let playlist = $state<Playlist | null>(null);
  let loading = $state(true);
  let loadError = $state("");
  let isMine = $state(false);
  let isLocal = $state(false);
  let isFavorite = $state(false);
  let showEdit = $state(false);
  let editTitle = $state("");
  let editCover = $state("");
  let playlistQuery = $state("");
  let playlistCoverUrl = $derived(proxyResourceUrl(playlist?.info.cover_img_url));
  const RENDER_STEP = 80;
  let renderLimit = $state(RENDER_STEP);
  let filteredTracks = $derived.by(() => {
    const tracks = playlist?.tracks ?? [];
    const keyword = playlistQuery.trim().toLowerCase();
    return tracks
      .map((track, idx) => ({ track, idx }))
      .filter(({ track }) => {
        if (!keyword) return true;
        return [track.title, track.artist, track.album]
          .filter(Boolean)
          .some((value) => String(value).toLowerCase().includes(keyword));
      });
  });
  // 歌单一次性全量返回（provider 无服务端分页），此处做客户端渐进渲染：先渲染前 N 首，触底再加 N。
  let visibleTracks = $derived(filteredTracks.slice(0, renderLimit));
  let hasMoreToRender = $derived(renderLimit < filteredTracks.length);

  async function loadPlaylist(useCache = true) {
    loading = true;
    loadError = "";
    MediaService.getPlaylist(listId, useCache).then((pl) => {
      playlist = pl;
      if (pl) {
        isMine = pl.info.id.slice(0, 2) === "my";
        isLocal = pl.info.id.slice(0, 2) === "lm";
        isFavorite = MediaService.isFavorite(listId);
        editTitle = pl.info.title;
        editCover = pl.info.cover_img_url ?? "";
      }
      loading = false;
    }).catch((err) => {
      console.error("[PlaylistView] playlist load failed", err);
      loadError = "加载失败";
      playlist = null;
      loading = false;
    });
  }

  $effect(() => {
    listId;
    void loadPlaylist();
  });

  // 切歌单或改搜索关键词时重置渐进渲染上限。
  $effect(() => {
    listId;
    playlistQuery;
    renderLimit = RENDER_STEP;
  });

  onMount(() => {
    function handleLocalMusicUpdated() {
      if (!listId.startsWith("lm")) return;
      void loadPlaylist(false);
    }
    window.addEventListener("listen1-local-music-updated", handleLocalMusicUpdated);
    return () => window.removeEventListener("listen1-local-music-updated", handleLocalMusicUpdated);
  });

  function playAll() {
    if (!playlist?.tracks?.length) return;
    player.setPlaylist(playlist.tracks);
    player.loadByIndex(0);
  }

  function playTrack(track: Track) {
    if (!playlist?.tracks) return;
    player.setPlaylist(playlist.tracks);
    player.playById(track.id);
  }

  function addAllToQueue() {
    if (playlist?.tracks) player.appendTracks(playlist.tracks);
  }

  async function addLocalTracks() {
    const tracks = await localmusic.openFilePicker();
    if (!tracks.length) return;
    const next = await MediaService.getPlaylist(listId, false);
    if (next) playlist = next;
  }

  async function toggleFavorite() {
    if (isFavorite) { MediaService.removeMyPlaylist(listId, "favorite"); isFavorite = false; }
    else { await MediaService.clonePlaylist(listId, "favorite"); isFavorite = true; }
  }

  async function cloneToMy() {
    await MediaService.clonePlaylist(listId, "my");
  }

  function removeTrack(track: Track, idx: number) {
    if (!playlist) return;
    MediaService.removeTrackFromMyPlaylist(listId, track.id);
    playlist = { ...playlist, tracks: playlist.tracks.filter((_, i) => i !== idx) };
  }

  function deletePlaylist() {
    if (!confirm(`确认删除「${playlist?.info.title}」？`)) return;
    MediaService.removeMyPlaylist(listId, "my");
    navigate({ type: "discover" });
  }

  function saveEdit() {
    MediaService.editMyPlaylist(listId, editTitle, editCover);
    if (playlist) playlist = { ...playlist, info: { ...playlist.info, title: editTitle, cover_img_url: editCover } };
    showEdit = false;
  }
</script>

<div class="page">
  {#if loading}
    <div class="loading-state">加载中...</div>
  {:else if playlist}
    <div class="playlist-detail">
      <!-- Header -->
      <div class="detail-head">
        <div class="detail-head-cover">
          {#if playlistCoverUrl}
            <div class="covershadow" style:background-image={cssImageUrl(playlist.info.cover_img_url)} style:opacity={1}></div>
            <img src={playlistCoverUrl} alt={playlist.info.title} />
          {:else}
            <div class="cover-placeholder">
              <svg width="40" height="40" viewBox="0 0 24 24" style="opacity:0.3">
                <path d="M9 18V5l12-2v13"/><circle cx="6" cy="18" r="3"/>
              </svg>
            </div>
          {/if}
          <div class="bottom" onclick={playAll} role="button" tabindex="0" onkeydown={(e) => runOnActionKey(e, playAll)}>
            <svg viewBox="0 0 24 24" fill="currentColor" stroke="none">
              <path d="M8 5v14l11-7z"/>
            </svg>
          </div>
        </div>

        <div class="detail-head-title">
          <h2>{playlist.info.title}</h2>

          <!-- Button list (matches original .playlist-button-list) -->
          <div class="playlist-button-list">
            {#if isLocal}
              <div class="playlist-button local-action-button" onclick={playAll} role="button" tabindex="0" onkeydown={(e) => runOnActionKey(e, playAll)}>
                <div class="play-list">
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="var(--theme-color)" style="stroke:none;flex:0 0 14px;margin-right:8px">
                    <path d="M8 5v14l11-7z"/>
                  </svg>
                  播放
                </div>
              </div>
            {:else}
              <div class="playlist-button playadd-button">
                <div class="play-list" onclick={playAll} role="button" tabindex="0" onkeydown={(e) => runOnActionKey(e, playAll)}>
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="var(--theme-color)" style="stroke:none;flex:0 0 14px;margin-right:4px">
                    <path d="M8 5v14l11-7z"/>
                  </svg>
                  播放
                </div>
                <div class="add-list" onclick={addAllToQueue} role="button" tabindex="0" onkeydown={(e) => runOnActionKey(e, addAllToQueue)} title="加入队列">
                  <svg width="14" height="14" viewBox="0 0 24 24" style="color:var(--theme-color)">
                    <path d="M12 5v14M5 12h14"/>
                  </svg>
                </div>
              </div>
            {/if}

            {#if isLocal}
              <div class="playlist-button local-action-button" onclick={addLocalTracks} role="button" tabindex="0" onkeydown={(e) => runOnActionKey(e, addLocalTracks)}>
                <div class="play-list">
                  <svg width="14" height="14" viewBox="0 0 24 24" style="color:var(--theme-color);flex:0 0 14px;margin-right:8px">
                    <path d="M12 5v14M5 12h14"/>
                  </svg>
                  添加本地音乐
                </div>
              </div>
            {/if}

            {#if !isMine && !isLocal}
              <div class="playlist-button clone-button" onclick={cloneToMy} role="button" tabindex="0" onkeydown={(e) => runOnActionKey(e, cloneToMy)}>
                <div class="play-list">
                  <svg width="16" height="16" viewBox="0 0 24 24" style="flex:0 0 16px;margin-right:8px">
                    <path d="M19 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11l5 5v11a2 2 0 0 1-2 2z"/>
                  </svg>
                  保存
                </div>
              </div>

              <div class="playlist-button fav-button" class:favorited={isFavorite} onclick={toggleFavorite} role="button" tabindex="0" onkeydown={(e) => runOnActionKey(e, toggleFavorite)}>
                <div class="play-list" class:favorited={isFavorite} class:notfavorite={!isFavorite}>
                  <svg width="16" height="16" viewBox="0 0 24 24" style="flex:0 0 16px;margin-right:8px" fill={isFavorite ? "rgb(102,102,102)" : "none"}>
                    <path d="M20.84 4.61a5.5 5.5 0 0 0-7.78 0L12 5.67l-1.06-1.06a5.5 5.5 0 0 0-7.78 7.78l1.06 1.06L12 21.23l7.78-7.78 1.06-1.06a5.5 5.5 0 0 0 0-7.78z"/>
                  </svg>
                  {isFavorite ? "已收藏" : "收藏"}
                </div>
              </div>
            {/if}

            {#if isMine}
              <div class="playlist-button edit-button" onclick={() => showEdit = true} role="button" tabindex="0" onkeydown={(e) => runOnActionKey(e, () => showEdit = true)}>
                <div class="play-list">
                  <svg width="16" height="16" viewBox="0 0 24 24" style="flex:0 0 16px;margin-right:8px">
                    <path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"/>
                    <path d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z"/>
                  </svg>
                  编辑
                </div>
              </div>
              <div class="playlist-button edit-button" onclick={deletePlaylist} role="button" tabindex="0" onkeydown={(e) => runOnActionKey(e, deletePlaylist)}>
                <div class="play-list" style="color:#ff4444">
                  <svg width="16" height="16" viewBox="0 0 24 24" style="flex:0 0 16px;margin-right:8px;stroke:#ff4444">
                    <polyline points="3 6 5 6 21 6"/>
                    <path d="M19 6l-1 14H6L5 6"/>
                  </svg>
                  删除
                </div>
              </div>
            {/if}

            {#if playlist.info.source_url}
              <a class="playlist-button" href={playlist.info.source_url} target="_blank">
                <div class="play-list">
                  <svg width="16" height="16" viewBox="0 0 24 24" style="flex:0 0 16px;margin-right:8px">
                    <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/>
                    <polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/>
                  </svg>
                  来源
                </div>
              </a>
            {/if}
          </div>
        </div>
      </div>

      <!-- Track list -->
      <ul class="detail-songlist playlist-songlist">
        {#if playlist.tracks.length > 0}
          <div class="playlist-search">
            <svg fill="currentColor" style="opacity:0.28;margin-right:4px;width:15px;height:15px;cursor:default" viewBox="0 0 24 24">
              <circle cx="11" cy="11" r="8"/><path d="m21 21-4.35-4.35"/>
            </svg>
            <input
              class="playlist-search-input"
              type="text"
              bind:value={playlistQuery}
              placeholder="搜索歌单"
            />
          </div>
        {/if}
        {#each visibleTracks as { track, idx } (`${track.id}-${idx}`)}
          <SongRow {track} index={idx + 1}
            onPlay={() => playTrack(track)}
            showRemove={isMine || isLocal}
            onRemove={() => removeTrack(track, idx)}
          />
        {/each}
        {#if hasMoreToRender}
          <li
            class="playlist-render-sentinel"
            use:infiniteScroll={{ onLoadMore: () => { renderLimit = Math.min(renderLimit + RENDER_STEP, filteredTracks.length); } }}
          ></li>
        {/if}
        {#if visibleTracks.length === 0}
          <li class="playlist-empty-row">
            <div>{isLocal ? "暂无本地音乐" : "暂无歌曲"}</div>
            {#if isLocal}
              <button type="button" class="empty-add-button" onclick={addLocalTracks}>添加本地音乐</button>
            {/if}
          </li>
        {/if}
      </ul>
    </div>
  {:else}
    <div class="empty-state">{loadError || "暂无内容"}</div>
  {/if}
</div>

<!-- Edit dialog -->
{#if showEdit}
  <div class="shadow" onclick={() => showEdit = false} role="none">
    <div class="dialog" onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.stopPropagation()} role="dialog" tabindex="-1" aria-label="编辑歌单">
      <div class="dialog-header">
        编辑歌单
        <span class="dialog-close" onclick={() => showEdit = false} role="button" tabindex="0" onkeydown={(e) => runOnActionKey(e, () => showEdit = false)}>×</span>
      </div>
      <div class="dialog-body">
        <div style="margin-bottom:16px">
          <div style="font-size:12px;color:var(--link-default-color);margin-bottom:6px">名称</div>
          <input type="text" bind:value={editTitle} class="dialog-input" />
        </div>
        <div style="margin-bottom:24px">
          <div style="font-size:12px;color:var(--link-default-color);margin-bottom:6px">封面链接</div>
          <input type="text" bind:value={editCover} class="dialog-input" placeholder="https://..." />
        </div>
        <div style="display:flex;gap:8px;justify-content:flex-end">
          <button class="dialog-btn" onclick={() => showEdit = false}>取消</button>
          <button class="dialog-btn primary" onclick={saveEdit}>保存</button>
        </div>
      </div>
    </div>
  </div>
{/if}

<style>
  .page { padding-bottom: 120px; }

  .loading-state { text-align: center; padding: 80px; color: var(--link-default-color); }

  .empty-state { text-align: center; padding: 80px; color: var(--text-subtitle-color); }

  .playlist-detail { padding-bottom: 37px; }

  .playlist-detail .detail-head {
    display: flex;
    margin-top: 11px;
    margin-bottom: 72px;
  }

  .detail-head-cover {
    position: relative;
    margin-left: 2vw;
    display: flex;
    justify-content: center;
    align-items: center;
    z-index: 1;
    height: 25vh;
    width: 25vh;
  }

  .detail-head-cover img,
  .cover-placeholder {
    position: relative;
    z-index: 1;
    height: 100%;
    width: 100%;
    border-radius: 0.75em;
    user-select: none;
    aspect-ratio: 1 / 1;
    object-fit: cover;
    border: solid 1px rgba(0, 0, 0, 0.04);
  }

  .cover-placeholder {
    background: var(--button-background-color);
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .detail-head-cover .covershadow {
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

  .detail-head-cover .bottom {
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

  .detail-head-cover:hover .bottom {
    opacity: 1;
  }

  .detail-head-cover:hover .bottom:hover {
    background: hsla(0, 0%, 100%, 0.28);
  }

  .detail-head-cover .bottom svg {
    height: 44%;
    width: 44%;
    margin-left: 4px;
  }

  .detail-head-title {
    flex: 1;
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    margin-left: 56px;
    margin-right: 2vw;
  }

  .detail-head-title h2 {
    font-size: 36px;
    font-weight: 700;
    margin-top: 10px;
    letter-spacing: 0;
  }

  .playlist-button-list {
    display: flex;
    flex-flow: row wrap;
  }

  .playlist-button {
    margin-top: 10px;
    min-height: 42px;
    cursor: pointer;
    display: flex;
    margin-right: 16px;
    border-radius: 8px;
    padding: 0 18px;
    width: auto;
    background-color: var(--button-background-color);
    color: var(--text-default-color);
    text-decoration: none;
    transition: 0.2s;
  }

  .playlist-button.favorited {
    background-color: var(--theme-color-hover);
    color: var(--theme-color);
  }

  .playlist-button:hover {
    background-color: var(--button-hover-background-color);
  }

  .playlist-button.playadd-button {
    flex: 0 0 156px;
    padding: 0;
    min-height: 42px;
    background-color: var(--theme-color-hover);
    color: var(--theme-color);
  }

  .playlist-button.local-action-button {
    flex: 0 0 auto;
    min-width: 128px;
    min-height: 42px;
    padding: 0 20px;
    background-color: var(--theme-color-hover);
    color: var(--theme-color);
  }

  .play-list {
    flex: 1;
    display: flex;
    align-items: center;
    font-size: 16px;
    line-height: 18px;
    font-weight: 700;
    user-select: none;
    color: inherit;
  }

  .playlist-button.playadd-button .play-list {
    padding: 0 0 0 18px;
    width: 112px;
  }

  .add-list {
    flex: 0 0 44px;
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 42px;
    cursor: pointer;
    transition: 0.2s;
    padding: 0 14px 0 0;
  }

  .playlist-button.clone-button,
  .playlist-button.edit-button,
  .playlist-button.fav-button {
    flex: 0 0 auto;
  }

  ul.detail-songlist {
    list-style: none;
    position: relative;
  }

  .detail-songlist.playlist-songlist {
    margin: 0 2vw;
    padding-top: 13px;
    transition: 0.3s;
  }

  ul.detail-songlist .playlist-search {
    position: absolute;
    right: 0;
    top: -50px;
    display: flex;
    width: 200px;
    height: 32px;
    background: var(--songlist-odd-background-color);
    border-style: none;
    border-radius: var(--default-border-radius);
    padding-left: 10px;
    margin-right: 40px;
    align-items: center;
  }

  ul.detail-songlist .playlist-search .playlist-search-input {
    width: 174px;
    background-color: transparent;
    border-style: none;
    font-size: 16px;
    font-weight: 600;
    color: var(--text-default-color);
  }

  ul.detail-songlist .playlist-search:hover,
  ul.detail-songlist .playlist-search:active {
    background-color: var(--search-input-background-color);
  }

  .playlist-empty-row {
    list-style: none;
    min-height: 160px;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 14px;
    color: var(--text-subtitle-color);
    font-size: 14px;
  }

  .empty-add-button {
    border: 0;
    min-width: 136px;
    min-height: 42px;
    padding: 0 20px;
    border-radius: 8px;
    background: var(--theme-color-hover);
    color: var(--theme-color);
    font-size: 15px;
    font-weight: 700;
    cursor: pointer;
  }

  .empty-add-button:hover {
    transform: translateY(-1px);
  }

  /* Dialog */
  .shadow {
    position: fixed;
    background: rgba(0,0,0,0.5);
    z-index: 9999;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    backdrop-filter: blur(10px);
    -webkit-backdrop-filter: blur(10px);
    animation: fadeIn var(--dur-med) var(--ease-soft);
  }

  .dialog {
    width: 440px;
    background: var(--dialog-background-color);
    border-radius: 16px;
    padding: 24px;
    color: var(--dialog-text-color);
    border: 1px solid var(--glass-border);
    box-shadow: var(--shadow-lift);
    animation: fadeInScale var(--dur-slow) var(--ease-spring);
  }

  .dialog-header {
    font-size: 16px;
    font-weight: bold;
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 20px;
    border-bottom: 1px solid var(--line-default-color);
    padding-bottom: 12px;
  }

  .dialog-close { cursor: pointer; font-size: 24px; opacity: 0.6; line-height: 1; }
  .dialog-close:hover { opacity: 1; }
  .dialog-input {
    width: 100%;
    padding: 8px 12px;
    background: var(--button-background-color);
    border: 1px solid var(--line-default-color);
    border-radius: 8px;
    color: var(--text-default-color);
    font-size: 14px;
  }

  .dialog-input:focus { border-color: var(--theme-color); background: var(--theme-color-hover); color: var(--theme-color); }

  .dialog-btn {
    padding: 8px 20px;
    border-radius: 8px;
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    background: var(--button-background-color);
    border: 1px solid var(--button-border-color);
    color: var(--text-default-color);
    min-width: unset; min-height: unset;
    transition: all 0.2s;
  }

  .dialog-btn:hover { transform: scale(1.05); background: var(--button-hover-background-color); }
  .dialog-btn.primary { background: var(--theme-color); border-color: var(--theme-color); color: #fff; }
  .dialog-btn.primary:hover { background: var(--accent-hover); }
</style>
