<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { MediaService } from "../../lib/providers/index";
  import type { LoginProvider, UserStatus } from "../../lib/providers/types";
  import { closeLoginWindow, openExternalUrl, openLoginWindow, syncLoginCookies } from "../../lib/tauri";
  import { toast } from "../../lib/stores/toast";
  import type { PlaylistInfo } from "../../lib/providers/types";

  let {
    source = "netease",
    navigate,
  }: {
    source?: string;
    navigate: (v: unknown) => void;
  } = $props();

  const providers: LoginProvider[] = MediaService.getLoginProviders();
  const loginCookieUrls = [
    "https://music.163.com",
    "https://interface3.music.163.com",
    "https://y.qq.com",
    "https://u.y.qq.com",
    "https://c.y.qq.com",
    "https://i.y.qq.com",
    "https://qq.com",
    "https://www.qq.com",
    "https://graph.qq.com",
    "https://xui.ptlogin2.qq.com",
    "https://www.kugou.com",
    "https://www.kuwo.cn",
    "https://api.bilibili.com",
    "https://www.bilibili.com",
    "https://passport.bilibili.com",
    "https://account.bilibili.com",
    "https://space.bilibili.com",
    "https://music.migu.cn",
    "https://m.music.migu.cn",
    "https://passport.migu.cn",
    "https://music.taihe.com",
    "https://music.91q.com",
  ];
  let statuses = $state<Record<string, UserStatus>>({});
  let loading = $state<Record<string, boolean>>({});
  let playlistLoading = $state<Record<string, boolean>>({});
  let activeTabs = $state<Record<string, "created" | "favorite">>({});
  let platformPlaylists = $state<Record<string, { created: PlaylistInfo[]; favorite: PlaylistInfo[] }>>({});
  const loginPollers = new Map<string, ReturnType<typeof setInterval>>();
  let loginPolling = $state<Record<string, boolean>>({});

  function withTimeout<T>(promise: Promise<T>, ms: number, label: string): Promise<T> {
    let timer: ReturnType<typeof setTimeout> | null = null;
    const timeout = new Promise<T>((_, reject) => {
      timer = setTimeout(() => reject(new Error(`${label} timed out after ${ms}ms`)), ms);
    });
    return Promise.race([promise, timeout]).finally(() => {
      if (timer) clearTimeout(timer);
    });
  }

  async function readLoginStatus(sourceId: string): Promise<UserStatus> {
    await withTimeout(syncLoginCookies(loginCookieUrls).catch(() => 0), 5000, "sync login cookies").catch(() => 0);
    const status = await withTimeout(MediaService.getUser(sourceId), 9000, `${sourceId} user status`);
    statuses = { ...statuses, [sourceId]: status };
    if (status.data?.is_login && status.data.user_id != null) {
      await loadPlatformPlaylists(sourceId, status.data.user_id);
    }
    return status;
  }

  async function refresh(sourceId: string) {
    loading = { ...loading, [sourceId]: true };
    try {
      await readLoginStatus(sourceId);
    } catch (error) {
      console.error("[ProfileView] refresh failed", sourceId, error);
      statuses = { ...statuses, [sourceId]: { status: "fail", data: { is_login: false, platform: sourceId } } };
    } finally {
      loading = { ...loading, [sourceId]: false };
    }
  }

  async function refreshAll() {
    await Promise.allSettled(providers.map((provider) => refresh(provider.id)));
  }

  function stopLoginPolling(sourceId: string) {
    const poller = loginPollers.get(sourceId);
    if (poller) clearInterval(poller);
    loginPollers.delete(sourceId);
    loginPolling = { ...loginPolling, [sourceId]: false };
  }

  function startLoginPolling(sourceId: string) {
    stopLoginPolling(sourceId);
    loginPolling = { ...loginPolling, [sourceId]: true };
    const deadline = Date.now() + 3 * 60 * 1000;
    let checking = false;

    const check = async () => {
      if (checking) return;
      if (Date.now() > deadline) {
        stopLoginPolling(sourceId);
        return;
      }
      checking = true;
      try {
        const status = await readLoginStatus(sourceId);
        if (status.data?.is_login) {
          stopLoginPolling(sourceId);
          await closeLoginWindow().catch(() => {});
          toast.success("登录成功");
        }
      } catch {
        // 登录过程中平台页经常会短暂拒绝接口，下一轮继续检查。
      } finally {
        checking = false;
      }
    };

    void check();
    loginPollers.set(sourceId, setInterval(check, 2500));
  }

  async function openLogin(sourceId: string) {
    const url = MediaService.getLoginUrl(sourceId);
    if (!url) {
      toast.error("此平台暂不支持登录");
      return;
    }
    try {
      await withTimeout(openLoginWindow(url), 4000, "open login window");
      startLoginPolling(sourceId);
      toast.info("登录窗口已打开，登录完成后会自动同步状态", 5200);
    } catch (error) {
      console.error("[ProfileView] open login window failed", error);
      await openExternalUrl(url);
      toast.info("内置窗口打开失败，已改用浏览器；浏览器登录无法自动同步，请优先使用内置窗口");
    }
  }

  async function openBrowserLogin(sourceId: string) {
    const url = MediaService.getLoginUrl(sourceId);
    if (!url) {
      toast.error("此平台暂不支持登录");
      return;
    }
    await openExternalUrl(url);
    toast.info("已用浏览器打开登录页，登录后回到这里点刷新");
  }

  async function closeLogin() {
    await closeLoginWindow().catch(() => {});
    providers.forEach((provider) => stopLoginPolling(provider.id));
    toast.info("已关闭登录窗口");
  }

  async function logout(sourceId: string) {
    await MediaService.logout(sourceId);
    statuses = { ...statuses, [sourceId]: { status: "fail", data: { is_login: false } } };
    platformPlaylists = { ...platformPlaylists, [sourceId]: { created: [], favorite: [] } };
    toast.success("已退出登录状态");
  }

  async function loadPlatformPlaylists(sourceId: string, userId: string | number) {
    playlistLoading = { ...playlistLoading, [sourceId]: true };
    try {
      const [created, favorite] = await Promise.all([
        withTimeout(MediaService.getUserCreatedPlaylist(sourceId, userId), 12000, `${sourceId} created playlists`).catch(() => []),
        withTimeout(MediaService.getUserFavoritePlaylist(sourceId, userId), 12000, `${sourceId} favorite playlists`).catch(() => []),
      ]);
      platformPlaylists = {
        ...platformPlaylists,
        [sourceId]: { created, favorite },
      };
      activeTabs = { ...activeTabs, [sourceId]: activeTabs[sourceId] ?? "created" };
    } finally {
      playlistLoading = { ...playlistLoading, [sourceId]: false };
    }
  }

  async function importPlatformPlaylist(playlist: PlaylistInfo) {
    try {
      const full = await MediaService.getPlaylist(playlist.id, false);
      if (!full || full.tracks.length === 0) {
        toast.error("歌单为空或读取失败");
        return;
      }
      const id = MediaService.saveExternalPlaylist("my", full);
      navigate({ type: "playlist", id });
      toast.success(`歌单已导入 ${full.tracks.length} 首`);
    } catch (error) {
      console.error("[ProfileView] import platform playlist failed", error);
      toast.error("导入歌单失败");
    }
  }

  function openPlatformPlaylist(playlist: PlaylistInfo) {
    navigate({ type: "playlist", id: playlist.id });
  }

  onMount(() => {
    void refreshAll();
  });

  onDestroy(() => {
    providers.forEach((provider) => stopLoginPolling(provider.id));
  });
