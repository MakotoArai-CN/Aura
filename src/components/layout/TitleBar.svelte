<script lang="ts">
  import { getTauriRuntimeDiagnostics, windowMinimize, windowMaximize, windowClose } from "../../lib/tauri";
  import { playerState } from "../../lib/stores/player";
  import { runOnActionKey } from "../../lib/keyboard";
  import { fly } from "svelte/transition";
  import { quintOut } from "svelte/easing";

  let {
    canGoBack = false,
    onBack,
    navigate,
    currentView,
  }: {
    canGoBack?: boolean;
    onBack?: () => void;
    navigate: (v: unknown) => void;
    currentView: { type: string };
  } = $props();

  let searchQuery = $state("");
  let searchFocused = $state(false);

  function doSearch() {
    if (searchQuery.trim()) navigate({ type: "search", query: searchQuery });
  }

  function runWindowCommand(name: string, command: () => Promise<unknown>) {
    command().catch((error) => {
      console.error(`[TitleBar] ${name} failed`, {
        error,
        tauri: getTauriRuntimeDiagnostics(),
      });
    });
  }

  function toggleSettings() {
    if (currentView.type === "settings") {
      if (canGoBack) onBack?.();
      else navigate({ type: "discover", source: "netease" });
      return;
    }
    navigate({ type: "settings" });
  }
</script>

