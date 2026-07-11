<script lang="ts">
  import { onMount } from "svelte";
  import { fade } from "svelte/transition";
  import { quintOut } from "svelte/easing";
  import TitleBar from "./components/layout/TitleBar.svelte";
  import Sidebar from "./components/layout/Sidebar.svelte";
  import BottomPlayer from "./components/layout/BottomPlayer.svelte";
  import ImmersivePlayer from "./components/layout/ImmersivePlayer.svelte";
  import SearchView from "./components/views/SearchView.svelte";
  import DiscoverView from "./components/views/DiscoverView.svelte";
  import PlaylistView from "./components/views/PlaylistView.svelte";
  import SettingsView from "./components/views/SettingsView.svelte";
  import ProfileView from "./components/views/ProfileView.svelte";
  import LyricSync from "./components/LyricSync.svelte";
  import Toast from "./components/ui/Toast.svelte";
  import { player } from "./lib/player";
  import {
    defaultCacheDir,
    defaultMusicDir,
    getTauriRuntimeDiagnostics,
    hideFloatWindow,
    isAutostartEnabled,
    listen,
    setCacheConfig,
    setProxyConfig,
    showFloatWindow,
  } from "./lib/tauri";
  import { settings, type AppSettings } from "./lib/stores/settings";
  import { enableGlobalShortcuts, disableGlobalShortcuts } from "./lib/shortcuts";
  import { bindPlayerKeyboard } from "./lib/keyboard";
  import { localmusic } from "./lib/providers/localmusic";

  type View =
    | { type: "discover"; source?: string }
    | { type: "search"; query?: string }
    | { type: "playlist"; id: string }
    | { type: "nowplaying" }
    | { type: "profile"; source?: string }
    | { type: "settings" };

  let currentView: View = $state({ type: "discover", source: "netease" });
  let history: View[] = $state([]);
  let immersivePlayerActive = $derived($settings.enableImmersivePlayer && $settings.theme === "black2");
  let lastTrayActionId = "";
  let contentView = $derived.by(() => {
    if (currentView.type !== "nowplaying") return currentView;
    return history[history.length - 1] ?? ({ type: "discover", source: "netease" } as View);
  });

  // Stable key that only flips when the "logical" view changes,
  // so transitions play on real navigations and not on every keystroke.
  let viewKey = $derived.by(() => {
    const view = contentView as View;
    if (view.type === "discover") return `d:${view.source ?? ""}`;
    if (view.type === "playlist") return `p:${view.id}`;
    if (view.type === "profile") return `profile:${view.source ?? ""}`;
    return view.type;
  });

  function navigate(view: unknown) {
    history = [...history, currentView];
    currentView = view as View;
  }

  function goBack() {
    if (history.length > 0) {
      currentView = history[history.length - 1];
      history = history.slice(0, -1);
    }
  }

  function closeNowPlaying() {
    if (history.length > 0) goBack();
    else currentView = { type: "discover", source: "netease" };
  }

  function handleTrayAction(payload: unknown) {
    const action = typeof payload === "string"
      ? payload
      : payload && typeof payload === "object" && "action" in payload
        ? String((payload as { action: unknown }).action)
        : "";
    const actionId = payload && typeof payload === "object" && "id" in payload
      ? String((payload as { id: unknown }).id)
      : "";

    if (!action) return;
    if (actionId && actionId === lastTrayActionId) return;
    if (actionId) lastTrayActionId = actionId;
    console.info("[App] tray action", action);
    if (action === "play_pause") player.togglePlayPause();
    else if (action === "prev") player.skip("prev");
    else if (action === "next") player.skip("next");
  }

  $effect(() => {
    const cache = $settings.audioCache;
    void setCacheConfig({
      enabled: cache.enabled,
      directory: cache.directory || undefined,
      max_bytes: cache.maxBytes,
    }).catch((error) => console.error("[App] set cache config failed", error));
  });

  $effect(() => {
    const zoom = Math.max(0.5, Math.min(2, $settings.zoomLevel || 1));
    document.documentElement.style.setProperty("zoom", String(zoom));
  });

  onMount(() => {
    let unlisten: (() => void) | null = null;
    let disposed = false;

    void listen<unknown>("tray-action", (e) => handleTrayAction(e.payload))
      .then((cleanup) => {
        unlisten = cleanup;
        if (disposed) cleanup();
      })
      .catch((error) => console.error("[App] tray listener failed", {
        error,
        tauri: getTauriRuntimeDiagnostics(),
      }));

    function handleTrayDomEvent(event: Event) {
      handleTrayAction((event as CustomEvent<unknown>).detail);
    }
    window.addEventListener("listen1-tray-action", handleTrayDomEvent);

    function handleMainVisibilityChange() {
      if (document.visibilityState === "hidden" && $settings.enableStopWhenClose) {
        player.pause();
      }
    }
    document.addEventListener("visibilitychange", handleMainVisibilityChange);

    void (async () => {
      await setProxyConfig($settings.proxy).catch((error) => {
        console.error("[App] set proxy config failed", {
          error,
          tauri: getTauriRuntimeDiagnostics(),
        });
      });
      const [cacheDir, musicDir] = await Promise.all([
        defaultCacheDir().catch(() => ""),
        defaultMusicDir().catch(() => ""),
      ]);
      const patch: Partial<AppSettings> = {};
      if (!$settings.audioCache.directory && cacheDir) {
        patch.audioCache = { ...$settings.audioCache, directory: cacheDir };
      }
      if (!$settings.downloadDir && musicDir) patch.downloadDir = musicDir;
      if (!$settings.localMusicScan.directory && musicDir) {
        patch.localMusicScan = { ...$settings.localMusicScan, directory: musicDir };
      }
      if (Object.keys(patch).length > 0) settings.patch(patch);

      await setCacheConfig({
        enabled: $settings.audioCache.enabled,
        directory: ($settings.audioCache.directory || cacheDir) || undefined,
        max_bytes: $settings.audioCache.maxBytes,
      }).catch((error) => {
        console.error("[App] set initial cache config failed", {
          error,
          tauri: getTauriRuntimeDiagnostics(),
        });
      });

      const enabledAtSystem = await isAutostartEnabled().catch(() => false);
      if (enabledAtSystem !== $settings.enableAutostart) {
        settings.patch({ enableAutostart: enabledAtSystem });
      }

      if ($settings.enableGlobalShortcut) await enableGlobalShortcuts();
      const scanDir = $settings.localMusicScan.directory || musicDir;
      if ($settings.localMusicScan.autoScan && scanDir) {
        await localmusic.scanDirectory(scanDir).catch((error) => {
          console.error("[App] auto scan local music failed", error);
        });
      }
      if ($settings.enableLyricFloatingWindow) {
        if ($settings.hideLyricFloatingWindowWhenMainVisible && document.visibilityState === "visible") {
          hideFloatWindow();
        } else {
          await showFloatWindow().catch((error) => {
            console.error("[App] show float window failed", {
              error,
              tauri: getTauriRuntimeDiagnostics(),
            });
          });
        }
      }
    })();

    const unbindKeyboard = bindPlayerKeyboard({
      navigate,
      isNowPlaying: () => currentView.type === "nowplaying",
      closeNowPlaying,
    });

    return () => {
      disposed = true;
      unlisten?.();
      void disableGlobalShortcuts();
      window.removeEventListener("listen1-tray-action", handleTrayDomEvent);
      document.removeEventListener("visibilitychange", handleMainVisibilityChange);
      unbindKeyboard();
    };
  });
