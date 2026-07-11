<script lang="ts">
  import { myplaylistLib, MediaService } from "../../lib/providers/index";
  import { toast } from "../../lib/stores/toast";
  import { runOnActionKey } from "../../lib/keyboard";

  let {
    navigate,
    activeView,
  }: {
    navigate: (v: unknown) => void;
    activeView: { type: string; id?: string; source?: string };
  } = $props();

  const LOCAL_PLAYLIST_ID = "lmplaylist_reserve";

  let open = $state(true);
  let myPls = $state(myplaylistLib.show("my"));
  let favPls = $state(myplaylistLib.show("favorite"));
  let showCreate = $state(false);
  let showImport = $state(false);
  let newPlaylistTitle = $state("");
  let importPlaylistUrl = $state("");
  let importingPlaylist = $state(false);

  $effect(() => {
    const id = setInterval(() => {
      myPls = myplaylistLib.show("my");
      favPls = myplaylistLib.show("favorite");
    }, 1000);
    return () => clearInterval(id);
  });

  function createPlaylist() {
    const title = newPlaylistTitle.trim();
    if (!title) return;
    myplaylistLib.create("my", title);
    myPls = myplaylistLib.show("my");
    newPlaylistTitle = "";
    showCreate = false;
  }

  async function importPlaylist() {
    const url = importPlaylistUrl.trim();
    if (!url || importingPlaylist) return;
    importingPlaylist = true;
    try {
      const parsed = await MediaService.parseURL(url);
      if (!parsed?.id) {
        toast.error("未识别此歌单链接");
        return;
      }
      const playlist = await MediaService.getPlaylist(parsed.id, false);
      if (!playlist || playlist.tracks.length === 0) {
        toast.error("歌单为空或读取失败");
        return;
      }
      const id = MediaService.saveExternalPlaylist("my", playlist);
      myPls = myplaylistLib.show("my");
      importPlaylistUrl = "";
      showImport = false;
      navigate({ type: "playlist", id });
      toast.success(`歌单已导入 ${playlist.tracks.length} 首`);
    } catch (error) {
      console.error("[Sidebar] import playlist failed", error);
      toast.error("导入歌单失败");
    } finally {
      importingPlaylist = false;
    }
  }

  function removeMyPl(id: string, e: MouseEvent) {
    e.stopPropagation();
    if (!confirm("确认删除此歌单？")) return;
    MediaService.removeMyPlaylist(id, "my");
    myPls = myplaylistLib.show("my");
    if (activeView.type === "playlist" && activeView.id === id) navigate({ type: "discover" });
  }
</script>