<!-- Navigation bar — position absolute, backdrop blur, matches original -->
<div class="navigation" data-tauri-drag-region>
  <!-- Back/Forward -->
  <div class="backfront">
    {#if canGoBack}
      <span
        class="icon back-icon"
        onclick={onBack}
        role="button" tabindex="0" onkeydown={(e) => runOnActionKey(e, () => onBack?.())}
        in:fly={{ x: -12, duration: 260, easing: quintOut }}
      >
        <svg width="18" height="18" viewBox="0 0 24 24"><path d="M19 12H5M5 12l7-7M5 12l7 7"/></svg>
      </span>
    {/if}
  </div>

  <!-- Search box -->
  <div class="search" class:focused={searchFocused} role="none">
    <svg width="16" height="16" viewBox="0 0 24 24" style="opacity:0.5;flex-shrink:0">
      <circle cx="11" cy="11" r="8"/><path d="m21 21-4.35-4.35"/>
    </svg>
    <input
      type="text"
      class="search-input"
      placeholder="搜索"
      bind:value={searchQuery}
      onfocus={() => searchFocused = true}
      onblur={() => searchFocused = false}
      onkeydown={(e) => e.key === "Enter" && doSearch()}
    />
  </div>

  <!-- Right: settings + window controls -->
  <div class="right-area" style="-webkit-app-region:no-drag">
    <span class="icon settings is-setting" onclick={toggleSettings}
      class:active-icon={currentView.type === "settings"}
      role="button" tabindex="0" onkeydown={(e) => runOnActionKey(e, toggleSettings)}>
      <svg width="22" height="22" viewBox="0 0 24 24">
        <circle cx="12" cy="12" r="3"/>
        <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/>
      </svg>
    </span>

    <div class="window-control">
      <button type="button" class="wc-btn" onclick={() => runWindowCommand("minimize", windowMinimize)} aria-label="最小化">
        <svg width="18" height="18" viewBox="0 0 24 24">
          <line x1="5" y1="12" x2="19" y2="12"/>
        </svg>
      </button>
      <button type="button" class="wc-btn" onclick={() => runWindowCommand("maximize", windowMaximize)} aria-label="最大化">
        <svg width="18" height="18" viewBox="0 0 24 24">
          <rect x="3" y="3" width="18" height="18" rx="2"/>
        </svg>
      </button>
      <button type="button" class="wc-btn close-btn" onclick={() => runWindowCommand("close", windowClose)} aria-label="关闭">
        <svg width="18" height="18" viewBox="0 0 24 24">
          <path d="M18 6L6 18M6 6l12 12"/>
        </svg>
      </button>
    </div>
  </div>
</div>

<style>
  .navigation {
    user-select: none;
    height: 64px;
    flex: 0 0 64px;
    display: flex;
    align-items: center;
    -webkit-app-region: drag;
    position: absolute;
    top: 0; right: 0; left: 0;
    z-index: 100;
    justify-content: space-between;
    -webkit-backdrop-filter: var(--nav-backdrop-filter, none);
    backdrop-filter: var(--nav-backdrop-filter, none);
    background-color: var(--nav-background-color);
    border-bottom: 0;
    transition: background 0.2s;
  }

  .backfront {
    flex: 0 0 45px;
    line-height: 46px;
    vertical-align: middle;
    padding: 0 13px;
    flex: 1;
    display: flex;
    align-items: center;
  }

  .icon {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 8px;
    border-radius: 25%;
    transition: background 0.2s, opacity 0.2s, color 0.2s;
    color: var(--text-default-color);
    opacity: 0.6;
    -webkit-app-region: no-drag;
    cursor: pointer;
  }

  .icon:hover {
    opacity: 1;
    background-color: var(--songlist-hover-background-color);
  }
  .icon:active { opacity: 0.8; }

  .icon.back-icon:hover svg { transform: translateX(-2px); }
  .icon.back-icon svg { transition: transform var(--dur-fast) var(--ease-spring); }

  .icon.active-icon {
    opacity: 1;
    background-color: var(--theme-color-hover);
    color: var(--theme-color);
  }

  .search {
    display: flex;
    width: 200px;
    height: 32px;
    background: var(--search-input-background-color);
    border-style: none;
    border-radius: var(--default-border-radius);
    padding-left: 10px;
    margin-right: 20px;
    align-items: center;
    gap: 8px;
    -webkit-app-region: no-drag;
    transition: background 0.2s;
  }

  .search:hover { background: var(--search-input-background-color); }

  .search.focused {
    box-shadow: none;
    background: var(--search-input-background-color);
  }

  .search-input {
    flex: 1;
    background: transparent;
    border: none;
    outline: none;
    font-size: 16px;
    font-weight: 600;
    color: var(--text-default-color);
    min-width: 0;
  }

  .search-input::placeholder { color: var(--link-default-color); }

  .right-area {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: flex-end;
    padding-right: 8px;
    gap: 0;
  }

  .settings.is-setting {
    margin-right: 8px;
  }

  .window-control {
    display: flex;
    align-items: center;
    gap: 2px;
  }

  .window-control .wc-btn {
    all: unset;
    margin-left: 4px;
    padding: 6px;
    box-sizing: content-box;
    border-radius: 6px;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    -webkit-app-region: no-drag;
    transition: background 0.2s;
  }

  .window-control .wc-btn svg {
    stroke: var(--player-icon-color);
    transition: stroke var(--dur-fast) var(--ease-soft);
  }

  .window-control .wc-btn:hover {
    background: var(--songlist-hover-background-color);
  }
  .window-control .wc-btn:hover svg {
    stroke: var(--text-default-color);
  }
  .window-control .wc-btn:active { opacity: 0.8; }
  .window-control .close-btn:hover {
    background: #ff4444;
  }
  .window-control .close-btn:hover svg {
    stroke: #fff;
  }

  @media (max-width: 720px) {
    .navigation {
      height: 58px;
      flex-basis: 58px;
      padding-top: var(--safe-top);
    }

    .backfront {
      flex: 0 0 42px;
      padding: 0 8px;
    }

    .search {
      flex: 1 1 auto;
      width: auto;
      min-width: 0;
      max-width: none;
      height: 34px;
      margin-right: 8px;
      padding-left: 9px;
    }

    .search-input {
      font-size: 15px;
    }

    .right-area {
      flex: 0 0 auto;
      padding-right: 6px;
    }

    .settings.is-setting {
      margin-right: 2px;
    }

    .window-control .wc-btn {
      margin-left: 0;
      padding: 5px;
    }

    .window-control .wc-btn svg {
      width: 16px;
      height: 16px;
    }
  }

  @media (max-width: 440px) {
    .search {
      max-width: calc(100vw - 172px);
    }

    .window-control .wc-btn {
      padding: 4px;
    }
  }
</style>
