import { isTauriRuntime } from "./tauri";

const BROKEN_STREAM_PREFIX = "http://stream.localhost/";
const LEGACY_STREAM_PREFIX = "stream://localhost/";
const LOCAL_STREAM_RE = /^http:\/\/(?:127\.0\.0\.1|localhost):\d+\/stream\/([^?]+)/i;
const NEEDS_REFERER_PROXY_RE = /^https?:\/\/[^/]*(?:hdslb\.com|bilibili\.com|bilivideo\.com|bilivideo\.cn)\//i;
const TAURI_STREAM_PREFIX = "stream://localhost/";

function streamBaseUrl(): string {
  if (typeof window === "undefined") return "";
  return (window as Window & { __LISTEN1_STREAM_BASE_URL__?: string }).__LISTEN1_STREAM_BASE_URL__ ?? "";
}

function normalizeUrl(url: string): string {
  let normalized = url.trim();
  if (!normalized) return "";
  if (normalized.startsWith("//")) normalized = `https:${normalized}`;
  if (normalized.startsWith("http://qpic.y.qq.com/")) {
    normalized = normalized.replace("http://", "https://");
  }
  return normalized;
}

function decodeStreamTarget(url: string): string | null {
  const localMatch = url.match(LOCAL_STREAM_RE);
  const encodedTarget = localMatch?.[1]
    ?? (url.startsWith(BROKEN_STREAM_PREFIX) ? url.slice(BROKEN_STREAM_PREFIX.length).split("?")[0] : null)
    ?? (url.startsWith(LEGACY_STREAM_PREFIX) ? url.slice(LEGACY_STREAM_PREFIX.length).split("?")[0] : null);
  if (!encodedTarget) return null;
  try {
    return decodeURIComponent(encodedTarget);
  } catch {
    return encodedTarget;
  }
}

export function proxyResourceUrl(url?: string | null): string {
  const normalized = normalizeUrl(url ?? "");
  if (!normalized) return "";
  if (
    normalized.startsWith("data:") ||
    normalized.startsWith("blob:")
  ) {
    return normalized;
  }
  const decoded = decodeStreamTarget(normalized);
  if (decoded) return decoded;
  const streamBase = streamBaseUrl();
  if (NEEDS_REFERER_PROXY_RE.test(normalized)) {
    const encoded = encodeURIComponent(normalized);
    if (streamBase) return `${streamBase}${encoded}`;
    if (isTauriRuntime()) return `${TAURI_STREAM_PREFIX}${encoded}`;
  }
  return normalized;
}

export function cssImageUrl(url?: string | null): string {
  const proxied = proxyResourceUrl(url);
  if (!proxied) return "";
  return `url("${proxied.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}")`;
}
