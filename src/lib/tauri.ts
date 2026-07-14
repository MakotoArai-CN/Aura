import { invoke as tauriInvoke, isTauri as coreIsTauri } from "@tauri-apps/api/core";
import { emit as tauriEmit, emitTo as tauriEmitTo, listen as tauriListen, type EventCallback, type UnlistenFn } from "@tauri-apps/api/event";
import { disable as disableAutostartPlugin, enable as enableAutostartPlugin, isEnabled as isAutostartEnabledPlugin } from "@tauri-apps/plugin-autostart";
import { open as shellOpen } from "@tauri-apps/plugin-shell";

type TauriGlobal = typeof globalThis & {
  __TAURI__?: {
    core?: {
      invoke?: <T = unknown>(cmd: string, args?: Record<string, unknown>) => Promise<T>;
    };
    event?: {
      listen?: <T = unknown>(event: string, handler: EventCallback<T>) => Promise<UnlistenFn>;
      emit?: <T = unknown>(event: string, payload?: T) => Promise<void>;
      emitTo?: <T = unknown>(target: string, event: string, payload?: T) => Promise<void>;
    };
  };
  __TAURI_INTERNALS__?: unknown;
  isTauri?: boolean;
};

function tauriGlobal(): TauriGlobal {
  return globalThis as TauriGlobal;
}

function tauriInjectedApi() {
  const g = tauriGlobal();
  const w = typeof window === "undefined" ? undefined : (window as Window & TauriGlobal);
  return g.__TAURI__ ?? w?.__TAURI__;
}

export function isTauriRuntime(): boolean {
  if (typeof window === "undefined") return false;
  const g = tauriGlobal();
  const w = window as Window & TauriGlobal;
  try {
    if (coreIsTauri()) return true;
  } catch {
    // Fall through to the injected-global checks below.
  }
  return Boolean(
    g.isTauri ||
    g.__TAURI_INTERNALS__ ||
    tauriInjectedApi() ||
    w.__TAURI_INTERNALS__ ||
    w.__TAURI__ ||
    navigator.userAgent.includes("Tauri")
  );
}

function invoke<T = unknown>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauriRuntime()) {
    return Promise.reject(new Error(`Tauri command unavailable outside Tauri: ${cmd}`));
  }
  const injectedInvoke = tauriInjectedApi()?.core?.invoke;
  if (injectedInvoke) return injectedInvoke<T>(cmd, args);
  return tauriInvoke<T>(cmd, args);
}

export function getTauriRuntimeDiagnostics() {
  const g = tauriGlobal();
  const w = typeof window === "undefined" ? undefined : (window as Window & TauriGlobal);
  return {
    isTauriRuntime: isTauriRuntime(),
    hasGlobalTauri: Boolean(g.__TAURI__),
    hasWindowTauri: Boolean(w?.__TAURI__),
    hasGlobalInternals: Boolean(g.__TAURI_INTERNALS__),
    hasWindowInternals: Boolean(w?.__TAURI_INTERNALS__),
    hasInjectedInvoke: Boolean(g.__TAURI__?.core?.invoke ?? w?.__TAURI__?.core?.invoke),
    hasInjectedEvent: Boolean(g.__TAURI__?.event ?? w?.__TAURI__?.event),
    userAgent: typeof navigator === "undefined" ? "" : navigator.userAgent,
  };
}

export function listen<T>(event: string, handler: EventCallback<T>): Promise<UnlistenFn> {
  if (!isTauriRuntime()) return Promise.resolve(() => {});
  const injectedListen = tauriInjectedApi()?.event?.listen;
  if (injectedListen) return injectedListen<T>(event, handler);
  return tauriListen<T>(event, handler);
}

export function emit<T>(event: string, payload?: T): Promise<void> {
  if (!isTauriRuntime()) return Promise.resolve();
  const injectedEmit = tauriInjectedApi()?.event?.emit;
  if (injectedEmit) return injectedEmit<T>(event, payload);
  return tauriEmit(event, payload);
}