</script>

<div class="profile-view">
  <div class="profile-header">
    <div>
      <h1>登录账号</h1>
      <p>登录后可读取平台创建和收藏歌单，并导入到本地歌单。</p>
    </div>
    <button type="button" class="action-btn" onclick={refreshAll}>刷新</button>
  </div>

  <div class="platform-list">
    {#each providers as provider}
      {@const status = statuses[provider.id]}
      {@const user = status?.data}
      {@const isLogin = Boolean(user?.is_login)}
      {@const tab = activeTabs[provider.id] ?? "created"}
      {@const lists = platformPlaylists[provider.id] ?? { created: [], favorite: [] }}
      {@const visiblePlaylists = lists[tab]}
      <div class="platform-card" class:active={source === provider.id}>
        <div class="platform-top">
          <div class="platform-main">
            {#if isLogin && user?.avatar}
              <img src={user.avatar} alt="" />
            {:else}
              <div class="login-icon">
                <svg viewBox="0 0 24 24">
                  <path d="M15 3h4a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2h-4"/>
                  <path d="M10 17l5-5-5-5"/>
                  <path d="M15 12H3"/>
                </svg>
              </div>
            {/if}
            <div>
              <strong>{isLogin ? user?.nickname ?? "已登录" : "未登录"}</strong>
              <span>{provider.name}</span>
            </div>
          </div>

          <div class="platform-actions">
            {#if isLogin}
              <button type="button" class="action-btn" onclick={() => logout(provider.id)}>退出</button>
            {:else}
              <button type="button" class="action-btn primary" onclick={() => openLogin(provider.id)}>
                {loginPolling[provider.id] ? "同步中" : "登录"}
              </button>
              <button type="button" class="action-btn" onclick={() => openBrowserLogin(provider.id)}>浏览器</button>
            {/if}
            <button type="button" class="icon-btn" disabled={loading[provider.id]} onclick={() => refresh(provider.id)} aria-label="刷新">
              <svg viewBox="0 0 24 24"><path d="M21 12a9 9 0 1 1-2.64-6.36"/><path d="M21 3v6h-6"/></svg>
            </button>
          </div>
        </div>

        {#if isLogin}
          <div class="playlist-tabs">
            <button
              type="button"
              class:active={tab === "created"}
              onclick={() => activeTabs = { ...activeTabs, [provider.id]: "created" }}
            >
              创建的歌单
            </button>
            <button
              type="button"
              class:active={tab === "favorite"}
              onclick={() => activeTabs = { ...activeTabs, [provider.id]: "favorite" }}
            >
              收藏的歌单
            </button>
          </div>

          <div class="platform-playlists">
            {#if playlistLoading[provider.id]}
              <div class="playlist-empty">读取中</div>
            {:else if visiblePlaylists.length === 0}
              <div class="playlist-empty">暂无可读取歌单</div>
            {:else}
              {#each visiblePlaylists as playlist}
                <div class="playlist-row">
                  {#if playlist.cover_img_url}
                    <img src={playlist.cover_img_url} alt="" />
                  {:else}
                    <div class="playlist-cover-placeholder"></div>
                  {/if}
                  <button type="button" class="playlist-title" onclick={() => openPlatformPlaylist(playlist)}>
                    {playlist.title || playlist.id}
                  </button>
                  <button type="button" class="action-btn compact" onclick={() => importPlatformPlaylist(playlist)}>导入</button>
                </div>
              {/each}
            {/if}
          </div>
        {/if}
      </div>
    {/each}
  </div>

  <div class="login-window-actions">
    <button type="button" class="action-btn" onclick={closeLogin}>关闭登录窗口</button>
  </div>
</div>

<style>
  .profile-view {
    width: min(760px, calc(100vw - var(--sidebar-width) - 56px));
    margin: 0 auto;
    padding: 10px 28px 140px;
    color: var(--text-default-color);
  }

  .profile-header {
    min-height: 72px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 18px;
    margin-bottom: 18px;
  }

  .profile-header h1 {
    font-size: 30px;
    line-height: 1.2;
    font-weight: 800;
  }

  .profile-header p,
  .platform-card span {
    color: var(--text-subtitle-color);
    font-size: 13px;
  }

  .platform-list {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .login-window-actions {
    position: sticky;
    bottom: 112px;
    z-index: 2;
    display: flex;
    justify-content: flex-end;
    padding-top: 12px;
    pointer-events: none;
  }

  .login-window-actions .action-btn {
    pointer-events: auto;
    background: var(--glass-bg);
    -webkit-backdrop-filter: var(--glass-blur);
    backdrop-filter: var(--glass-blur);
  }

  .platform-card {
    min-height: 78px;
    border-radius: 10px;
    padding: 12px 14px;
    background: var(--button-background-color);
    border: 1px solid var(--line-default-color);
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .platform-card.active {
    border-color: var(--theme-color);
  }

  .platform-main {
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .platform-main img,
  .login-icon {
    width: 48px;
    height: 48px;
    border-radius: 10px;
    object-fit: cover;
    flex: 0 0 48px;
  }

  .login-icon {
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--theme-color-hover);
    color: var(--theme-color);
  }

  .login-icon svg,
  .icon-btn svg {
    width: 22px;
    height: 22px;
  }

  .platform-main strong {
    display: block;
    font-size: 16px;
    font-weight: 800;
  }

  .platform-actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .platform-top {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 14px;
  }

  .playlist-tabs {
    display: flex;
    gap: 8px;
    border-bottom: 1px solid var(--line-default-color);
    padding-bottom: 8px;
  }

  .playlist-tabs button {
    min-height: 32px;
    padding: 0 12px;
    border-radius: 8px;
    color: var(--text-subtitle-color);
    background: transparent;
    font-weight: 800;
  }

  .playlist-tabs button.active {
    background: var(--theme-color-hover);
    color: var(--theme-color);
  }

  .platform-playlists {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .playlist-row {
    min-height: 48px;
    display: flex;
    align-items: center;
    gap: 10px;
    border-radius: 8px;
    padding: 6px;
  }

  .playlist-row:hover {
    background: var(--songlist-hover-background-color);
  }

  .playlist-row img,
  .playlist-cover-placeholder {
    width: 36px;
    height: 36px;
    border-radius: 6px;
    object-fit: cover;
    flex: 0 0 36px;
  }

  .playlist-cover-placeholder {
    background: var(--theme-color-hover);
  }

  .playlist-title {
    all: unset;
    flex: 1;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    cursor: pointer;
    font-weight: 700;
  }

  .playlist-empty {
    min-height: 42px;
    display: flex;
    align-items: center;
    color: var(--text-subtitle-color);
    font-size: 13px;
  }

  .action-btn,
  .icon-btn {
    min-height: 36px;
    border-radius: 8px;
    color: var(--text-default-color);
    background: var(--button-background-color);
    border: 1px solid var(--line-default-color);
    font-weight: 800;
    white-space: nowrap;
  }

  .action-btn {
    padding: 0 16px;
  }

  .action-btn.compact {
    min-height: 30px;
    padding: 0 12px;
    font-size: 12px;
  }

  .icon-btn {
    width: 36px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    color: var(--text-subtitle-color);
  }

  .action-btn:hover,
  .icon-btn:hover {
    background: var(--button-hover-background-color);
  }

  .action-btn.primary {
    background: var(--theme-color);
    border-color: var(--theme-color);
    color: #fff;
  }

  .icon-btn:disabled {
    opacity: 0.45;
    pointer-events: none;
  }

  @media (max-width: 760px) {
    .profile-view {
      width: 100%;
      padding: 8px 16px 140px;
    }

    .profile-header,
    .platform-top {
      align-items: flex-start;
      flex-direction: column;
    }

    .platform-actions {
      width: 100%;
      justify-content: flex-end;
    }
  }
</style>
