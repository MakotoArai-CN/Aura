import { writable } from "svelte/store";

export type ToastKind = "info" | "success" | "warn" | "error";
export interface ToastItem {
  id: number;
  kind: ToastKind;
  message: string;
  duration: number;
}

const items = writable<ToastItem[]>([]);
let seq = 0;

export const toasts = { subscribe: items.subscribe };

export function pushToast(message: string, opts: { kind?: ToastKind; duration?: number } = {}) {
  const t: ToastItem = {
    id: ++seq,
    kind: opts.kind ?? "info",
    message,
    duration: opts.duration ?? 2600,
  };
  items.update((list) => [...list, t]);
  setTimeout(() => removeToast(t.id), t.duration);
  return t.id;
}

export function removeToast(id: number) {
  items.update((list) => list.filter((t) => t.id !== id));
}

// Convenience shortcuts
export const toast = {
  info: (m: string, d?: number) => pushToast(m, { kind: "info", duration: d }),
  success: (m: string, d?: number) => pushToast(m, { kind: "success", duration: d }),
  warn: (m: string, d?: number) => pushToast(m, { kind: "warn", duration: d }),
  error: (m: string, d?: number) => pushToast(m, { kind: "error", duration: d }),
};
