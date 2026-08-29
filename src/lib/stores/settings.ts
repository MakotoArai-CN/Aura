import { writable, derived, get } from "svelte/store";

export interface ProxyConfig {
  mode: "system" | "direct" | "manual";
  protocol?: string;
  host?: string;
  port?: number;
}

export type LyricWindowVariantMode = "off" | "translation" | "phonetic";

export interface LyricWindowSettings {
  fontSize: number;
  color: string;
  colorScheme: string;
  gradientFrom: string;
  gradientTo: string;
  backgroundAlpha: number;
  lines: 1 | 2;
  offsetMs: number;
  staggeredLayout: boolean;
  locked: boolean;
  rememberLockState: boolean;
  variantMode: LyricWindowVariantMode;
}

export interface AppSettings {
  theme: "origin" | "origin2" | "black2" | "white2" | "liquidGlass" | "auto";
  enableAutostart: boolean;
  enableGlobalShortcut: boolean;
  enableAutoChooseSource: boolean;
  autoChooseSourceList: string[];
  enableStopWhenClose: boolean;
  enableNowplayingCoverBackground: boolean;
  enableCoverAdaptiveTheme: boolean;
  enableNowplayingBitrate: boolean;
  enableNowplayingPlatform: boolean;
  enableLyricFloatingWindow: boolean;
  hideLyricFloatingWindowWhenMainVisible: boolean;
  proxy: ProxyConfig;
  lyricWindow: LyricWindowSettings;
  bottomPlayerAddAction: "queue" | "playlist";
  keyboardShortcuts: Record<string, string[]>;
  globalShortcuts: Record<string, string[]>;
  localMusicScan: {
    directory: string;
    autoScan: boolean;
  };
  downloadDir: string;
  audioCache: {
    enabled: boolean;
    directory: string;
    maxBytes: number;
    skipWhenLocalQualitySufficient: boolean;
  };
  zoomLevel: number;
  /**
   * 视觉效果档位。"auto" = 交给 Rust 的硬件检测 + 运行时 CPU 看门狗；
   * 其余为手动锁定，检测与看门狗一律不再插手。
   * 注意 "auto" 最高只会自动选到 high；"ultra" 是纯手动选项。
   */
  effectTier: "auto" | "ultra" | "high" | "mid" | "low";
  /**
   * 是否给 WebView 开 GPU 加速。默认关。
   *
   * 和 effectTier 完全独立：档位只管 CSS 效果，这个只管 WebView2 的
   * `--disable-gpu --disable-gpu-compositing` 启动参数。改动要下次启动才生效，
   * 因为那两个参数必须在第一个 WebView 创建之前写进环境变量。
   */
  enableGpuAcceleration: boolean;
}

const defaults: AppSettings = {
  theme: "black2",
  enableAutostart: false,
  enableGlobalShortcut: false,
  enableAutoChooseSource: true,
  autoChooseSourceList: ["netease", "qq", "kugou", "kuwo", "bilibili", "migu", "taihe"],
  enableStopWhenClose: true,
  enableNowplayingCoverBackground: false,
  enableCoverAdaptiveTheme: false,
  enableNowplayingBitrate: false,
  enableNowplayingPlatform: false,
  enableLyricFloatingWindow: false,
  hideLyricFloatingWindowWhenMainVisible: true,
  proxy: { mode: "system" },
  lyricWindow: {
    fontSize: 24,
    color: "#ffffff",
    colorScheme: "classic",
    gradientFrom: "#7cf7c8",
    gradientTo: "#f0a6ff",
    backgroundAlpha: 0.8,
    lines: 1,
    offsetMs: 0,
    staggeredLayout: false,
    locked: false,
    rememberLockState: true,
    variantMode: "translation",
  },
  bottomPlayerAddAction: "queue",
  keyboardShortcuts: {
    togglePlay: ["Space", "K", "P"],
    prevTrack: ["BracketLeft", "Comma"],
    nextTrack: ["BracketRight", "Period"],
    seekBackward: ["ArrowLeft", "J"],
    seekForward: ["ArrowRight", "L"],
    volumeUp: ["ArrowUp"],
    volumeDown: ["ArrowDown"],
    mute: ["M"],
    search: ["F"],
    closeNowPlaying: ["Escape"],
  },
  globalShortcuts: {
    togglePlay: ["CmdOrCtrl+Alt+Space", "MediaPlayPause"],
    prevTrack: ["CmdOrCtrl+Alt+Left", "MediaPreviousTrack"],
    nextTrack: ["CmdOrCtrl+Alt+Right", "MediaNextTrack"],
    volumeUp: ["CmdOrCtrl+Alt+Up"],
    volumeDown: ["CmdOrCtrl+Alt+Down"],
    mute: ["CmdOrCtrl+Alt+M"],
    stop: ["MediaStop"],
  },
  localMusicScan: {
    directory: "",
    autoScan: false,
  },
  downloadDir: "",
  audioCache: {
    enabled: true,
    directory: "",
    maxBytes: 2 * 1024 * 1024 * 1024,
    skipWhenLocalQualitySufficient: true,
  },
  zoomLevel: 1,
  effectTier: "auto",
  enableGpuAcceleration: false,
};

