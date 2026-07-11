<script lang="ts">
  import { onMount } from "svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import { settings, type AppSettings } from "../../lib/stores/settings";
  import {
    clearAudioCache,
    defaultCacheDir,
    defaultMusicDir,
    getTauriRuntimeDiagnostics,
    getCacheStats,
    getProxyConfig,
    getResourceUsage,
    setAutostart,
    setCacheConfig,
    setProxyConfig,
    type AudioCacheStats,
    type ResourceUsage,
  } from "../../lib/tauri";
  import { enableGlobalShortcuts, disableGlobalShortcuts } from "../../lib/shortcuts";
  import { SEARCHABLE_SOURCES } from "../../lib/providers/index";
  import { localmusic } from "../../lib/providers/localmusic";
  import {
    SHORTCUT_ACTIONS,
    eventToShortcut,
    shortcutHasModifier,
    shortcutLabel,
    validateShortcutMap,
    type ShortcutAction,
  } from "../../lib/shortcutConfig";
  import { toast } from "../../lib/stores/toast";

  const THEMES: Array<{ id: AppSettings["theme"]; name: string; desc: string; color: string }> = [
    { id: "black2", name: "深曜黑", desc: "沉静高对比暗色", color: "#222222" },
    { id: "white2", name: "晨雾白", desc: "柔和清爽亮色", color: "#ffffff" },
    { id: "liquidGlass", name: "流光玻璃", desc: "通透层次材质", color: "#dfe8ee" },
  ];

  const SOURCE_NAMES: Record<string, string> = {
    netease: "网易云", qq: "QQ音乐", kugou: "酷狗",
    kuwo: "酷我", bilibili: "哔哩哔哩", migu: "咪咕", taihe: "千千",
  };

  const PROXY_MODES = [
    { id: "system", name: "跟随系统" },
    { id: "direct", name: "直连" },
    { id: "manual", name: "手动" },
  ] as const;

  const PROXY_PROTOCOLS = ["http", "https", "socks5", "socks4"];
  const APP_VERSION = "1.0.0";
  const GITHUB_REPO = "listen1/listen1_desktop";
  const LYRIC_COLOR_SCHEMES = [
    { id: "classic", name: "经典白", preview: "linear-gradient(135deg, #ffffff, #d7e1ff)" },
    { id: "gold", name: "暖金", preview: "linear-gradient(135deg, #fff1b8, #ffb86b)" },
    { id: "cyan", name: "青蓝", preview: "linear-gradient(135deg, #8be9fd, #5cc8ff)" },
    { id: "rose", name: "玫瑰", preview: "linear-gradient(135deg, #ffc1d6, #ff6aa2)" },
    { id: "aurora", name: "极光渐变", preview: "linear-gradient(135deg, #7cf7c8, #74a7ff 50%, #f0a6ff)" },
    { id: "sunset", name: "落日渐变", preview: "linear-gradient(135deg, #ffd166, #ff6b6b 52%, #845ec2)" },
    { id: "custom-gradient", name: "自定义渐变", preview: "" },
  ];

  let proxyMode = $state($settings.proxy?.mode ?? "system");
  let proxyHost = $state($settings.proxy?.host ?? "127.0.0.1");
  let proxyPort = $state(String($settings.proxy?.port ?? 1080));
  let proxyProtocol = $state($settings.proxy?.protocol ?? "http");
  let proxySaved = $state(false);
  let cacheStats = $state<AudioCacheStats | null>(null);
  let resourceUsage = $state<ResourceUsage | null>(null);
  let scanningMusic = $state(false);
  let shortcutCapture = $state<{ scope: "keyboard" | "global"; action: ShortcutAction } | null>(null);
  let showKeyboardShortcutSettings = $state(false);
  let showGlobalShortcutSettings = $state(false);
  let latestVersion = $state("");
  let themeRipple = $state<{
    x: number;
    y: number;
    color: string;
    surface: string;
    glow: string;
    id: number;
  } | null>(null);
  let rippleId = 0;
  let immersivePlayerAvailable = $derived($settings.theme === "black2");

  onMount(async () => {
    try {
      const cfg = await getProxyConfig();
      if (cfg.mode) {
        proxyMode = cfg.mode;
        proxyHost = cfg.host ?? "127.0.0.1";
        proxyPort = String(cfg.port ?? 1080);
        proxyProtocol = cfg.protocol ?? "http";
      }
    } catch {}

    const [musicDir, cacheDir] = await Promise.all([
      defaultMusicDir().catch(() => ""),
      defaultCacheDir().catch(() => ""),
    ]);

    const patch: Partial<AppSettings> = {};
    if (!$settings.downloadDir && musicDir) patch.downloadDir = musicDir;
    if (!$settings.localMusicScan.directory && musicDir) {
      patch.localMusicScan = { ...$settings.localMusicScan, directory: musicDir };
    }
    if (!$settings.audioCache.directory && cacheDir) {
      patch.audioCache = { ...$settings.audioCache, directory: cacheDir };
    }
    if ($settings.lyricWindow.variantMode !== "off" && $settings.lyricWindow.lines === 2) {
      patch.lyricWindow = { ...$settings.lyricWindow, lines: 1 };
    }
    if (Object.keys(patch).length > 0) settings.patch(patch);

    await Promise.all([refreshCacheStats(), refreshResourceUsage()]);
    await checkLatestVersion();
  });

  function normalizeVersion(value: string) {
    return value.trim().replace(/^[^\d]*/, "").split(/[+-]/)[0];
  }

  function compareVersions(a: string, b: string) {
    const left = normalizeVersion(a).split(".").map((part) => Number.parseInt(part, 10) || 0);
    const right = normalizeVersion(b).split(".").map((part) => Number.parseInt(part, 10) || 0);
    const length = Math.max(left.length, right.length, 3);
    for (let i = 0; i < length; i++) {
      const diff = (left[i] ?? 0) - (right[i] ?? 0);
      if (diff !== 0) return diff;
    }
    return 0;
  }

  async function fetchLatestTag() {
    const releaseResp = await fetch(`https://api.github.com/repos/${GITHUB_REPO}/releases/latest`, {
      headers: { Accept: "application/vnd.github+json" },
    });
    if (releaseResp.ok) {
      const release = await releaseResp.json() as { tag_name?: string; name?: string };
      const version = normalizeVersion(release.tag_name || release.name || "");
      if (version) return version;
    }

    const tagsResp = await fetch(`https://api.github.com/repos/${GITHUB_REPO}/tags?per_page=1`, {
      headers: { Accept: "application/vnd.github+json" },
    });
    if (!tagsResp.ok) return "";
    const tags = await tagsResp.json() as Array<{ name?: string }>;
    return normalizeVersion(tags[0]?.name ?? "");
  }

  async function checkLatestVersion() {
    try {
      const version = await fetchLatestTag();
      latestVersion = version && compareVersions(version, APP_VERSION) > 0 ? version : "";
    } catch (error) {
      console.warn("[Settings] version check failed", error);
      latestVersion = "";
    }
  }

  function formatBytes(bytes: number) {
    if (bytes < 1024) return `${bytes} B`;
    const units = ["KB", "MB", "GB", "TB"];
    let value = bytes / 1024;
    let unit = units[0];
    for (let i = 1; i < units.length && value >= 1024; i++) {
      value /= 1024;
      unit = units[i];
    }
    return `${value.toFixed(value >= 10 ? 1 : 2)} ${unit}`;
  }

  function formatPercent(value: number) {
    return `${value.toFixed(value >= 10 ? 1 : 2)}%`;
  }

  function themeColor(id: AppSettings["theme"]) {
    return THEMES.find((item) => item.id === id)?.color ?? "#222222";
  }

  function themeSurface(id: AppSettings["theme"]) {
    if (id === "white2") {
      return "radial-gradient(circle at 24% 18%, rgba(1,122,254,0.24), transparent 28%), radial-gradient(circle at 76% 76%, rgba(60,76,120,0.18), transparent 34%), linear-gradient(135deg, rgba(255,255,255,0.88) 0%, rgba(235,241,249,0.90) 45%, rgba(214,224,238,0.92) 100%)";
    }
    if (id === "liquidGlass") {
      return "linear-gradient(135deg, rgba(255,255,255,0.88) 0%, rgba(214,231,238,0.82) 46%, rgba(236,244,247,0.9) 100%)";
    }
    return "radial-gradient(circle at 28% 18%, rgba(1,122,254,0.18), transparent 28%), linear-gradient(135deg, #2b2b2d 0%, #222222 44%, #171719 100%)";
  }

  function themeGlow(id: AppSettings["theme"]) {
    if (id === "white2") return "rgba(28,86,148,0.30)";
    if (id === "liquidGlass") return "rgba(90,200,250,0.24)";
    return "rgba(1,122,254,0.28)";
  }

  function switchTheme(event: MouseEvent, id: AppSettings["theme"]) {
    if ($settings.theme === id) return;
    const current = ++rippleId;
    themeRipple = {
      x: event.clientX,
      y: event.clientY,
      color: themeColor(id),
      surface: themeSurface(id),
      glow: themeGlow(id),
      id: current,
    };
    setTimeout(() => {
      if (themeRipple?.id === current) settings.patch({ theme: id });
    }, 1040);
    setTimeout(() => {
      if (themeRipple?.id === current) themeRipple = null;
    }, 1720);
  }

  async function exportBackup() {
    const items: Record<string, unknown> = {};
    for (const key of Object.keys(localStorage)) {
      try { items[key] = JSON.parse(localStorage.getItem(key)!); }
      catch { items[key] = localStorage.getItem(key); }
    }
    const blob = new Blob([JSON.stringify(items, null, 2)], { type: "application/json" });
    const a = document.createElement("a");
    a.href = URL.createObjectURL(blob);
    a.download = "listen1_backup.json";
    a.click();
    URL.revokeObjectURL(a.href);
    toast.success("备份已导出");
  }

  async function importBackup(e: Event) {
    const file = (e.target as HTMLInputElement).files?.[0];
    if (!file) return;
    try {
      const data = JSON.parse(await file.text());
      for (const [k, v] of Object.entries(data)) {
        localStorage.setItem(k, typeof v === "string" ? v : JSON.stringify(v));
      }
      toast.success("导入成功，部分设置重启后生效");
    } catch (error) {
      console.error("[Settings] import failed", error);
      toast.error("导入失败");
    }
  }

  function toggleFloatWindow(enabled: boolean) {
    // 仅切换开关，浮窗显隐由 LyricSync 统一决策（显式开启会立即显示以给出反馈）。
    settings.patch({ enableLyricFloatingWindow: enabled });
  }

  async function toggleGlobalShortcut(enabled: boolean) {
    settings.patch({ enableGlobalShortcut: enabled });
    enabled ? await enableGlobalShortcuts() : await disableGlobalShortcuts();
  }

  function patchShortcut(scope: "keyboard" | "global", action: ShortcutAction, shortcuts: string[]) {
    const clean = shortcuts.map((item) => item.trim()).filter(Boolean);
    if (scope === "global") {
      const withoutModifier = clean.find((item) => !shortcutHasModifier(item));
      if (withoutModifier) {
        toast.warn("全局快捷键需要包含 Ctrl/Alt/Shift/Command 或媒体键");
        return;
      }
    }
    const current = scope === "keyboard" ? $settings.keyboardShortcuts : $settings.globalShortcuts;
    const next = { ...current, [action]: clean };
    const validation = validateShortcutMap(next);
    if (!validation.ok) {
      toast.warn(validation.message);
      return;
    }
    settings.patch(scope === "keyboard" ? { keyboardShortcuts: next } : { globalShortcuts: next });
    if (scope === "global" && $settings.enableGlobalShortcut) {
      void disableGlobalShortcuts().then(() => enableGlobalShortcuts());
    }
  }

  function handleShortcutCapture(e: KeyboardEvent, scope: "keyboard" | "global", action: ShortcutAction) {
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Escape") {
      shortcutCapture = null;
      return;
    }
    const shortcut = eventToShortcut(e, scope === "global");
    if (!shortcut) return;
    const current = scope === "keyboard" ? $settings.keyboardShortcuts : $settings.globalShortcuts;
    const existing = current[action] ?? [];
    patchShortcut(scope, action, [...existing.filter((item) => item !== shortcut), shortcut]);
    shortcutCapture = null;
  }

  function removeShortcut(scope: "keyboard" | "global", action: ShortcutAction, shortcut: string) {
    const current = scope === "keyboard" ? $settings.keyboardShortcuts : $settings.globalShortcuts;
    patchShortcut(scope, action, (current[action] ?? []).filter((item) => item !== shortcut));
  }

  async function toggleAutostart(enabled: boolean) {
    try {
      await setAutostart(enabled);
      settings.patch({ enableAutostart: enabled });
      toast.success(enabled ? "已开启开机自启" : "已关闭开机自启");
    } catch (error) {
      console.error("[Settings] autostart failed", error);
      toast.error("自启动设置失败");
    }
  }

  function toggleSource(src: string) {
    const list = $settings.autoChooseSourceList;
    const updated = list.includes(src) ? list.filter((s) => s !== src) : [...list, src];
    if (updated.length === 0) {
      toast.warn("至少保留一个备用音源");
      return;
    }
    settings.patch({ autoChooseSourceList: updated.filter((s) => SEARCHABLE_SOURCES.includes(s as never)) });
  }

  async function applyProxy() {
    const config = proxyMode === "manual"
      ? { mode: proxyMode as "manual", host: proxyHost, port: Number(proxyPort), protocol: proxyProtocol }
      : { mode: proxyMode as "system" | "direct" };

    settings.patch({ proxy: config });
    await setProxyConfig(config);
    proxySaved = true;
    setTimeout(() => proxySaved = false, 2000);
  }

  async function chooseDownloadDir() {
    const selected = await open({ directory: true, multiple: false });
    if (typeof selected === "string") settings.patch({ downloadDir: selected });
  }

  async function chooseMusicScanDir() {
    const selected = await open({ directory: true, multiple: false });
    if (typeof selected === "string") {
      settings.patch({ localMusicScan: { ...$settings.localMusicScan, directory: selected } });
    }
  }

  async function scanLocalMusicNow() {
    const directory = $settings.localMusicScan.directory || $settings.downloadDir;
    if (!directory || scanningMusic) return;
    scanningMusic = true;
    try {
      const result = await localmusic.scanDirectory(directory);
      toast.success(`扫描完成：新增 ${result.added} 首，更新 ${result.updated} 首`);
    } catch (error) {
      console.error("[Settings] scan local music failed", error);
      toast.error("扫描音乐目录失败");
    } finally {
      scanningMusic = false;
    }
  }

  async function chooseCacheDir() {
    const selected = await open({ directory: true, multiple: false });
    if (typeof selected === "string") {
      await patchCache({ directory: selected });
    }
  }

  async function patchCache(partial: Partial<AppSettings["audioCache"]>) {
    const next = { ...$settings.audioCache, ...partial };
    settings.patch({ audioCache: next });
    await setCacheConfig({
      enabled: next.enabled,
      directory: next.directory || undefined,
      max_bytes: next.maxBytes,
    }).catch((error) => {
      console.error("[Settings] set cache config failed", {
        error,
        tauri: getTauriRuntimeDiagnostics(),
      });
      toast.error("缓存设置保存失败");
    });
    await refreshCacheStats();
  }

  async function refreshCacheStats() {
    cacheStats = await getCacheStats().catch(() => null);
  }

  async function refreshResourceUsage() {
    resourceUsage = await getResourceUsage().catch((error) => {
      console.warn("[Settings] resource usage failed", error);
      return null;
    });
  }

  // 设置页可见时每 5 秒自动刷新资源统计（后端采样本身 sleep ~220ms，5s 间隔足够平滑）。
  $effect(() => {
    const timer = window.setInterval(() => {
      void refreshResourceUsage();
    }, 5000);
    return () => window.clearInterval(timer);
  });

  async function clearCache() {
    await clearAudioCache();
    await refreshCacheStats();
    toast.success("缓存已清空");
  }