export function emitTo<T>(target: string, event: string, payload?: T): Promise<void> {
  if (!isTauriRuntime()) return Promise.resolve();
  const injectedEmitTo = tauriInjectedApi()?.event?.emitTo;
  if (injectedEmitTo) return injectedEmitTo<T>(target, event, payload);
  return tauriEmitTo(target, event, payload);
}

export async function openExternalUrl(url: string): Promise<void> {
  if (!url) return;
  if (isTauriRuntime()) {
    await shellOpen(url);
    return;
  }
  window.open(url, "_blank", "noopener,noreferrer");
}

export async function openLoginWindow(url: string): Promise<void> {
  if (!url) return;
  if (isTauriRuntime()) {
    await invoke("open_login_window", { url });
    return;
  }
  window.open(url, "_blank", "noopener,noreferrer");
}

export async function closeLoginWindow(): Promise<void> {
  if (!isTauriRuntime()) return;
  await invoke("close_login_window");
}

export async function syncLoginCookies(urls: string[]): Promise<number> {
  if (!isTauriRuntime()) return 0;
  return invoke<number>("sync_login_cookies", { urls });
}

export async function getLoginCookies(url: string): Promise<Record<string, string>> {
  if (!isTauriRuntime()) return {};
  return invoke<Record<string, string>>("get_login_cookies", { url });
}

export async function getBackendCookie(url: string, name: string): Promise<string | null> {
  if (!isTauriRuntime()) return null;
  return invoke<string | null>("get_backend_cookie", { url, name });
}

export async function setBackendCookie(url: string, cookie: string): Promise<void> {
  if (!isTauriRuntime()) return;
  await invoke("set_backend_cookie", { url, cookie });
}

export async function clearLoginCookies(url: string, names: string[]): Promise<void> {
  if (!isTauriRuntime()) return;
  await invoke("clear_login_cookies", { url, names });
}

export interface HttpRequestOptions {
  url: string;
  method?: "GET" | "POST" | "PUT" | "DELETE" | "PATCH";
  headers?: Record<string, string>;
  body?: string;
}

export interface HttpResponse<T = unknown> {
  status: number;
  headers: Record<string, string>;
  data: T;
  text?: string;
}

export async function httpRequest<T = unknown>(
  options: HttpRequestOptions
): Promise<HttpResponse<T>> {
  if (!isTauriRuntime()) {
    const resp = await fetch("/__listen1_http_request", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(options),
    });
    if (!resp.ok) {
      const message = await resp.text();
      throw new Error(message || `HTTP proxy failed: ${resp.status}`);
    }
    return (await resp.json()) as HttpResponse<T>;
  }
  return invoke<HttpResponse<T>>("http_request", { options });
}

export async function get<T = unknown>(
  url: string,
  headers?: Record<string, string>
): Promise<HttpResponse<T>> {
  return httpRequest<T>({ url, method: "GET", headers });
}

export async function post<T = unknown>(
  url: string,
  body: string | object,
  headers?: Record<string, string>
): Promise<HttpResponse<T>> {
  const bodyStr = typeof body === "string" ? body : JSON.stringify(body);
  const h: Record<string, string> = {
    "Content-Type": "application/json",
    ...headers,
  };
  return httpRequest<T>({ url, method: "POST", headers: h, body: bodyStr });
}

export async function postForm<T = unknown>(
  url: string,
  params: Record<string, string>,
  headers?: Record<string, string>
): Promise<HttpResponse<T>> {
  const body = new URLSearchParams(params).toString();
  return httpRequest<T>({
    url,
    method: "POST",
    headers: {
      "Content-Type": "application/x-www-form-urlencoded",
      ...headers,
    },
    body,
  });
}

export interface ProxyConfig {
  mode: "system" | "direct" | "manual";
  host?: string;
  port?: number;
  protocol?: string;
}

export const setProxyConfig = (config: ProxyConfig) =>
  invoke("set_proxy_config", { config });