</script>

<div
  class="wrap"
  data-theme={$settings.theme === "white2" ? "light" : $settings.theme === "liquidGlass" ? "liquid-glass" : "dark"}
  data-immersive-player={immersivePlayerActive}
>
  <div class="body-bg">
    <div class="main" id="listen1-glass-scene">
      <div class="sidebar">
        <Sidebar {navigate} activeView={contentView} />
      </div>
      <div class="content">
        <TitleBar
          canGoBack={history.length > 0}
          onBack={goBack}
          {navigate}
          currentView={contentView}
        />
        <div class="browser">
          {#key viewKey}
            <div
              class="view-holder"
              in:fade={{ duration: 260, easing: quintOut }}
            >
              {#if contentView.type === "discover"}
                <DiscoverView {navigate} source={(contentView as { source?: string }).source ?? "netease"} />
              {:else if contentView.type === "search"}
                <SearchView {navigate} query={(contentView as { query?: string }).query} />
              {:else if contentView.type === "playlist"}
                <PlaylistView {navigate} listId={(contentView as { id: string }).id} />
              {:else if contentView.type === "profile"}
                <ProfileView {navigate} source={(contentView as { source?: string }).source ?? "netease"} />
              {:else if contentView.type === "settings"}
                <SettingsView />
              {/if}
            </div>
          {/key}
        </div>
      </div>
    </div>
    {#if immersivePlayerActive}
      <ImmersivePlayer
        {navigate}
        activeView={currentView}
        nowPlayingOpen={currentView.type === "nowplaying"}
        onCloseNowPlaying={closeNowPlaying}
      />
    {:else}
      <BottomPlayer
        {navigate}
        activeView={currentView}
        nowPlayingOpen={currentView.type === "nowplaying"}
        onCloseNowPlaying={closeNowPlaying}
      />
    {/if}
  </div>

  <Toast />
  <LyricSync />
</div>

<style>
  .wrap {
    display: flex;
    height: var(--app-viewport-height);
    width: var(--app-viewport-width);
    flex-direction: column;
    overflow: hidden;
    outline: solid 1px var(--windows-border-color);
    box-sizing: border-box;
    color: var(--text-default-color);
  }

  .wrap[data-immersive-player="true"] {
    --player-height: 0px;
  }

  .body-bg {
    display: flex;
    flex-direction: column;
    height: 100%;
    background-color: var(--color-body-bg);
    border-radius: var(--app-shell-radius);
    overflow: hidden;
    transition: background-color 0.35s ease, background 0.35s ease, color 0.35s ease;
  }

  .wrap[data-theme="liquid-glass"] .body-bg {
    background:
      linear-gradient(135deg, rgba(255,255,255,0.74), rgba(214,231,238,0.68) 48%, rgba(236,244,247,0.78)),
      var(--color-body-bg);
  }

  .wrap[data-theme="liquid-glass"] .body-bg::before {
    content: "";
    position: fixed;
    inset: 0;
    pointer-events: none;
    background-image:
      linear-gradient(rgba(255,255,255,0.28) 1px, transparent 1px),
      linear-gradient(90deg, rgba(255,255,255,0.22) 1px, transparent 1px);
    background-size: 42px 42px;
    mask-image: linear-gradient(to bottom, rgba(0,0,0,0.55), transparent 70%);
  }

  .main {
    flex: 1;
    display: flex;
    overflow: hidden;
    min-height: 0;
  }

  .sidebar {
    flex: 0 0 auto;
    display: flex;
    flex-direction: column;
    padding-left: var(--app-edge-gap);
    padding-top: var(--app-edge-gap);
  }

  .content {
    flex: 1;
    display: flex;
    flex-direction: column;
    position: relative;
    overflow: hidden;
  }

  .browser {
    flex: 1;
    overflow-y: auto;
    overflow-x: hidden;
    margin-top: 64px;
    padding-bottom: calc(var(--player-height) + var(--mobile-nav-height) + var(--safe-bottom) + 24px);
  }

  .view-holder {
    min-height: 100%;
  }

  @media (max-width: 720px) {
    .main {
      display: block;
      position: relative;
    }

    .sidebar {
      position: fixed;
      left: calc(var(--safe-left) + 8px);
      right: calc(var(--safe-right) + 8px);
      bottom: calc(var(--player-height) + var(--player-edge-gap) + var(--safe-bottom) + 8px);
      z-index: 125;
      padding: 0;
      display: block;
      pointer-events: auto;
    }

    .content {
      width: 100%;
      height: 100%;
    }

    .browser {
      margin-top: 58px;
      padding-bottom: calc(var(--player-height) + var(--mobile-nav-height) + var(--player-edge-gap) + var(--safe-bottom) + 22px);
    }
  }
</style>
