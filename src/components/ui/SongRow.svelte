<script lang="ts">
  import { playerState } from "../../lib/stores/player";
  import { settings } from "../../lib/stores/settings";
  import { MediaService, myplaylistLib } from "../../lib/providers/index";
  import { localmusic } from "../../lib/providers/localmusic";
  import { proxyResourceUrl } from "../../lib/resourceUrl";
  import { downloadTrackFile, type DownloadMetadata } from "../../lib/tauri";
  import { toast } from "../../lib/stores/toast";
  import { runOnActionKey } from "../../lib/keyboard";
  import type { Track } from "../../lib/stores/player";

  let { track, index, onPlay, showRemove = false, showSourceBadge = false, onRemove }: {
    track: Track; index?: number; onPlay: () => void;
    showRemove?: boolean; showSourceBadge?: boolean; onRemove?: () => void;
  } = $props();

  let showMenu = $state(false);
  let menuX = $state(0);
  let menuY = $state(0);
  let myPls = $state<Array<{ id: string; title: string }>>([]);
  let createMode = $state(false);
  let createTitle = $state("");
  let downloading = $state(false);

  let isPlaying = $derived($playerState.currentTrack?.id === track.id && $playerState.playing);
  let isCurrent = $derived($playerState.currentTrack?.id === track.id);
  let isUnavailable = $derived(Boolean(track.disabled));
  let isLocalTrack = $derived(track.source === "localmusic" || track.id.startsWith("lmtrack_"));
  let canDownload = $derived(!isLocalTrack);
  let thumbUrl = $derived(proxyResourceUrl(track.img_url));

  function openMenu(e: MouseEvent) {
    e.preventDefault(); e.stopPropagation();
    showMenu = true;
    myPls = myplaylistLib.show("my").map((p) => ({ id: p.info.id, title: p.info.title }));
    menuX = Math.min(e.clientX, window.innerWidth - 200);
    menuY = Math.min(e.clientY, window.innerHeight - 220);
  }

  function addToQueue() {
    (window as unknown as { l1Player?: { insertTrack: (t: Track) => void } }).l1Player?.insertTrack(track);
    showMenu = false;
  }

  function addToPlaylist(id: string) { MediaService.addTrackToMyPlaylist(id, track); showMenu = false; }

  function createAndAdd() {
    const title = createTitle.trim();
    if (!title) return;
    MediaService.createMyPlaylist(title, track);
    createTitle = "";
    createMode = false;
    showMenu = false;
  }

  function downloadFileName() {
    const artist = track.artist?.trim();
    const title = track.title?.trim() || "Listen1 Track";
    return artist ? `${artist} - ${title}` : title;
  }

  async function downloadMetadata(sourceTrack: Track): Promise<DownloadMetadata> {
    let lyrics = sourceTrack.lyric ?? "";
    if (!lyrics && sourceTrack.id) {
      const lyricResult = await MediaService.getLyric(sourceTrack.id, sourceTrack.album_id ?? "", sourceTrack.lyric_url, sourceTrack.tlyric_url).catch(() => null);
      lyrics = [lyricResult?.lyric ?? "", lyricResult?.tlyric ?? ""].filter(Boolean).join("\n");
    }
    return {
      title: sourceTrack.title,
      artist: sourceTrack.artist,
      album: sourceTrack.album,
      lyrics,
      cover_url: sourceTrack.img_url,
    };
  }

  async function handleDownload(e?: MouseEvent) {
    e?.stopPropagation();
    if (downloading || isUnavailable) return;

    downloading = true;
    try {
      let url = track.url || track.sound_url || "";
      let metadataTrack = track;
      if (!url) {
        const result = await MediaService.getUrl(track.id);
        url = result?.url ?? "";
        if (result?.track) metadataTrack = { ...track, ...result.track };
      }
      if (!url) throw new Error("no playable url");

      const result = await downloadTrackFile(
        url,
        downloadFileName(),
        $settings.downloadDir || undefined,
        await downloadMetadata(metadataTrack)
      );
      const localResult = await localmusic.refreshDownloadedTrack(result.path).catch((error) => {
        console.warn("[SongRow] refresh downloaded local track failed", error);
        return null;
      });
      if (localResult) {
        window.dispatchEvent(new CustomEvent("listen1-local-music-updated", {
          detail: { path: result.path, track: localResult.track, stats: localResult },
        }));
      }
      if (result.metadata_error) {
        console.warn("[SongRow] metadata write failed", result.metadata_error);
        toast.warn(`已下载：${result.file_name}，元数据写入失败`);
      } else {
        toast.success(`已下载：${result.file_name}，已更新本地音乐`);
      }
    } catch (error) {
      console.error("[SongRow] download failed", error);
      toast.error("下载失败");
    } finally {
      downloading = false;
    }
  }
