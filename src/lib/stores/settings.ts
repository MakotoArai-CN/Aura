import { writable, get } from "svelte/store";

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
  theme: "origin" | "origin2" | "black2" | "white2" | "liquidGlass";
  enableImmersivePlayer: boolean;
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
}

const defaults: AppSettings = {
  theme: "black2",
  enableImmersivePlayer: false,
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
      if (
        typeof (parsed as { enableImmersivePlayer?: unknown }).enableImmersivePlayer === "undefined" &&
        (parsed as { enableMineradioStage?: unknown }).enableMineradioStage === true
      ) {
        next.enableImmersivePlayer = true;
      }
      if ((parsed as { theme?: string; playerTheme?: string }).theme === "mineradio") {
        next.theme = "black2";
        next.enableImmersivePlayer = true;
      }
      if ((parsed as { playerTheme?: string }).playerTheme === "mineradio") {
        next.enableImmersivePlayer = true;
      }
      delete (next as AppSettings & { playerTheme?: unknown; enableMineradioStage?: unknown }).playerTheme;
      delete (next as AppSettings & { playerTheme?: unknown; enableMineradioStage?: unknown }).enableMineradioStage;
      return next;
    }
  } catch {}
  return { ...defaults };
}

function createSettingsStore() {
  const { subscribe, set, update } = writable<AppSettings>(loadSettings());

  return {
    subscribe,
    update,
    set(s: AppSettings) {
      set(s);
      localStorage.setItem("listen1_settings", JSON.stringify(s));
    },
    patch(partial: Partial<AppSettings>) {
      update((s) => {
        const next = { ...s, ...partial };
        localStorage.setItem("listen1_settings", JSON.stringify(next));
        return next;
      });
    },
  };
}

export const settings = createSettingsStore();
