import { register, unregisterAll } from "@tauri-apps/plugin-global-shortcut";
import { player } from "./player";
import { get } from "svelte/store";
import { settings } from "./stores/settings";
import { SHORTCUT_ACTIONS, type ShortcutAction, normalizeShortcut, validateShortcutMap } from "./shortcutConfig";

let enabled = false;
let registeredKeys: string[] = [];

function runAction(action: ShortcutAction) {
  if (action === "togglePlay") player.togglePlayPause();
  else if (action === "prevTrack") player.skip("prev");
  else if (action === "nextTrack") player.skip("next");
  else if (action === "volumeUp") {
    player.adjustVolume(true);
    player.unmute();
  } else if (action === "volumeDown") player.adjustVolume(false);
  else if (action === "mute") player.toggleMute();
  else if (action === "stop") player.pause();
}

function configuredShortcuts() {
  const validation = validateShortcutMap(get(settings).globalShortcuts);
  if (!validation.ok) {
    console.warn("[shortcuts] invalid global shortcut map", validation.message);
    return [];
  }
  const actions = new Set(SHORTCUT_ACTIONS.filter((item) => item.scope !== "window").map((item) => item.id));
  return Object.entries(get(settings).globalShortcuts)
    .filter(([action]) => actions.has(action as ShortcutAction))
    .flatMap(([action, keys]) =>
      (keys ?? [])
        .map(normalizeShortcut)
        .filter(Boolean)
        .map((keys) => ({ keys, action: () => runAction(action as ShortcutAction) }))
    );
}

export async function enableGlobalShortcuts() {
  if (enabled) return;
  registeredKeys = [];
  try {
    for (const s of configuredShortcuts()) {
      await register(s.keys, s.action).then(() => {
        registeredKeys.push(s.keys);
      }).catch((e) => {
        console.warn(`Global shortcut not available: ${s.keys}`, e);
      });
    }
    enabled = registeredKeys.length > 0;
  } catch (e) {
    console.warn("Global shortcuts not available:", e);
  }
}

export async function refreshGlobalShortcuts() {
  await disableGlobalShortcuts();
  await enableGlobalShortcuts();
}

export async function disableGlobalShortcuts() {
  if (!enabled) return;
  try {
    await unregisterAll();
    enabled = false;
    registeredKeys = [];
  } catch {}
}