</script>

<svelte:window onclick={() => { if (showMenu) showMenu = false; }} />

<li class="isSearchType"
  class:playing={isCurrent}
  class:disabled={isUnavailable}
  ondblclick={!isUnavailable ? onPlay : undefined}
  oncontextmenu={openMenu}
  role="row" tabindex="0"
  onkeydown={(e) => e.key === "Enter" && !isUnavailable && onPlay()}>

  <!-- Album thumb -->
  <button type="button" class="thumb-button" onclick={() => !isUnavailable && onPlay()} disabled={isUnavailable}>
    {#if thumbUrl}
      <img src={thumbUrl} alt="" />
    {:else}
      <span class="thumb-placeholder"></span>
    {/if}
  </button>

  <!-- Title + artist -->
  <div class="title-and-artist">
    <div class="container">
      <div class="title truncate" class:disabled-text={track.disabled}>
        {#if showSourceBadge && track.source}
          <span class="source-badge">{track.source}</span>
        {/if}
        <span class="track-title-text" onclick={() => !isUnavailable && onPlay()} role="button" tabindex="0" onkeydown={(e) => runOnActionKey(e, () => { if (!isUnavailable) onPlay(); })}>{track.title}</span>
      </div>
      <div class="artist truncate">
        {track.artist}
      </div>
    </div>
  </div>

  <div class="album truncate">
    {track.album ?? ""}
  </div>

  <!-- Tools -->
  <div class="tools">
    <span class="icon" onclick={(e) => { e.stopPropagation(); addToQueue(); }}
      role="button" tabindex="0" onkeydown={(e) => runOnActionKey(e, addToQueue)} title="加入播放队列">
      <svg width="16" height="16" viewBox="0 0 24 24"><path d="M12 5v14M5 12h14"/></svg>
    </span>
    <span class="icon" onclick={(e) => openMenu(e as unknown as MouseEvent)}
      role="button" tabindex="0" onkeydown={(e) => runOnActionKey(e, () => openMenu(e as unknown as MouseEvent))} title="添加到歌单">
      <svg width="16" height="16" viewBox="0 0 24 24">
        <path d="M19 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11l5 5v4"/>
        <path d="M17 14v6M14 17h6"/>
      </svg>
    </span>
    {#if canDownload}
      <span class="icon" class:busy={downloading} onclick={handleDownload}
        role="button" tabindex="0" onkeydown={(e) => runOnActionKey(e, () => handleDownload())} title="下载">
        {#if downloading}
          <span class="mini-spinner"></span>
        {:else}
          <svg width="16" height="16" viewBox="0 0 24 24">
            <path d="M12 3v12M7 10l5 5 5-5"/>
            <path d="M5 21h14"/>
          </svg>
        {/if}
      </span>
    {/if}
    {#if showRemove && onRemove}
      <span class="icon" onclick={(e) => { e.stopPropagation(); onRemove?.(); }}
        role="button" tabindex="0" onkeydown={(e) => runOnActionKey(e, () => onRemove?.())} title="移除">
        <svg width="16" height="16" viewBox="0 0 24 24"><path d="M18 6L6 18M6 6l12 12"/></svg>
      </span>
    {/if}
    {#if track.source_url}
      <span class="icon" onclick={(e) => { e.stopPropagation(); window.open(track.source_url, "_blank"); }}
        role="button" tabindex="0" onkeydown={(e) => runOnActionKey(e, () => window.open(track.source_url, "_blank"))} title="在来源打开">
        <svg width="16" height="16" viewBox="0 0 24 24">
          <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/>
          <polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/>
        </svg>
      </span>
    {/if}
  </div>

  {#if isCurrent}
    <div class="row-spectrum" class:active={isPlaying} aria-hidden="true">
      <span></span><span></span><span></span><span></span><span></span><span></span>
    </div>
  {/if}
</li>

{#if showMenu}
  <div class="ctx-menu" style="top:{menuY}px;left:{menuX}px"
    role="menu" tabindex="0"
    onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.stopPropagation()}>
    <div class="menu-item primary" onclick={() => { onPlay(); showMenu = false; }} role="menuitem" tabindex="0" onkeydown={(e) => runOnActionKey(e, () => { onPlay(); showMenu = false; })}>
      <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor" style="stroke:none"><path d="M8 5v14l11-7z"/></svg>
      立即播放
    </div>
    <div class="menu-item" onclick={addToQueue} role="menuitem" tabindex="0" onkeydown={(e) => runOnActionKey(e, addToQueue)}>加入播放队列</div>
    <div class="menu-sep"></div>
    {#if myPls.length > 0}
      <div class="menu-label">添加到歌单</div>
      {#each myPls as pl}
        <div class="menu-item sub" onclick={() => addToPlaylist(pl.id)} role="menuitem" tabindex="0" onkeydown={(e) => runOnActionKey(e, () => addToPlaylist(pl.id))}>{pl.title}</div>
      {/each}
    {/if}
    {#if createMode}
      <div class="menu-create">
        <input
          bind:value={createTitle}
          placeholder="歌单名称"
          onkeydown={(e) => {
            if (e.key === "Enter") createAndAdd();
            if (e.key === "Escape") createMode = false;
          }}
        />
        <button type="button" onclick={createAndAdd}>创建</button>
      </div>
    {:else}
      <div class="menu-item sub" onclick={() => createMode = true} role="menuitem" tabindex="0" onkeydown={(e) => runOnActionKey(e, () => createMode = true)}>+ 新建歌单</div>
    {/if}
    {#if track.source_url}
      <div class="menu-sep"></div>
      <div class="menu-item" onclick={() => { window.open(track.source_url, "_blank"); showMenu = false; }}
        role="menuitem" tabindex="0" onkeydown={(e) => runOnActionKey(e, () => { window.open(track.source_url, "_blank"); showMenu = false; })}>在来源打开</div>
    {/if}
  </div>
{/if}

<style>
  li.isSearchType {
    position: relative;
    transition: background-color 0.18s, color 0.18s;
    display: flex;
    align-items: center;
    padding: 10px 8px 14px;
    border-radius: 12px;
    user-select: none;
    cursor: default;
    list-style: none;
  }

  li.isSearchType:hover {
    background-color: var(--songlist-hover-background-color);
  }

  li.isSearchType.playing,
  li.isSearchType.playing:hover {
    background-color: color-mix(in srgb, var(--theme-color) 14%, transparent);
    color: var(--text-default-color);
  }

  .thumb-button {
    all: unset;
    display: block;
    height: 46px;
    width: 46px;
    margin-right: 20px;
    flex-shrink: 0;
    cursor: pointer;
  }

  .thumb-button:disabled {
    cursor: default;
  }

  .thumb-button img,
  .thumb-placeholder {
    display: block;
    object-fit: cover;
    border-radius: 8px;
    height: 46px;
    width: 46px;
    border: 1px solid rgba(0,0,0,0.04);
    box-shadow: none;
    transition: opacity 0.2s;
  }

  li.isSearchType:hover .thumb-button img { opacity: 0.96; }

  .thumb-placeholder { background: var(--button-background-color); }

  .title-and-artist {
    flex: 1;
    display: flex;
    min-width: 0;
  }

  .container { display: flex; flex-direction: column; min-width: 0; width: 100%; }

  .title {
    font-size: 18px;
    font-weight: 600;
    overflow: hidden;
    display: flex;
    align-items: center;
    min-width: 0;
    white-space: nowrap;
  }

  .title.disabled-text { color: var(--text-disable-color); }
  li.playing .title { color: var(--theme-color); }

  .track-title-text {
    display: block;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .source-badge {
    border: solid 1px #ccc;
    border-radius: 4px;
    margin-right: 10px;
    display: inline-block;
    padding: 0 4px;
    color: #ccc;
    font-size: 12px;
    max-width: 44px;
    text-align: center;
    white-space: nowrap;
    height: min-content;
    line-height: 16px;
    flex: 0 0 auto;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .artist {
    font-size: 13px;
    margin-top: 2px;
    opacity: 0.68;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .album {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    display: flex;
    font-size: 16px;
    opacity: 0.88;
    white-space: nowrap;
  }

  .tools {
    flex: 0 0 142px;
    display: flex;
    align-items: center;
    justify-content: flex-start;
  }

  .icon {
    height: 16px;
    width: 16px;
    color: #9d9d9d;
    margin-top: 2px;
    margin-right: 10px;
    cursor: pointer;
    display: flex;
    align-items: center;
    padding: 0;
    border-radius: 0;
    transition: color 0.18s, opacity 0.18s;
    box-sizing: content-box;
  }

  .icon:hover { color: var(--text-default-color); background: transparent; }

  .icon.busy {
    cursor: default;
    opacity: 0.8;
  }

  .mini-spinner {
    width: 14px;
    height: 14px;
    border: 2px solid rgba(127, 127, 127, 0.24);
    border-top-color: currentColor;
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .row-spectrum {
    position: absolute;
    left: 74px;
    right: 16px;
    bottom: 3px;
    height: 12px;
    display: flex;
    align-items: flex-end;
    gap: 3px;
    pointer-events: none;
    opacity: 0.95;
  }

  .row-spectrum span {
    width: 4px;
    height: 3px;
    border-radius: 999px;
    background: var(--theme-color);
    opacity: 0.35;
  }

  .row-spectrum.active span {
    animation: spectrumBeat 1.1s ease-in-out infinite;
  }

  .row-spectrum span:nth-child(2) { animation-delay: 0.12s; }
  .row-spectrum span:nth-child(3) { animation-delay: 0.24s; }
  .row-spectrum span:nth-child(4) { animation-delay: 0.08s; }
  .row-spectrum span:nth-child(5) { animation-delay: 0.19s; }
  .row-spectrum span:nth-child(6) { animation-delay: 0.31s; }

  @keyframes spectrumBeat {
    0%, 100% { height: 3px; opacity: 0.55; }
    45% { height: 12px; opacity: 1; }
  }

  @keyframes spectrumIdle {
    0%, 100% { height: 3px; opacity: 0.3; }
    50% { height: 6px; opacity: 0.5; }
  }

  /* Context menu */
  .ctx-menu {
    position: fixed;
    z-index: 500;
    background: var(--glass-bg);
    -webkit-backdrop-filter: var(--glass-blur);
    backdrop-filter: var(--glass-blur);
    border: 1px solid var(--glass-border);
    border-radius: var(--default-border-radius);
    padding: 6px;
    min-width: 200px;
    box-shadow: var(--shadow-lift);
    transform-origin: top left;
    animation: fadeInScale var(--dur-fast) var(--ease-spring);
  }

  .menu-item {
    padding: 8px 14px;
    font-size: 13px;
    border-radius: 8px;
    cursor: pointer;
    transition: background var(--dur-fast) var(--ease-soft),
                color var(--dur-fast) var(--ease-soft),
                transform var(--dur-fast) var(--ease-spring);
    color: var(--text-default-color);
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .menu-item:hover { background: var(--songlist-hover-background-color); }
  .menu-item.primary { color: var(--theme-color); }
  .menu-item.sub { padding-left: 20px; opacity: 0.8; }
  .menu-sep { height: 1px; background: var(--line-default-color); margin: 4px 0; }
  .menu-label { font-size: 11px; color: var(--link-default-color); padding: 4px 12px; }

  .menu-create {
    display: flex;
    gap: 6px;
    padding: 6px;
  }

  .menu-create input {
    flex: 1;
    min-width: 0;
    height: 30px;
    border: 1px solid var(--line-default-color);
    border-radius: 8px;
    padding: 0 8px;
    background: var(--button-background-color);
    color: var(--text-default-color);
    font-size: 13px;
  }

  .menu-create button {
    height: 30px;
    padding: 0 10px;
    border-radius: 8px;
    background: var(--theme-color);
    color: #fff;
    font-size: 12px;
    font-weight: 700;
  }
</style>