function loadSettings(): AppSettings {
  try {
    const stored = localStorage.getItem("listen1_settings");
    if (stored) {
      const parsed = JSON.parse(stored);
      const next = { ...defaults, ...parsed };
      next.audioCache = { ...defaults.audioCache, ...(parsed.audioCache ?? {}) };
      next.lyricWindow = { ...defaults.lyricWindow, ...(parsed.lyricWindow ?? {}) };
      // 未开启「记住锁定状态」时，每次启动都回到未锁定，避免桌面歌词默认卡在锁定穿透。
      if (!next.lyricWindow.rememberLockState) {
        next.lyricWindow.locked = false;
      }
      // Migrate legacy flags: variantMode is now the single source of truth.
      // If an old profile has no variantMode, derive it from the retired
      // enableLyricFloatingWindowTranslation flag so the user's choice survives.
      const legacyTranslationFlag = (parsed as { enableLyricFloatingWindowTranslation?: unknown }).enableLyricFloatingWindowTranslation;
      if (!["off", "translation", "phonetic"].includes(next.lyricWindow.variantMode)) {
        next.lyricWindow.variantMode = legacyTranslationFlag === false ? "off" : "translation";
      } else if (
        typeof (parsed.lyricWindow as Partial<LyricWindowSettings> | undefined)?.variantMode === "undefined" &&
        legacyTranslationFlag === false
      ) {
        next.lyricWindow.variantMode = "off";
      }
      delete (next as AppSettings & { enableLyricTranslation?: unknown; enableLyricFloatingWindowTranslation?: unknown }).enableLyricTranslation;
      delete (next as AppSettings & { enableLyricTranslation?: unknown; enableLyricFloatingWindowTranslation?: unknown }).enableLyricFloatingWindowTranslation;
      next.keyboardShortcuts = { ...defaults.keyboardShortcuts, ...(parsed.keyboardShortcuts ?? {}) };
      next.globalShortcuts = { ...defaults.globalShortcuts, ...(parsed.globalShortcuts ?? {}) };
      next.localMusicScan = { ...defaults.localMusicScan, ...(parsed.localMusicScan ?? {}) };
      if (next.theme === "origin" || next.theme === "origin2") next.theme = "black2";
      if ((parsed as { theme?: string }).theme === "mineradio") next.theme = "black2";
      delete (next as AppSettings & { playerTheme?: unknown; enableMineradioStage?: unknown; enableImmersivePlayer?: unknown }).playerTheme;
      delete (next as AppSettings & { playerTheme?: unknown; enableMineradioStage?: unknown; enableImmersivePlayer?: unknown }).enableMineradioStage;
      delete (next as AppSettings & { playerTheme?: unknown; enableMineradioStage?: unknown; enableImmersivePlayer?: unknown }).enableImmersivePlayer;
      return next;
    }
  } catch {}
  return { ...defaults };
}

/**
 * 需要去抖落盘的键：只有会被"拖"出来的那几个。
 *
 * range 滑条的 oninput 按像素连发，每次都全量 stringify 整个设置对象太浪费。
 * 除此之外的设置全是点一下就定的开关/单选，必须**同步**落盘——托盘「退出」走的是
 * Rust 侧 `app.exit(0)`，进程说没就没，前端没有 beforeunload 可以补救。GPU 开关就
 * 栽在这上面：拨完 200ms 内退出，localStorage 里根本没写进去，下次启动读回默认值
 * false，再被挂载时的自愈回写把 Rust 那份开关文件也刷成 off，用户看到的就是"我明明
 * 开了 GPU"。
 */
const DEBOUNCED_KEYS = new Set<keyof AppSettings>(["zoomLevel", "lyricWindow"]);

function createSettingsStore() {
  const { subscribe, set, update } = writable<AppSettings>(loadSettings());
  let persistTimer: ReturnType<typeof setTimeout> | null = null;

  function writeNow(next: AppSettings) {
    if (persistTimer) {
      clearTimeout(persistTimer);
      persistTimer = null;
    }
    try {
      localStorage.setItem("listen1_settings", JSON.stringify(next));
    } catch {}
  }

  function persistDebounced(next: AppSettings) {
    if (persistTimer) clearTimeout(persistTimer);
    persistTimer = setTimeout(() => writeNow(next), 200);
  }

  return {
    subscribe,
    update,
    set(s: AppSettings) {
      set(s);
      writeNow(s);
    },
    patch(partial: Partial<AppSettings>) {
      let next = {} as AppSettings;
      update((s) => {
        next = { ...s, ...partial };
        return next;
      });
      const keys = Object.keys(partial) as (keyof AppSettings)[];
      if (keys.length > 0 && keys.every((key) => DEBOUNCED_KEYS.has(key))) {
        persistDebounced(next);
      } else {
        writeNow(next);
      }
    },
  };
}

export const settings = createSettingsStore();

/** 系统是否处于深色模式；仅在「自动」外观下被读取。 */
const systemDark = writable(
  typeof window !== "undefined" && window.matchMedia
    ? window.matchMedia("(prefers-color-scheme: dark)").matches
    : true,
);

if (typeof window !== "undefined" && window.matchMedia) {
  const mql = window.matchMedia("(prefers-color-scheme: dark)");
  mql.addEventListener("change", (e) => systemDark.set(e.matches));
}

/** 实际生效的主题：把「自动」折叠成 black2 / white2，其余原样透出。 */
export const resolvedTheme = derived(
  [settings, systemDark],
  ([$settings, $systemDark]): Exclude<AppSettings["theme"], "auto"> =>
    $settings.theme === "auto" ? ($systemDark ? "black2" : "white2") : $settings.theme,
);