<div class="sidebar-content" class:opensidebar={open}>
  <!-- Logo / toggle -->
  <div class="logo-content" onclick={() => open = !open} role="button" tabindex="0" onkeydown={(e) => runOnActionKey(e, () => open = !open)}>
    <div class="logo-mark" aria-hidden="true">
      <!-- <svg width="30" height="30" viewBox="0 0 48 48" fill="none">
        <rect x="3" y="3" width="42" height="42" rx="10.5" fill="#111827"/>
        <circle cx="24" cy="24" r="13.2" fill="#0B1120" stroke="#FFFFFF" stroke-opacity="0.14" stroke-width="1.4"/>
        <path d="M16.8 16.2c4.2-3 10-3.2 14.6-.6M14.6 22c6.3-6 16.2-6.2 22.7-.4M14.8 27.7c6.4 5.9 16.3 5.7 22.5-.5" stroke="url(#sidebar-logo-mark)" stroke-width="2.4" stroke-linecap="round"/>
        <path d="M21.1 17.9 31.2 24l-10.1 6.1V17.9Z" fill="#F8FAFC"/>
        <circle cx="14.1" cy="24" r="1.9" fill="#7DD3FC"/>
        <circle cx="24" cy="11.6" r="1.8" fill="#34D399"/>
        <circle cx="35.9" cy="24" r="1.9" fill="#F472B6"/>
        <defs>
          <linearGradient id="sidebar-logo-mark" x1="14" y1="15" x2="37" y2="31" gradientUnits="userSpaceOnUse">
            <stop stop-color="#7DD3FC"/>
            <stop offset="0.52" stop-color="#34D399"/>
            <stop offset="1" stop-color="#F472B6"/>
          </linearGradient>
        </defs>
      </svg> -->
      <img src="/logo.png" alt="Aura" width="50" height="50" style="margin-left: 10px;"/>
    </div>
    <div class="logo-title" style="margin-left: 50px;">
      <span>Aura</span>
    </div>
  </div>

  <div class="sidebar-scroll-content">
    <div class="menu-control"></div>

    <!-- Playlist discovery -->
    <div class="menu-title">
      <span class="title" class:opensidebar={open}>歌单</span>
    </div>
    <ul>
      <li class:active={activeView.type === "discover"}>
        <button type="button" class="sidebar-nav-btn"
          onclick={() => navigate({ type: "discover", source: "netease" })}>
          <div class="sidebar-block" class:opensidebar={open}>
            <div>
              <svg width="18" height="18" viewBox="0 0 24 24">
                <rect x="3" y="4" width="7" height="7" rx="1.5"/>
                <rect x="14" y="4" width="7" height="7" rx="1.5"/>
                <rect x="3" y="15" width="7" height="5" rx="1.5"/>
                <rect x="14" y="15" width="7" height="5" rx="1.5"/>
              </svg>
            </div>
            <span class="nav-label">发现歌单</span>
          </div>
        </button>
      </li>
      <li class:active={activeView.type === "playlist" && activeView.id === LOCAL_PLAYLIST_ID}>
        <button type="button" class="sidebar-nav-btn"
          onclick={() => navigate({ type: "playlist", id: LOCAL_PLAYLIST_ID })}>
          <div class="sidebar-block" class:opensidebar={open}>
            <div>
              <svg width="18" height="18" viewBox="0 0 24 24">
                <path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>
              </svg>
            </div>
            <span class="nav-label">本地音乐</span>
          </div>
        </button>
      </li>
    </ul>

    <!-- My playlists -->
    <div class="menu-title">
      <span class="title" class:opensidebar={open}>创建的歌单</span>
      {#if open}
        <button type="button" class="sidebar-icon-btn" onclick={() => showImport = true} aria-label="导入歌单" title="导入歌单">
          <svg width="18" height="18" viewBox="0 0 24 24">
            <path d="M12 3v12M7 10l5 5 5-5"/><path d="M5 21h14"/>
          </svg>
        </button>
        <button type="button" class="sidebar-icon-btn" onclick={() => showCreate = true} aria-label="新建歌单" title="新建歌单">
          <svg width="18" height="18" viewBox="0 0 24 24">
            <path d="M12 5v14M5 12h14"/>
          </svg>
        </button>
      {/if}
    </div>
    <ul>
      {#each myPls as pl}
        <li class:active={activeView.type === "playlist" && activeView.id === pl.info.id}>
        <button type="button" class="sidebar-nav-btn"
          onclick={() => navigate({ type: "playlist", id: pl.info.id })}
          ondblclick={(e) => removeMyPl(pl.info.id, e)}
          title="双击可删除歌单">
            <div class="sidebar-block" class:opensidebar={open}>
              <div>
                <svg width="18" height="18" viewBox="0 0 24 24">
                  <circle cx="12" cy="12" r="9"/>
                  <circle cx="12" cy="12" r="2"/>
                </svg>
              </div>
              <span class="truncate">{pl.info.title}</span>
            </div>
          </button>
          {#if open}
            <button
              type="button"
              class="sidebar-remove-btn"
              onclick={(e) => removeMyPl(pl.info.id, e)}
              aria-label={`删除歌单 ${pl.info.title}`}
              title="删除歌单"
            >
              <svg width="14" height="14" viewBox="0 0 24 24"><path d="M18 6 6 18M6 6l12 12"/></svg>
            </button>
          {/if}
        </li>
      {/each}
      {#if myPls.length === 0 && open}
        <li class="empty-row">
          <div class="sidebar-empty">暂无创建的歌单</div>
        </li>
      {/if}
    </ul>

    <!-- Favorites -->
    {#if open}
      <div class="menu-title">
        <span class="title opensidebar">收藏歌单</span>
      </div>
      <ul>
        {#each favPls as pl}
          <li class:active={activeView.type === "playlist" && activeView.id === pl.info.id}>
            <button type="button" class="sidebar-nav-btn"
              onclick={() => navigate({ type: "playlist", id: pl.info.id })}>
              <div class="sidebar-block opensidebar">
                <div>
                  <svg width="18" height="18" viewBox="0 0 24 24">
                    <circle cx="12" cy="12" r="9"/>
                    <circle cx="12" cy="12" r="2"/>
                  </svg>
                </div>
                <span class="truncate">{pl.info.title}</span>
              </div>
            </button>
          </li>
        {/each}
        {#if favPls.length === 0}
          <li class="empty-row">
            <div class="sidebar-empty">暂无收藏的歌单</div>
          </li>
        {/if}
      </ul>
    {/if}

    {#if open}
      <div class="menu-title">
        <span class="title opensidebar">账号</span>
      </div>
      <ul>
        <li class:active={activeView.type === "profile"}>
          <button type="button" class="sidebar-nav-btn" onclick={() => navigate({ type: "profile", source: "netease" })}>
            <div class="sidebar-block opensidebar">
              <div>
                <svg width="18" height="18" viewBox="0 0 24 24">
                  <circle cx="12" cy="8" r="4"/><path d="M4 21a8 8 0 0 1 16 0"/>
                </svg>
              </div>
              <span class="truncate">登录账号</span>
            </div>
          </button>
        </li>
      </ul>
    {/if}
  </div>
</div>

{#if showCreate}
  <div class="sidebar-dialog-mask" onclick={() => showCreate = false} role="none">
    <div class="sidebar-dialog" onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.stopPropagation()} role="dialog" tabindex="-1" aria-label="新建歌单">
      <div class="dialog-title">新建歌单</div>
      <input
        class="dialog-input"
        bind:value={newPlaylistTitle}
        placeholder="歌单名称"
        onkeydown={(e) => {
          if (e.key === "Enter") createPlaylist();
          if (e.key === "Escape") showCreate = false;
        }}
      />
      <div class="dialog-actions">
        <button type="button" onclick={() => showCreate = false}>取消</button>
        <button type="button" class="primary" onclick={createPlaylist}>创建</button>
      </div>
    </div>
  </div>
{/if}

{#if showImport}
  <div class="sidebar-dialog-mask" onclick={() => showImport = false} role="none">
    <div class="sidebar-dialog" onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.stopPropagation()} role="dialog" tabindex="-1" aria-label="导入歌单">
      <div class="dialog-title">导入歌单</div>
      <input
        class="dialog-input"
        bind:value={importPlaylistUrl}
        placeholder="粘贴网易云 / QQ / B站 / 咪咕等歌单链接"
        onkeydown={(e) => {
          if (e.key === "Enter") importPlaylist();
          if (e.key === "Escape") showImport = false;
        }}
      />
      <div class="dialog-actions">
        <button type="button" onclick={() => showImport = false}>取消</button>
        <button type="button" class="primary" disabled={importingPlaylist} onclick={importPlaylist}>
          {importingPlaylist ? "导入中" : "导入"}
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .sidebar-content {
    height: calc(100vh - var(--player-height) - 3vh);
    overflow: hidden;
    width: var(--sidebar-collapsed);
    transition: width 0.2s;
    background: var(--sidebar-background);
    border-radius: 10px;
    cursor: default;
    display: flex;
    flex-direction: column;
    border: 0;
  }

  .sidebar-content.opensidebar { width: var(--sidebar-width); }

  .logo-content {
    display: flex;
    align-items: center;
    justify-content: flex-start;
    margin: 10px;
    margin-bottom: 0;
    padding-bottom: 10px;
    border-bottom: 1px solid var(--sidebar-splitter);
    transition: 0.2s;
    cursor: pointer;
    flex-shrink: 0;
  }

  .logo-mark {
    width: 44px;
    height: 44px;
    display: flex;
    align-items: center;
    justify-content: center;
    flex: 0 0 44px;
    transition: 0.2s;
  }

  .logo-content:hover .logo-mark { opacity: 0.86; }

  .logo-title {
    padding-right: 10px;
    display: flex;
    align-items: center;
    overflow: hidden;
  }

  .logo-title span {
    color: var(--text-default-color);
    opacity: 0;
    width: 0;
    font-size: 18px;
    font-weight: 760;
    letter-spacing: 0;
    white-space: nowrap;
    transition: 0.2s;
  }

  .opensidebar > .logo-content { border-bottom: 1px solid transparent; }
  .opensidebar > .logo-content .logo-title span { opacity: 1; width: 90px; }

  .sidebar-scroll-content {
    overflow-x: hidden;
    overflow-y: overlay;
    min-height: 0;
    flex: 1;
  }

  .sidebar-scroll-content::-webkit-scrollbar { display: none; }
  .opensidebar .sidebar-scroll-content:hover::-webkit-scrollbar { display: block; width: 2px; }

  .menu-control { height: 74px; -webkit-app-region: drag; }

  .menu-title {
    height: 34px;
    line-height: 34px;
    margin: 8px 12px;
    color: var(--link-default-color);
    padding-left: 10px;
    display: flex;
    align-items: center;
    font-size: 14px;
    letter-spacing: 0;
    text-transform: none;
  }

  .sidebar-icon-btn {
    background: none;
    border: none;
    padding: 0;
    margin-left: 0;
    margin-right: 10px;
    cursor: pointer;
    display: flex;
    align-items: center;
    color: var(--link-default-color);
    transition: 0.2s;
  }
  .sidebar-icon-btn:hover { color: var(--text-default-color); }
  .sidebar-icon-btn svg { stroke: currentColor; }

  .sidebar-nav-btn {
    all: unset;
    display: block;
    width: 100%;
    cursor: pointer;
  }

  .sidebar-remove-btn {
    all: unset;
    position: absolute;
    top: 50%;
    right: 20px;
    transform: translateY(-50%);
    display: flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    border-radius: 6px;
    color: var(--text-subtitle-color);
    cursor: pointer;
    opacity: 0;
    pointer-events: none;
    transition: opacity 0.2s, background-color 0.2s, color 0.2s;
  }

  ul li:hover .sidebar-remove-btn,
  .sidebar-remove-btn:focus-visible {
    opacity: 1;
    pointer-events: auto;
  }

  .sidebar-remove-btn:hover {
    background: var(--songlist-hover-background-color);
    color: #ff4444;
  }

  .sidebar-remove-btn:focus-visible {
    outline: 1px solid var(--theme-color);
    outline-offset: 1px;
  }

  ul li.active .sidebar-remove-btn {
    color: rgba(255, 255, 255, 0.82);
  }

  ul li.active .sidebar-remove-btn:hover {
    background: rgba(255, 255, 255, 0.18);
    color: #fff;
  }

  .sidebar-remove-btn svg {
    stroke: currentColor;
  }

  .menu-title .title {
    user-select: none;
    white-space: nowrap;
    opacity: 0;
    transition: 0.2s;
    width: 0;
    flex: 0;
  }

  .menu-title .title.opensidebar { opacity: 1; flex: 1; width: auto; }

  ul li {
    cursor: pointer;
    padding: 4px 10px;
    position: relative;
  }

  ul li .sidebar-block {
    display: flex;
    align-items: center;
    line-height: 36px;
    min-height: 54px;
    padding: 8px 14px;
    margin: 1px 0;
    transition: background-color 0.2s, color 0.2s, border-radius 0.2s;
    color: var(--sidebar-hover-text-color);
    border-radius: var(--default-border-radius);
    background-color: var(--sidebar-button-background);
    position: relative;
    overflow: hidden;
  }

  /* Collapsed sidebar shows blocks; expanded strips the background */
  ul li .sidebar-block.opensidebar {
    background-color: transparent;
  }

  ul li .sidebar-block > div {
    display: flex;
    align-items: center;
    justify-content: center;
    margin-right: 14px;
    flex-shrink: 0;
  }

  ul li span.truncate,
  ul li .nav-label {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    font-size: var(--text-default-size);
    font-weight: 600;
  }

  ul li:hover .sidebar-block {
    background: var(--theme-color-hover);
    color: var(--text-default-color);
    border-radius: 10px;
    transition: background-color 0.2s, color 0.2s, border-radius 0.2s;
  }
  ul li:hover .sidebar-block.opensidebar {
    background: var(--sidebar-button-background);
    color: var(--text-default-color);
  }

  /* Active — shows accent bar on the left */
  ul li.active .sidebar-block,
  ul li.active:hover .sidebar-block {
    background: var(--theme-color);
    color: #fff;
    border-radius: 10px;
    box-shadow: none;
  }

  ul li.active::before {
    content: none;
  }

  .opensidebar ul li.active .sidebar-block {
    background: var(--theme-color);
    color: #fff;
    box-shadow: none;
  }

  ul li.empty-row {
    padding: 0 20px;
    cursor: default;
  }

  .sidebar-empty {
    color: var(--link-default-color);
    font-size: 13px;
    line-height: 30px;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  svg { width: 22px; height: 22px; }

  .sidebar-dialog-mask {
    position: fixed;
    inset: 0;
    z-index: 800;
    background: var(--shadow-mask);
    display: flex;
    align-items: center;
    justify-content: center;
    -webkit-app-region: no-drag;
  }

  .sidebar-dialog {
    width: 360px;
    border-radius: 10px;
    padding: 20px;
    background: var(--dialog-background-color);
    border: 1px solid var(--line-default-color);
    box-shadow: var(--shadow-lift);
  }

  .dialog-title {
    font-size: 18px;
    font-weight: 700;
    margin-bottom: 14px;
  }

  .dialog-input {
    width: 100%;
    height: 42px;
    border: 1px solid var(--line-default-color);
    border-radius: 8px;
    background: var(--button-background-color);
    color: var(--text-default-color);
    padding: 0 12px;
    font-size: 15px;
  }

  .dialog-input:focus {
    border-color: var(--theme-color);
  }

  .dialog-actions {
    display: flex;
    justify-content: flex-end;
    gap: 10px;
    margin-top: 16px;
  }

  .dialog-actions button {
    height: 36px;
    min-width: 74px;
    border-radius: 8px;
    background: var(--button-background-color);
    color: var(--text-default-color);
    font-weight: 600;
  }

  .dialog-actions .primary {
    background: var(--theme-color);
    color: #fff;
  }

  @media (max-width: 720px) {
    .sidebar-content,
    .sidebar-content.opensidebar {
      width: 100%;
      height: var(--mobile-nav-height);
      border-radius: 12px;
      background: var(--nav-background-color);
      border: 1px solid var(--line-default-color);
      box-shadow: 0 10px 28px rgba(0, 0, 0, 0.22);
      -webkit-backdrop-filter: saturate(180%) blur(18px);
      backdrop-filter: saturate(180%) blur(18px);
    }

    .logo-content,
    .menu-control,
    .menu-title,
    .empty-row {
      display: none;
    }

    .sidebar-scroll-content {
      height: 100%;
      overflow: hidden;
    }

    .sidebar-scroll-content > ul {
      display: flex;
      height: 100%;
      align-items: center;
      justify-content: space-around;
      gap: 6px;
      padding: 0 8px;
    }

    .sidebar-scroll-content > ul:not(:first-of-type) {
      display: none;
    }

    ul li {
      flex: 1;
      min-width: 0;
      padding: 0;
    }

    .sidebar-nav-btn {
      height: 100%;
    }

    ul li .sidebar-block,
    ul li .sidebar-block.opensidebar {
      min-height: 44px;
      height: 44px;
      margin: 0;
      padding: 0 8px;
      justify-content: center;
      gap: 6px;
      background: transparent;
      color: var(--text-default-color);
    }

    ul li .sidebar-block > div {
      margin-right: 0;
    }

    ul li .nav-label,
    ul li span.truncate {
      display: block;
      max-width: 72px;
      font-size: 12px;
      font-weight: 600;
    }

    ul li.active .sidebar-block,
    ul li.active:hover .sidebar-block {
      background: var(--theme-color-hover);
      color: var(--theme-color);
      box-shadow: none;
    }

    svg {
      width: 20px;
      height: 20px;
    }

    .sidebar-dialog {
      width: min(360px, calc(100vw - 32px));
    }
  }
</style>
