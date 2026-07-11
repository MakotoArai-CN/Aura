import { get } from "svelte/store";
import { settings } from "./stores/settings";

export type ShortcutAction =
  | "togglePlay"
  | "prevTrack"
  | "nextTrack"
  | "seekBackward"
  | "seekForward"
  | "volumeUp"
  | "volumeDown"
  | "mute"
  | "search"
  | "closeNowPlaying"
  | "stop";

export const SHORTCUT_ACTIONS: Array<{ id: ShortcutAction; label: string; scope: "both" | "window" | "global" }> = [
  { id: "togglePlay", label: "播放/暂停", scope: "both" },
  { id: "prevTrack", label: "上一首", scope: "both" },
  { id: "nextTrack", label: "下一首", scope: "both" },
  { id: "seekBackward", label: "快退", scope: "window" },
  { id: "seekForward", label: "快进", scope: "window" },
  { id: "volumeUp", label: "音量增加", scope: "both" },
  { id: "volumeDown", label: "音量降低", scope: "both" },
  { id: "mute", label: "静音", scope: "both" },
  { id: "search", label: "搜索", scope: "window" },
  { id: "closeNowPlaying", label: "收起播放页", scope: "window" },
  { id: "stop", label: "停止播放", scope: "global" },
];

const KEY_ALIASES: Record<string, string> = {
  " ": "Space",
  Esc: "Escape",
  Left: "ArrowLeft",
  Right: "ArrowRight",
  Up: "ArrowUp",
  Down: "ArrowDown",
  "[": "BracketLeft",
  "]": "BracketRight",
  ",": "Comma",
  ".": "Period",
};

const LABELS: Record<string, string> = {
  Space: "Space",
  Escape: "Esc",
  ArrowLeft: "←",
  ArrowRight: "→",
  ArrowUp: "↑",
  ArrowDown: "↓",
  BracketLeft: "[",
  BracketRight: "]",
  Comma: ",",
  Period: ".",
  MediaPlayPause: "Media Play",
  MediaNextTrack: "Media Next",
  MediaPreviousTrack: "Media Prev",
  MediaStop: "Media Stop",
};

function normalizeKeyName(key: string): string {
  const trimmed = key.trim();
  const aliased = KEY_ALIASES[trimmed] ?? trimmed;
  if (/^Key[A-Z]$/.test(aliased)) return aliased.slice(3);
  if (/^Digit[0-9]$/.test(aliased)) return aliased.slice(5);
  if (aliased.length === 1) return aliased.toUpperCase();
  return aliased;
}

export function normalizeShortcut(value: string): string {
  const parts = value
    .split("+")
    .map((part) => part.trim())
    .filter(Boolean);
  if (parts.length === 0) return "";

  const key = normalizeKeyName(parts[parts.length - 1]);
  const mods = new Set(parts.slice(0, -1).map((part) => {
    const lower = part.toLowerCase();
    if (lower === "cmdorctrl" || lower === "commandorcontrol") return "CmdOrCtrl";
    if (lower === "cmd" || lower === "meta" || lower === "command") return "Meta";
    if (lower === "ctrl" || lower === "control") return "Ctrl";
    if (lower === "option") return "Alt";
    if (lower === "alt") return "Alt";
    if (lower === "shift") return "Shift";
    return part;
  }));

  return [
    mods.has("CmdOrCtrl") ? "CmdOrCtrl" : "",
    mods.has("Ctrl") ? "Ctrl" : "",
    mods.has("Meta") ? "Meta" : "",
    mods.has("Alt") ? "Alt" : "",
    mods.has("Shift") ? "Shift" : "",
    key,
  ].filter(Boolean).join("+");
}

export function eventToShortcut(event: KeyboardEvent, global = false): string {
  const key = normalizeKeyName(event.code && event.code !== "Unidentified" ? event.code : event.key);
  const mods = [
    global && (event.ctrlKey || event.metaKey) ? "CmdOrCtrl" : "",
    !global && event.ctrlKey ? "Ctrl" : "",
    !global && event.metaKey ? "Meta" : "",
    event.altKey ? "Alt" : "",
    event.shiftKey ? "Shift" : "",
  ].filter(Boolean);
  return normalizeShortcut([...mods, key].join("+"));
}

export function shortcutLabel(value: string): string {
  const normalized = normalizeShortcut(value);
  return normalized
    .split("+")
    .filter(Boolean)
    .map((part) => LABELS[part] ?? part)
    .join(" + ");
}

export function shortcutHasModifier(value: string): boolean {
  const normalized = normalizeShortcut(value);
  return /(^|\+)(CmdOrCtrl|Ctrl|Meta|Alt|Shift)\+/.test(normalized) || normalized.startsWith("Media");
}

export function validateShortcutMap(map: Record<string, string[]>): { ok: true } | { ok: false; message: string } {
  const seen = new Map<string, string>();
  for (const action of Object.keys(map)) {
    for (const raw of map[action] ?? []) {
      const shortcut = normalizeShortcut(raw);
      if (!shortcut) continue;
      const existing = seen.get(shortcut);
      if (existing && existing !== action) {
        const currentLabel = SHORTCUT_ACTIONS.find((item) => item.id === action)?.label ?? action;
        const existingLabel = SHORTCUT_ACTIONS.find((item) => item.id === existing)?.label ?? existing;
        return { ok: false, message: `${shortcutLabel(shortcut)} 已用于 ${existingLabel}，不能再分配给 ${currentLabel}` };
      }
      seen.set(shortcut, action);
    }
  }
  return { ok: true };
}

export function actionForShortcut(map: Record<string, string[]>, shortcut: string): ShortcutAction | null {
  const normalized = normalizeShortcut(shortcut);
  for (const [action, shortcuts] of Object.entries(map)) {
    if ((shortcuts ?? []).some((value) => normalizeShortcut(value) === normalized)) return action as ShortcutAction;
  }
  return null;
}

export function currentShortcuts() {
  const s = get(settings);
  return {
    keyboard: s.keyboardShortcuts,
    global: s.globalShortcuts,
  };
}