export const getProxyConfig = (): Promise<ProxyConfig> =>
  invoke("get_proxy_config");
export const getAudioStreamUrl = (url: string, noCacheWrite = false): Promise<string> =>
  invoke("audio_stream_url", { url, noCacheWrite });

export const windowMinimize = () => invoke("window_minimize");
export const windowMaximize = () => invoke("window_maximize");
export const windowClose = () => invoke("window_close");
export const windowQuit = () => invoke("window_quit");
export const showFloatWindow = () => invoke("show_float_window");
export const hideFloatWindow = () => invoke("hide_float_window");
export const closeFloatWindow = () => invoke("close_float_window");
export const setFloatWindowHeight = (height: number) =>
  invoke("set_float_window_height", { height });
export const moveFloatWindow = (x: number, y: number) =>
  invoke("move_float_window", { x, y });
export const getFloatWindowPosition = (): Promise<[number, number]> =>
  invoke("get_float_window_position");
export const setFloatingLyricPayload = (payload: unknown): Promise<void> =>
  invoke("set_floating_lyric_payload", { payload: JSON.stringify(payload) });
export const getFloatingLyricPayload = async <T = unknown>(): Promise<T | null> => {
  const payload = await invoke<string | null>("get_floating_lyric_payload");
  if (!payload) return null;
  return JSON.parse(payload) as T;
};
export const setFloatingLyricSettings = (payload: unknown): Promise<void> =>
  invoke("set_floating_lyric_settings", { payload: JSON.stringify(payload) });
export const getFloatingLyricSettings = async <T = unknown>(): Promise<T | null> => {
  const payload = await invoke<string | null>("get_floating_lyric_settings");
  if (!payload) return null;
  return JSON.parse(payload) as T;
};

export interface AudioMeta {
  title?: string;
  artist?: string;
  album?: string;
  lyrics?: string;
  cover?: string;
  duration?: number;
  bitrate?: number;
}

export const readAudioTags = (path: string): Promise<AudioMeta> =>
  invoke("read_audio_tags", { path });

export const scanMusicDirectory = (directory: string): Promise<string[]> =>
  invoke("scan_music_directory", { directory });

export interface DownloadResult {
  path: string;
  file_name: string;
  metadata_written?: boolean;
  metadata_error?: string | null;
}

export interface DownloadMetadata {
  title?: string;
  artist?: string;
  album?: string;
  lyrics?: string;
  cover_url?: string;
}

export const defaultMusicDir = (): Promise<string> =>
  invoke("default_music_dir");

export const downloadTrackFile = (
  url: string,
  fileName: string,
  directory?: string,
  metadata?: DownloadMetadata
): Promise<DownloadResult> =>
  invoke("download_track", { url, fileName, directory, metadata });

export interface AudioCacheConfig {
  enabled: boolean;
  directory?: string;
  max_bytes: number;
}

export interface AudioCacheStats {
  enabled: boolean;
  directory: string;
  max_bytes: number;
  total_bytes: number;
  entry_count: number;
  hot_count: number;
  warm_count: number;
  cold_count: number;
}

export interface ResourceUsage {
  memory_bytes: number;
  cpu_percent: number;
  process_count: number;
  webview_count: number;
}

export const defaultCacheDir = (): Promise<string> =>
  invoke("default_cache_dir");
export const setCacheConfig = (config: AudioCacheConfig) =>
  invoke("set_cache_config", { config });
export const getCacheStats = (): Promise<AudioCacheStats> =>
  invoke("get_cache_stats");
export const clearAudioCache = () =>
  invoke("clear_audio_cache");
export const getResourceUsage = (): Promise<ResourceUsage> =>
  invoke("get_resource_usage");

export async function setAutostart(enabled: boolean): Promise<void> {
  if (!isTauriRuntime()) return;
  if (enabled) await enableAutostartPlugin();
  else await disableAutostartPlugin();
}

export async function isAutostartEnabled(): Promise<boolean> {
  if (!isTauriRuntime()) return false;
  return isAutostartEnabledPlugin();
}