</script>

<div class="settings-view">
  <div class="settings-header">
    <div>
      <h1>设置</h1>
      <p>播放器、下载、缓存、歌词和网络行为</p>
    </div>
    <div class="cache-pill">
      <span>缓存</span>
      <strong>{cacheStats ? formatBytes(cacheStats.total_bytes) : "--"}</strong>
    </div>
  </div>

  <section class="settings-section appearance">
    <div class="section-head">
      <h2>外观</h2>
      <span>主题会从点击处扩散切换</span>
    </div>
    <div class="theme-grid">
      {#each THEMES as t}
        <button
          type="button"
          class="theme-card"
          class:active={$settings.theme === t.id}
          onclick={(e) => switchTheme(e as MouseEvent, t.id)}
        >
          <span class="theme-swatch" style:background={t.color}></span>
          <span class="theme-name">{t.name}</span>
          <span class="theme-desc">{t.desc}</span>
        </button>
      {/each}
    </div>
  </section>

  <section class="settings-section">
    <div class="section-head">
      <h2>沉浸式播放器</h2>
      <span>仅替换深曜黑下的底部播放器</span>
    </div>
    <div class="setting-row">
      <div>
        <strong>沉浸式播放栏</strong>
        <span>
          {#if !$settings.enableImmersivePlayer}
            使用标准底部播放器
          {:else if immersivePlayerAvailable}
            深曜黑下启用沉浸式播放栏
          {:else}
            仅深曜黑主题可用，当前使用标准底部播放器
          {/if}
        </span>
      </div>
      <label class="toggle">
        <input type="checkbox" checked={$settings.enableImmersivePlayer}
          onchange={(e) => settings.patch({ enableImmersivePlayer: (e.target as HTMLInputElement).checked })} />
        <span></span>
      </label>
    </div>
    <div class="setting-row">
      <div>
        <strong>封面自适应颜色</strong>
        <span>展开播放器和底部控件会从当前封面提取强调色</span>
      </div>
      <label class="toggle">
        <input type="checkbox" checked={$settings.enableCoverAdaptiveTheme}
          onchange={(e) => settings.patch({ enableCoverAdaptiveTheme: (e.target as HTMLInputElement).checked })} />
        <span></span>
      </label>
    </div>
  </section>

  <section class="settings-section">
    <div class="section-head">
      <h2>启动与播放</h2>
      <span>桌面行为和播放容错</span>
    </div>
    <div class="setting-row">
      <div>
        <strong>开机自启动</strong>
        <span>注册到系统启动项</span>
      </div>
      <label class="toggle">
        <input type="checkbox" checked={$settings.enableAutostart}
          onchange={(e) => toggleAutostart((e.target as HTMLInputElement).checked)} />
        <span></span>
      </label>
    </div>
    <div class="setting-row">
      <div>
        <strong>关闭时停止播放</strong>
        <span>关闭主窗口时暂停音乐</span>
      </div>
      <label class="toggle">
        <input type="checkbox" checked={$settings.enableStopWhenClose}
          onchange={(e) => settings.patch({ enableStopWhenClose: (e.target as HTMLInputElement).checked })} />
        <span></span>
      </label>
    </div>
    <div class="setting-row">
      <div>
        <strong>界面缩放</strong>
        <span>{Math.round($settings.zoomLevel * 100)}%</span>
      </div>
      <div class="zoom-control">
        <input type="range" class="range-input" min="50" max="200" step="10" value={$settings.zoomLevel * 100}
          oninput={(e) => settings.patch({ zoomLevel: Number((e.target as HTMLInputElement).value) / 100 })} />
        <button type="button" class="zoom-reset" onclick={() => settings.patch({ zoomLevel: 1 })}>重置</button>
      </div>
    </div>
    <div class="setting-row">
      <div>
        <strong>全局快捷键</strong>
        <span>媒体键 + Ctrl/Alt 组合键，失败的键位会自动跳过</span>
      </div>
      <label class="toggle">
        <input type="checkbox" checked={$settings.enableGlobalShortcut}
          onchange={(e) => toggleGlobalShortcut((e.target as HTMLInputElement).checked)} />
        <span></span>
      </label>
    </div>
    <div class="setting-row shortcut-row collapsible desktop-shortcut-setting">
      <button
        type="button"
        class="collapse-head"
        aria-expanded={showKeyboardShortcutSettings}
        onclick={() => showKeyboardShortcutSettings = !showKeyboardShortcutSettings}
      >
        <div>
          <strong>键盘控制</strong>
          <span>窗口内快捷键可自定义，输入框内不会触发</span>
        </div>
        <span class="collapse-chevron" class:open={showKeyboardShortcutSettings}>⌄</span>
      </button>
      {#if showKeyboardShortcutSettings}
        <div class="shortcut-list">
          {#each SHORTCUT_ACTIONS.filter((item) => item.scope !== "global") as action}
            <div class="shortcut-editor">
              <span>{action.label}</span>
              <div class="shortcut-chips">
                {#each $settings.keyboardShortcuts[action.id] ?? [] as shortcut}
                  <button type="button" class="key-chip" onclick={() => removeShortcut("keyboard", action.id, shortcut)}>
                    {shortcutLabel(shortcut)}
                  </button>
                {/each}
                <button
                  type="button"
                  class="key-add"
                  onkeydown={(e) => shortcutCapture?.scope === "keyboard" && shortcutCapture.action === action.id
                    ? handleShortcutCapture(e, "keyboard", action.id)
                    : undefined}
                  onclick={() => shortcutCapture = { scope: "keyboard", action: action.id }}
                >
                  {shortcutCapture?.scope === "keyboard" && shortcutCapture.action === action.id ? "按下键位" : "添加"}
                </button>
              </div>
            </div>
          {/each}
        </div>
      {/if}
    </div>
    <div class="setting-row shortcut-row collapsible desktop-shortcut-setting">
      <button
        type="button"
        class="collapse-head"
        aria-expanded={showGlobalShortcutSettings}
        onclick={() => showGlobalShortcutSettings = !showGlobalShortcutSettings}
      >
        <div>
          <strong>全局快捷键设置</strong>
          <span>需要组合键或媒体键；重复键位会被拒绝</span>
        </div>
        <span class="collapse-chevron" class:open={showGlobalShortcutSettings}>⌄</span>
      </button>
      {#if showGlobalShortcutSettings}
        <div class="shortcut-list">
          {#each SHORTCUT_ACTIONS.filter((item) => item.scope !== "window") as action}
            <div class="shortcut-editor">
              <span>{action.label}</span>
              <div class="shortcut-chips">
                {#each $settings.globalShortcuts[action.id] ?? [] as shortcut}
                  <button type="button" class="key-chip" onclick={() => removeShortcut("global", action.id, shortcut)}>
                    {shortcutLabel(shortcut)}
                  </button>
                {/each}
                <button
                  type="button"
                  class="key-add"
                  onkeydown={(e) => shortcutCapture?.scope === "global" && shortcutCapture.action === action.id
                    ? handleShortcutCapture(e, "global", action.id)
                    : undefined}
                  onclick={() => shortcutCapture = { scope: "global", action: action.id }}
                >
                  {shortcutCapture?.scope === "global" && shortcutCapture.action === action.id ? "按下组合键" : "添加"}
                </button>
              </div>
            </div>
          {/each}
        </div>
      {/if}
    </div>
    <div class="setting-row wrap">
      <div>
        <strong>自动切换音源</strong>
        <span>当前平台不可播时搜索备用平台</span>
      </div>
      <label class="toggle">
        <input type="checkbox" checked={$settings.enableAutoChooseSource}
          onchange={(e) => settings.patch({ enableAutoChooseSource: (e.target as HTMLInputElement).checked })} />
        <span></span>
      </label>
      {#if $settings.enableAutoChooseSource}
        <div class="chip-row full">
          {#each SEARCHABLE_SOURCES as src}
            <button
              type="button"
              class="chip"
              class:active={$settings.autoChooseSourceList.includes(src)}
              onclick={() => toggleSource(src)}
            >{SOURCE_NAMES[src] ?? src}</button>
          {/each}
        </div>
      {/if}
    </div>
  </section>

  <section class="settings-section">
    <div class="section-head">
      <h2>下载与缓存</h2>
      <span>下载到音乐目录，播放缓存默认上限 2GB</span>
    </div>
    <div class="setting-row path-row">
      <div>
        <strong>下载目录</strong>
        <span>{$settings.downloadDir || "使用系统音乐目录"}</span>
      </div>
      <button type="button" class="action-btn" onclick={chooseDownloadDir}>更改</button>
    </div>
    <div class="setting-row path-row">
      <div>
        <strong>本地音乐扫描目录</strong>
        <span>{$settings.localMusicScan.directory || "使用系统音乐目录"}</span>
      </div>
      <div class="button-group">
        <button type="button" class="action-btn" onclick={chooseMusicScanDir}>更改</button>
        <button type="button" class="action-btn primary" onclick={scanLocalMusicNow} disabled={scanningMusic}>
          {scanningMusic ? "扫描中" : "扫描"}
        </button>
      </div>
    </div>
    <div class="setting-row">
      <div>
        <strong>启动时自动扫描</strong>
        <span>增量更新本地音乐，不会清空已有列表；macOS 需要用户授权目录访问</span>
      </div>
      <label class="toggle">
        <input type="checkbox" checked={$settings.localMusicScan.autoScan}
          onchange={(e) => settings.patch({ localMusicScan: { ...$settings.localMusicScan, autoScan: (e.target as HTMLInputElement).checked } })} />
        <span></span>
      </label>
    </div>
    <div class="setting-row">
      <div>
        <strong>播放缓存</strong>
        <span>缓存到用户目录的 Listen1 文件夹，低评分缓存会自动清理</span>
      </div>
      <label class="toggle">
        <input type="checkbox" checked={$settings.audioCache.enabled}
          onchange={(e) => patchCache({ enabled: (e.target as HTMLInputElement).checked })} />
        <span></span>
      </label>
    </div>
    <div class="setting-row">
      <div>
        <strong>本地音乐优先省缓存</strong>
        <span>本地同歌质量不低于网络音源时，不再额外写入网络缓存</span>
      </div>
      <label class="toggle">
        <input type="checkbox" checked={$settings.audioCache.skipWhenLocalQualitySufficient}
          onchange={(e) => patchCache({ skipWhenLocalQualitySufficient: (e.target as HTMLInputElement).checked })} />
        <span></span>
      </label>
    </div>
    <div class="setting-row path-row">
      <div>
        <strong>缓存目录</strong>
        <span>{$settings.audioCache.directory || cacheStats?.directory || "用户目录/Listen1/Cache"}</span>
      </div>
      <button type="button" class="action-btn" onclick={chooseCacheDir}>更改</button>
    </div>
    <div class="setting-row">
      <div>
        <strong>缓存上限</strong>
        <span>{formatBytes($settings.audioCache.maxBytes)}</span>
      </div>
      <input
        class="range-input"
        type="range"
        min="0.5"
        max="10"
        step="0.5"
        value={$settings.audioCache.maxBytes / 1024 / 1024 / 1024}
        oninput={(e) => patchCache({ maxBytes: Number((e.target as HTMLInputElement).value) * 1024 * 1024 * 1024 })}
      />
    </div>
    <div class="status-grid">
      <div class="status-card">
        <div class="status-head">
          <div>
            <strong>资源开销</strong>
            <span>{resourceUsage ? `${resourceUsage.process_count} 进程 / WebView ${resourceUsage.webview_count}` : "未读取"}</span>
          </div>
          <button type="button" class="mini-action" onclick={refreshResourceUsage}>刷新</button>
        </div>
        <div class="metric-row">
          <span>内存</span>
          <strong>{resourceUsage ? formatBytes(resourceUsage.memory_bytes) : "--"}</strong>
        </div>
        <div class="metric-row">
          <span>CPU</span>
          <strong>{resourceUsage ? formatPercent(resourceUsage.cpu_percent) : "--"}</strong>
        </div>
      </div>

      <div class="status-card">
        <div class="status-head">
          <div>
            <strong>缓存状态</strong>
            <span>{cacheStats ? `${cacheStats.entry_count} 首，热 ${cacheStats.hot_count} / 常规 ${cacheStats.warm_count} / 冷 ${cacheStats.cold_count}` : "未读取"}</span>
          </div>
          <div class="button-group compact">
            <button type="button" class="mini-action" onclick={refreshCacheStats}>刷新</button>
            <button type="button" class="mini-action danger" onclick={clearCache}>清空</button>
          </div>
        </div>
        <div class="metric-row">
          <span>已用</span>
          <strong>{cacheStats ? formatBytes(cacheStats.total_bytes) : "--"}</strong>
        </div>
        <div class="metric-row">
          <span>上限</span>
          <strong>{cacheStats ? formatBytes(cacheStats.max_bytes) : "--"}</strong>
        </div>
      </div>
    </div>
  </section>

  <section class="settings-section">
    <div class="section-head">
      <h2>歌词</h2>
      <span>播放页和桌面浮动歌词</span>
    </div>
    <div class="setting-row">
      <div><strong>桌面浮动歌词</strong><span>独立置顶歌词窗口</span></div>
      <label class="toggle">
        <input type="checkbox" checked={$settings.enableLyricFloatingWindow}
          onchange={(e) => toggleFloatWindow((e.target as HTMLInputElement).checked)} />
        <span></span>
      </label>
    </div>
    <div class="setting-row">
      <div><strong>主界面打开时隐藏桌面歌词</strong><span>主窗口显示时临时隐藏，隐藏后自动恢复</span></div>
      <label class="toggle">
        <input type="checkbox" checked={$settings.hideLyricFloatingWindowWhenMainVisible}
          onchange={(e) => settings.patch({ hideLyricFloatingWindowWhenMainVisible: (e.target as HTMLInputElement).checked })} />
        <span></span>
      </label>
    </div>
    <div class="setting-row">
      <div><strong>添加按钮行为</strong><span>底部播放器加号的默认操作</span></div>
      <div class="chip-row">
        <button type="button" class="chip" class:active={$settings.bottomPlayerAddAction === "queue"}
          onclick={() => settings.patch({ bottomPlayerAddAction: "queue" })}>播放队列</button>
        <button type="button" class="chip" class:active={$settings.bottomPlayerAddAction === "playlist"}
          onclick={() => settings.patch({ bottomPlayerAddAction: "playlist" })}>我的歌单</button>
      </div>
    </div>
    {#if $settings.enableLyricFloatingWindow}
      <div class="setting-row">
        <div><strong>字体大小</strong><span>{$settings.lyricWindow.fontSize}px</span></div>
        <input type="range" class="range-input" min="12" max="60" value={$settings.lyricWindow.fontSize}
          oninput={(e) => settings.patch({ lyricWindow: { ...$settings.lyricWindow, fontSize: Number((e.target as HTMLInputElement).value) } })} />
      </div>
      <div class="setting-row">
        <div><strong>桌面歌词行数</strong><span>{$settings.lyricWindow.lines === 2 ? "当前行 + 下一行（显示译文/音标时自动回退为单行）" : "仅当前行"}</span></div>
        <div class="chip-row">
          <button type="button" class="chip" class:active={$settings.lyricWindow.lines === 1}
            onclick={() => settings.patch({ lyricWindow: { ...$settings.lyricWindow, lines: 1 } })}>单行</button>
          <button type="button" class="chip" class:active={$settings.lyricWindow.lines === 2}
            onclick={() => settings.patch({ lyricWindow: { ...$settings.lyricWindow, lines: 2 } })}>两行</button>
        </div>
      </div>
      <div class="setting-row">
        <div><strong>错位歌词布局</strong><span>两行歌词时当前句偏左、下一句偏右，轮换时向左上移动</span></div>
        <label class="toggle">
          <input type="checkbox" checked={$settings.lyricWindow.staggeredLayout}
            onchange={(e) => settings.patch({ lyricWindow: { ...$settings.lyricWindow, staggeredLayout: (e.target as HTMLInputElement).checked } })} />
          <span></span>
        </label>
      </div>
      <div class="setting-row">
        <div><strong>记住锁定状态</strong><span>再次打开桌面歌词时恢复上次锁定状态</span></div>
        <label class="toggle">
          <input type="checkbox" checked={$settings.lyricWindow.rememberLockState}
            onchange={(e) => settings.patch({ lyricWindow: { ...$settings.lyricWindow, rememberLockState: (e.target as HTMLInputElement).checked } })} />
          <span></span>
        </label>
      </div>
      <div class="setting-row">
        <div><strong>锁定桌面歌词</strong><span>锁定后禁止拖拽并允许鼠标穿透</span></div>
        <label class="toggle">
          <input type="checkbox" checked={$settings.lyricWindow.locked}
            onchange={(e) => settings.patch({ lyricWindow: { ...$settings.lyricWindow, locked: (e.target as HTMLInputElement).checked } })} />
          <span></span>
        </label>
      </div>
      <div class="setting-row">
        <div><strong>歌词颜色</strong><span>{$settings.lyricWindow.color}</span></div>
        <input type="color" class="color-input" value={$settings.lyricWindow.color}
          oninput={(e) => settings.patch({ lyricWindow: { ...$settings.lyricWindow, colorScheme: "custom", color: (e.target as HTMLInputElement).value } })} />
      </div>
      <div class="setting-row wrap">
        <div><strong>歌词配色</strong><span>桌面歌词支持纯色和渐变色</span></div>
        <div class="color-scheme-grid full">
          {#each LYRIC_COLOR_SCHEMES as scheme}
            <button
              type="button"
              class="color-scheme"
              class:active={$settings.lyricWindow.colorScheme === scheme.id}
              onclick={() => settings.patch({ lyricWindow: { ...$settings.lyricWindow, colorScheme: scheme.id, color: scheme.id === "classic" ? "#ffffff" : $settings.lyricWindow.color } })}
            >
              <span style:background={scheme.id === "custom-gradient" ? `linear-gradient(135deg, ${$settings.lyricWindow.gradientFrom}, ${$settings.lyricWindow.gradientTo})` : scheme.preview}></span>
              {scheme.name}
            </button>
          {/each}
        </div>
      </div>
      {#if $settings.lyricWindow.colorScheme === "custom-gradient"}
        <div class="setting-row">
          <div><strong>渐变颜色</strong><span>起始色 → 结束色</span></div>
          <div class="gradient-pickers">
            <input type="color" class="color-input" aria-label="渐变起始色" value={$settings.lyricWindow.gradientFrom}
              oninput={(e) => settings.patch({ lyricWindow: { ...$settings.lyricWindow, gradientFrom: (e.target as HTMLInputElement).value } })} />
            <span class="gradient-arrow" aria-hidden="true">→</span>
            <input type="color" class="color-input" aria-label="渐变结束色" value={$settings.lyricWindow.gradientTo}
              oninput={(e) => settings.patch({ lyricWindow: { ...$settings.lyricWindow, gradientTo: (e.target as HTMLInputElement).value } })} />
          </div>
        </div>
      {/if}
      <div class="setting-row">
        <div><strong>背景透明度</strong><span>{Math.round($settings.lyricWindow.backgroundAlpha * 100)}%</span></div>
        <input type="range" class="range-input" min="0" max="100" value={$settings.lyricWindow.backgroundAlpha * 100}
          oninput={(e) => settings.patch({ lyricWindow: { ...$settings.lyricWindow, backgroundAlpha: Number((e.target as HTMLInputElement).value) / 100 } })} />
      </div>
    {/if}
  </section>

  <section class="settings-section">
    <div class="section-head">
      <h2>代理</h2>
      <span>影响平台请求和下载</span>
    </div>
    <div class="setting-row wrap">
      <div><strong>代理模式</strong><span>{proxyMode === "manual" ? `${proxyProtocol}://${proxyHost}:${proxyPort}` : "当前模式会立即应用到后端请求"}</span></div>
      <div class="chip-row">
        {#each PROXY_MODES as m}
          <button
            type="button"
            class="chip"
            class:active={proxyMode === m.id}
            onclick={() => proxyMode = m.id}
          >{m.name}</button>
        {/each}
      </div>
      {#if proxyMode === "manual"}
        <div class="proxy-manual full">
          <select bind:value={proxyProtocol}>
            {#each PROXY_PROTOCOLS as p}
              <option value={p}>{p.toUpperCase()}</option>
            {/each}
          </select>
          <input type="text" bind:value={proxyHost} placeholder="127.0.0.1" />
          <input type="number" bind:value={proxyPort} placeholder="1080" min="1" max="65535" />
        </div>
      {/if}
    </div>
    <div class="setting-row">
      <div><strong>应用代理</strong><span>{proxySaved ? "已保存到本地设置" : "点击后立即更新请求代理"}</span></div>
      <button type="button" class="action-btn primary" class:saved={proxySaved} onclick={applyProxy}>{proxySaved ? "已应用" : "应用"}</button>
    </div>
  </section>

  <section class="settings-section">
    <div class="section-head">
      <h2>数据</h2>
      <span>本地歌单和设置备份</span>
    </div>
    <div class="setting-row">
      <div><strong>导出备份</strong><span>保存 localStorage 数据为 JSON</span></div>
      <button type="button" class="action-btn" onclick={exportBackup}>导出</button>
    </div>
    <div class="setting-row">
      <div><strong>导入备份</strong><span>从 JSON 文件恢复数据</span></div>
      <label class="action-btn file-label">
        导入
        <input type="file" accept=".json" onchange={importBackup} />
      </label>
    </div>
  </section>

  <section class="settings-section about">
    <div class="section-head">
      <h2>关于</h2>
      <span>Aura</span>
    </div>
    <div class="about-grid">
      <div>
        <span>版本</span>
        <strong class="version-line">
          {APP_VERSION}
          {#if latestVersion}
            <em>new</em>
            <b>{latestVersion}</b>
          {/if}
        </strong>
      </div>
      <div><span>技术栈</span><strong>Svelte 5 / Vite / Tauri 2</strong></div>
      <a href="https://github.com/listen1/listen1_desktop" target="_blank">GitHub</a>
    </div>
  </section>

  {#if themeRipple}
    <div
      class="theme-ripple"
      style="--ripple-x:{themeRipple.x}px;--ripple-y:{themeRipple.y}px;--ripple-color:{themeRipple.color};--ripple-surface:{themeRipple.surface};--ripple-glow:{themeRipple.glow}"
    ></div>
  {/if}
</div>

<style>
  .settings-view {
    width: min(980px, calc(100vw - var(--sidebar-width) - 56px));
    margin: 0 auto;
    padding: 10px 28px 140px;
    color: var(--text-default-color);
  }

  .settings-header {
    min-height: 72px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 18px;
    margin-bottom: 18px;
  }

  .settings-header h1 {
    font-size: 30px;
    line-height: 1.2;
    font-weight: 800;
  }

  .settings-header p,
  .section-head span,
  .setting-row span {
    color: var(--text-subtitle-color);
  }

  .cache-pill {
    min-width: 132px;
    height: 48px;
    border-radius: 10px;
    padding: 6px 14px;
    background: var(--button-background-color);
    border: 1px solid var(--line-default-color);
    display: flex;
    flex-direction: column;
    justify-content: center;
  }

  .cache-pill span { font-size: 12px; color: var(--text-subtitle-color); }
  .cache-pill strong { font-size: 16px; }

  .settings-section {
    border-top: 1px solid var(--line-default-color);
    padding: 20px 0 10px;
  }

  .section-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 18px;
    margin-bottom: 12px;
  }

  .section-head h2 {
    font-size: 17px;
    font-weight: 800;
  }

  .section-head span {
    font-size: 12px;
    text-align: right;
  }

  .theme-grid {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 12px;
  }

  .theme-card {
    min-height: 86px;
    border-radius: 10px;
    padding: 14px;
    background: var(--button-background-color);
    border: 1px solid var(--line-default-color);
    display: grid;
    grid-template-columns: 34px 1fr;
    grid-template-rows: auto auto;
    column-gap: 12px;
    row-gap: 2px;
    text-align: left;
    transition: background 0.18s, border-color 0.18s, transform 0.18s;
  }

  .theme-card:hover {
    transform: translateY(-1px);
    background: var(--button-hover-background-color);
  }

  .theme-card.active {
    border-color: var(--theme-color);
    box-shadow: inset 0 0 0 1px var(--theme-color-ope);
  }

  .theme-swatch {
    grid-row: 1 / span 2;
    width: 34px;
    height: 34px;
    border-radius: 50%;
    border: 1px solid var(--line-default-color);
  }

  .theme-name { font-size: 15px; font-weight: 800; color: var(--text-default-color); }
  .theme-desc { font-size: 12px; color: var(--text-subtitle-color); }

  .setting-row {
    min-height: 58px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    border-radius: 10px;
    padding: 8px 12px;
    transition: background 0.16s;
  }

  .setting-row:hover {
    background: var(--songlist-hover-background-color);
  }

  .setting-row > div:first-child {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .setting-row strong {
    font-size: 14px;
    font-weight: 800;
  }

  .setting-row span {
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .setting-row.wrap {
    flex-wrap: wrap;
  }

  .full {
    flex: 0 0 100%;
    width: 100%;
  }

  .path-row > div:first-child {
    flex: 1;
  }

  .chip-row {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: 8px;
  }

  .chip {
    min-height: 32px;
    padding: 0 12px;
    border-radius: 8px;
    color: var(--text-default-color);
    background: var(--button-background-color);
    border: 1px solid var(--line-default-color);
    font-size: 13px;
    font-weight: 700;
  }

  .chip:hover {
    background: var(--button-hover-background-color);
  }

  .chip.active {
    background: var(--theme-color);
    border-color: var(--theme-color);
    color: #fff;
  }

  .shortcut-row {
    align-items: flex-start;
  }

  .shortcut-row.collapsible {
    flex-direction: column;
    align-items: stretch;
    gap: 10px;
  }

  .collapse-head {
    width: 100%;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    color: var(--text-default-color);
    text-align: left;
  }

  .collapse-head > div {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .collapse-chevron {
    flex: 0 0 auto;
    width: 24px;
    height: 24px;
    border-radius: 8px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    color: var(--text-subtitle-color);
    background: var(--button-background-color);
    transition: transform 0.18s, background 0.18s, color 0.18s;
  }

  .collapse-chevron.open {
    transform: rotate(180deg);
    color: var(--theme-color);
    background: var(--theme-color-hover);
  }

  .shortcut-list {
    display: grid;
    gap: 8px;
    width: min(560px, 54vw);
  }

  .shortcut-editor {
    display: grid;
    grid-template-columns: 96px minmax(0, 1fr);
    align-items: center;
    gap: 10px;
  }

  .shortcut-editor > span {
    font-size: 12px;
    color: var(--text-subtitle-color);
    white-space: nowrap;
  }

  .shortcut-chips {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: 6px;
  }

  .key-chip,
  .key-add {
    min-height: 28px;
    padding: 0 9px;
    border-radius: 7px;
    border: 1px solid var(--line-default-color);
    background: var(--button-background-color);
    color: var(--text-default-color);
    font-size: 12px;
    font-weight: 800;
    white-space: nowrap;
  }

  .key-add {
    color: var(--theme-color);
    border-color: var(--theme-color-ope);
  }

  .key-chip:hover {
    border-color: #ff4444;
    color: #ff4444;
  }

  .toggle {
    position: relative;
    width: 46px;
    height: 26px;
    flex: 0 0 46px;
  }

  .toggle input {
    position: absolute;
    inset: 0;
    opacity: 0;
  }

  .toggle span {
    position: absolute;
    inset: 0;
    border-radius: 999px;
    background: var(--button-background-color);
    border: 1px solid var(--line-default-color);
    transition: background 0.2s;
  }

  .toggle span::after {
    content: "";
    position: absolute;
    top: 3px;
    left: 3px;
    width: 18px;
    height: 18px;
    border-radius: 50%;
    background: #fff;
    box-shadow: 0 1px 5px rgba(0,0,0,0.28);
    transition: transform 0.2s;
  }

  .toggle input:checked + span {
    background: var(--theme-color);
    border-color: var(--theme-color);
  }

  .toggle input:checked + span::after {
    transform: translateX(20px);
  }

  .range-input {
    width: min(240px, 34vw);
    accent-color: var(--theme-color);
  }

  .zoom-control {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .zoom-reset {
    flex: 0 0 auto;
    min-height: 30px;
    padding: 0 12px;
    border-radius: 8px;
    border: 1px solid var(--line-default-color);
    background: var(--button-background-color);
    color: var(--text-default-color);
    font-size: 12px;
    cursor: pointer;
    transition: background 0.2s, color 0.2s;
  }

  .zoom-reset:hover {
    color: var(--theme-color);
    border-color: var(--theme-color);
  }

  .action-btn {
    min-height: 36px;
    padding: 0 16px;
    border-radius: 8px;
    color: var(--text-default-color);
    background: var(--button-background-color);
    border: 1px solid var(--line-default-color);
    font-weight: 800;
    white-space: nowrap;
  }

  .action-btn:hover {
    background: var(--button-hover-background-color);
  }

  .action-btn.primary,
  .action-btn.saved {
    background: var(--theme-color);
    border-color: var(--theme-color);
    color: #fff;
  }

  .button-group {
    display: flex;
    gap: 8px;
  }

  .button-group.compact {
    gap: 6px;
  }

  .status-grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 12px;
    margin: 10px 0 4px;
  }

  .status-card {
    min-height: 126px;
    border-radius: 8px;
    padding: 12px;
    background: var(--button-background-color);
    border: 1px solid var(--line-default-color);
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .status-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 10px;
  }

  .status-head > div:first-child {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .status-head strong {
    font-size: 14px;
    font-weight: 800;
  }

  .status-head span,
  .metric-row span {
    color: var(--text-subtitle-color);
    font-size: 12px;
  }

  .status-head span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .metric-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    min-height: 24px;
  }

  .metric-row strong {
    font-size: 15px;
    font-weight: 900;
  }

  .mini-action {
    min-height: 28px;
    padding: 0 10px;
    border-radius: 7px;
    color: var(--text-default-color);
    background: var(--nav-background-color);
    border: 1px solid var(--line-default-color);
    font-size: 12px;
    font-weight: 800;
    white-space: nowrap;
  }

  .mini-action:hover {
    background: var(--button-hover-background-color);
  }

  .mini-action.danger:hover {
    background: #ff4444;
    border-color: #ff4444;
    color: #fff;
  }

  .color-scheme-grid {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 8px;
  }

  .gradient-pickers {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .gradient-arrow {
    color: var(--text-subtitle-color);
    font-weight: 800;
  }

  .color-scheme {
    min-height: 36px;
    border-radius: 8px;
    border: 1px solid var(--line-default-color);
    background: var(--button-background-color);
    color: var(--text-default-color);
    font-size: 12px;
    font-weight: 800;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 10px;
  }

  .color-scheme span {
    width: 24px;
    height: 16px;
    border-radius: 999px;
    border: 1px solid rgba(255,255,255,0.24);
  }

  .color-scheme.active {
    border-color: var(--theme-color);
    box-shadow: inset 0 0 0 1px var(--theme-color-ope);
  }

  .file-label {
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }

  .file-label input {
    display: none;
  }

  .proxy-manual {
    display: grid;
    grid-template-columns: 128px 1fr 120px;
    gap: 10px;
  }

  .proxy-manual input,
  .proxy-manual select {
    height: 38px;
    border-radius: 8px;
    border: 1px solid var(--line-default-color);
    background: var(--button-background-color);
    color: var(--text-default-color);
    padding: 0 10px;
  }

  .about-grid {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 12px;
  }

  .about-grid > div,
  .about-grid > a {
    min-height: 56px;
    border-radius: 10px;
    padding: 10px 12px;
    background: var(--button-background-color);
    border: 1px solid var(--line-default-color);
    display: flex;
    flex-direction: column;
    justify-content: center;
    color: var(--text-default-color);
  }

  .about-grid span {
    font-size: 12px;
    color: var(--text-subtitle-color);
  }

  .about-grid strong,
  .about-grid a {
    font-size: 14px;
    font-weight: 800;
  }

  .version-line {
    display: flex;
    align-items: center;
    gap: 7px;
    flex-wrap: wrap;
  }

  .version-line em {
    display: inline-flex;
    align-items: center;
    height: 18px;
    padding: 0 7px;
    border-radius: 999px;
    background: rgba(34, 197, 94, 0.16);
    color: #22c55e;
    font-size: 11px;
    font-style: normal;
    font-weight: 900;
    line-height: 1;
  }

  .version-line b {
    color: var(--theme-color);
    font-size: 14px;
  }

  .theme-ripple {
    position: fixed;
    inset: 0;
    z-index: 2000;
    pointer-events: none;
    overflow: hidden;
    background: var(--ripple-surface);
    clip-path: circle(0 at var(--ripple-x) var(--ripple-y));
    opacity: 0;
    will-change: clip-path, opacity, transform;
    animation: themeSpread 1700ms cubic-bezier(0.22, 0.61, 0.36, 1) forwards;
  }

  .theme-ripple::before,
  .theme-ripple::after {
    content: "";
    position: absolute;
    inset: -16%;
    pointer-events: none;
  }

  .theme-ripple::before {
    background:
      radial-gradient(circle at var(--ripple-x) var(--ripple-y),
        color-mix(in srgb, var(--ripple-color) 52%, #89a8cf 48%) 0 5%,
        var(--ripple-glow) 18%,
        transparent 44%),
      linear-gradient(115deg, transparent 0 35%, rgba(97,130,180,0.20) 48%, transparent 61%),
      repeating-linear-gradient(90deg, rgba(64,94,128,0.025) 0 1px, transparent 1px 5px);
    mix-blend-mode: screen;
    opacity: 0.72;
    transform: translate3d(-2%, 0, 0);
    animation: themeSheen 1700ms cubic-bezier(0.22, 0.61, 0.36, 1) forwards;
  }

  .theme-ripple::after {
    background:
      radial-gradient(circle at var(--ripple-x) var(--ripple-y), transparent 0 42%, rgba(48,82,128,0.24) 43%, transparent 46%),
      radial-gradient(circle at 74% 18%, rgba(1,122,254,0.12), transparent 32%);
    opacity: 0.42;
    filter: blur(10px);
    animation: themeEdge 1700ms cubic-bezier(0.22, 0.61, 0.36, 1) forwards;
  }

  @keyframes themeSpread {
    0% { clip-path: circle(0 at var(--ripple-x) var(--ripple-y)); opacity: 0.98; }
    64% { clip-path: circle(150vmax at var(--ripple-x) var(--ripple-y)); opacity: 0.98; }
    80% { clip-path: circle(150vmax at var(--ripple-x) var(--ripple-y)); opacity: 0.92; }
    100% { clip-path: circle(150vmax at var(--ripple-x) var(--ripple-y)); opacity: 0; }
  }

  @keyframes themeSheen {
    0% { opacity: 0; transform: translate3d(-8%, 0, 0) scale(1); }
    18% { opacity: 0.74; }
    68% { opacity: 0.52; transform: translate3d(8%, 0, 0) scale(1.03); }
    100% { opacity: 0; transform: translate3d(12%, 0, 0) scale(1.04); }
  }

  @keyframes themeEdge {
    0% { opacity: 0.5; transform: scale(0.98); }
    62% { opacity: 0.24; transform: scale(1.02); }
    100% { opacity: 0; transform: scale(1.04); }
  }

  @media (max-width: 760px) {
    .desktop-shortcut-setting {
      display: none;
    }

    .settings-view {
      width: 100%;
      padding: 8px 16px 140px;
    }

    .settings-header,
    .section-head {
      align-items: flex-start;
      flex-direction: column;
    }

    .theme-grid,
    .about-grid,
    .proxy-manual,
    .status-grid,
    .color-scheme-grid {
      grid-template-columns: 1fr;
    }

    .setting-row {
      align-items: flex-start;
      flex-direction: column;
    }

    .range-input {
      width: 100%;
    }

    .shortcut-list {
      width: 100%;
    }

    .shortcut-editor {
      grid-template-columns: 1fr;
    }

    .shortcut-chips {
      justify-content: flex-start;
    }
  }
</style>
