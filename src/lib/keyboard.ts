import { player } from "./player";
import { get } from "svelte/store";
import { settings } from "./stores/settings";
import { actionForShortcut, eventToShortcut, type ShortcutAction } from "./shortcutConfig";

type Navigate = (view: unknown) => void;
type CloseNowPlaying = () => void;

function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  const tag = target.tagName.toLowerCase();
  return tag === "input" || tag === "textarea" || tag === "select";
}

export function isActionKey(event: KeyboardEvent): boolean {
  return event.key === "Enter" || event.key === " ";
}

export function runOnActionKey(event: KeyboardEvent, action: () => void) {
  if (!isActionKey(event)) return;
  event.preventDefault();
  action();
}

export function bindPlayerKeyboard(options: {
  navigate: Navigate;
  isNowPlaying: () => boolean;
  closeNowPlaying: CloseNowPlaying;
}) {
  function runAction(action: ShortcutAction, event: KeyboardEvent, seekStep: number) {
    if (action === "togglePlay") player.togglePlayPause();
    else if (action === "prevTrack") player.skip("prev");
    else if (action === "nextTrack") player.skip("next");
    else if (action === "seekBackward") player.seekRelative(-seekStep);
    else if (action === "seekForward") player.seekRelative(seekStep);
    else if (action === "volumeUp") {
      player.adjustVolume(true);
      player.unmute();
    } else if (action === "volumeDown") player.adjustVolume(false);
    else if (action === "mute") player.toggleMute();
    else if (action === "search") options.navigate({ type: "search" });
    else if (action === "closeNowPlaying" && options.isNowPlaying()) options.closeNowPlaying();
    else return false;
    event.preventDefault();
    return true;
  }

  function handleKey(event: KeyboardEvent) {
    if (event.defaultPrevented || event.repeat || isEditableTarget(event.target)) return;

    const seekStep = event.shiftKey ? 15 : 5;
    const action = actionForShortcut(get(settings).keyboardShortcuts, eventToShortcut(event));
    if (action) runAction(action, event, seekStep);
  }

  window.addEventListener("keydown", handleKey);
  return () => window.removeEventListener("keydown", handleKey);
}
