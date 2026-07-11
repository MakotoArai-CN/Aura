import type { IncomingMessage, ServerResponse } from "node:http";
import { defineConfig, type Plugin } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

const host = process.env.TAURI_DEV_HOST;
const DEV_HTTP_PROXY_PATH = "/__listen1_http_request";
const MOBILE_UA = "Mozilla/5.0 (iPhone; CPU iPhone OS 14_3 like Mac OS X) AppleWebKit/534.30 (KHTML, like Gecko) Version/4.0 Mobile Safari/534.30";
const DESKTOP_UA = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_14_2) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/72.0.3626.119 Safari/537.36";

interface HttpRequestOptions {
  url: string;
  method?: "GET" | "POST" | "PUT" | "DELETE" | "PATCH";
  headers?: Record<string, string>;
  body?: string;
}

function readBody(req: IncomingMessage): Promise<string> {
  return new Promise((resolve, reject) => {
    let body = "";
    req.setEncoding("utf8");
    req.on("data", (chunk) => {
      body += chunk;
      if (body.length > 10 * 1024 * 1024) {
        reject(new Error("Request body is too large"));
        req.destroy();
      }
    });
    req.on("end", () => resolve(body));
    req.on("error", reject);
  });
}

function writeText(res: ServerResponse, status: number, text: string) {
  res.statusCode = status;
  res.setHeader("Content-Type", "text/plain; charset=utf-8");
  res.end(text);
}

function writeJson(res: ServerResponse, value: unknown) {
  res.statusCode = 200;
  res.setHeader("Content-Type", "application/json; charset=utf-8");
  res.end(JSON.stringify(value));
}

function hasHeader(headers: Record<string, string>, name: string): boolean {
  return Object.keys(headers).some((key) => key.toLowerCase() === name.toLowerCase());
}

function setHeaderIfAbsent(headers: Record<string, string>, name: string, value: string) {
  if (!hasHeader(headers, name)) headers[name] = value;
}

function injectHeaders(url: string, headers: Record<string, string>) {
  let referer = "";
  let origin = "";
  let ua = "";
  let defaultOrigin = true;

  if (url.includes("://music.163.com/") || url.includes("://interface3.music.163.com/")) {
    referer = "http://music.163.com/";
  }
  if (url.includes("://gist.githubusercontent.com/")) {
    referer = "https://gist.githubusercontent.com/";
  }
  if (url.includes(".xiami.com/")) {
    referer = "https://www.xiami.com/";
  }
  if (url.includes("c.y.qq.com/")) {
    referer = "https://y.qq.com/";
    origin = "https://y.qq.com";
  }
  if (
    url.includes("y.qq.com/") ||
    url.includes("qpic.y.qq.com/") ||
    url.includes("y.gtimg.cn/") ||
    url.includes("qqmusic.qq.com/") ||
    url.includes("imgcache.qq.com/")
  ) {
    referer = "https://y.qq.com/";
  }
  if (url.includes(".kugou.com/")) {
    referer = "https://www.kugou.com/";
    ua = MOBILE_UA;
  }
  if (url.includes(".kuwo.cn/")) {
    referer = "http://www.kuwo.cn/";
  }
  if (url.includes(".bilibili.com/") || url.includes(".bilivideo.com/") || url.includes(".hdslb.com/")) {
    referer = "https://www.bilibili.com/";
    defaultOrigin = false;
  }
  if (url.includes(".bilivideo.cn")) {
    referer = "https://www.bilibili.com/";
    origin = "https://www.bilibili.com/";
    defaultOrigin = false;
  }
  if (url.includes(".migu.cn")) {
    referer = "http://music.migu.cn/v3/music/player/audio?from=migu";
  }
  if (url.includes("m.music.migu.cn")) {
    referer = "https://m.music.migu.cn/";
  }
  if (url.includes(".taihe.com/") || url.includes("music.91q.com")) {
    referer = "https://music.taihe.com/";
  }

  if (referer) setHeaderIfAbsent(headers, "Referer", referer);
  if (origin) {
    setHeaderIfAbsent(headers, "Origin", origin);
  } else if (defaultOrigin && referer) {
    setHeaderIfAbsent(headers, "Origin", referer);
  }
  if (ua) {
    headers["User-Agent"] = ua;
  } else {
    setHeaderIfAbsent(headers, "User-Agent", DESKTOP_UA);
  }
}

function charsetFromContentType(contentType: string): string | undefined {
  return contentType
    .split(";")
    .map((part) => part.trim())
    .find((part) => part.toLowerCase().startsWith("charset="))
    ?.slice("charset=".length)
    .trim()
    .replace(/^"|"$/g, "");
}

function decodeWith(label: string, bytes: Uint8Array, fatal = false): string | undefined {
  try {
    return new TextDecoder(label, { fatal }).decode(bytes);
  } catch {
    return undefined;
  }
}

function decodeResponseText(headers: Headers, bytes: Uint8Array): string {
  const utf8 = decodeWith("utf-8", bytes, true);
  if (utf8 !== undefined) return utf8;

  const charset = charsetFromContentType(headers.get("content-type") ?? "");
  if (charset) {
    const decoded = decodeWith(charset, bytes);
    if (decoded !== undefined) return decoded;
  }

  return (
    decodeWith("gb18030", bytes) ??
    decodeWith("gbk", bytes) ??
    decodeWith("utf-8", bytes) ??
    ""
  );
}

function listen1HttpProxy(): Plugin {
  return {
    name: "listen1-http-proxy",
    configureServer(server) {
      server.middlewares.use(DEV_HTTP_PROXY_PATH, async (req, res) => {
        try {
          if (req.method !== "POST") {
            writeText(res, 405, "Method not allowed");
            return;
          }

          const options = JSON.parse(await readBody(req)) as HttpRequestOptions;
          if (!/^https?:\/\//i.test(options.url)) {
            writeText(res, 400, "Only http(s) urls can be proxied");
            return;
          }

          const method = (options.method ?? "GET").toUpperCase();
          const reqHeaders = { ...(options.headers ?? {}) };
          injectHeaders(options.url, reqHeaders);

          const init: RequestInit = {
            method,
            headers: reqHeaders,
          };
          if (options.body && method !== "GET") {
            init.body = options.body;
          }

          const upstream = await fetch(options.url, init);
          const respHeaders: Record<string, string> = {};
          upstream.headers.forEach((value, key) => {
            respHeaders[key] = value;
          });

          const bytes = new Uint8Array(await upstream.arrayBuffer());
          const text = decodeResponseText(upstream.headers, bytes);
          let data: unknown = text;
          try {
            data = JSON.parse(text);
          } catch {
            // Keep HTML or plain lyric responses as text.
          }

          writeJson(res, {
            status: upstream.status,
            headers: respHeaders,
            data,
            text,
          });
        } catch (error) {
          writeText(res, 500, error instanceof Error ? error.message : String(error));
        }
      });
    },
  };
}

export default defineConfig({
  plugins: [listen1HttpProxy(), svelte()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  build: {
    rollupOptions: {
      input: {
        main: "index.html",
        float: "float.html",
        login: "login.html",
      },
      output: {
        manualChunks(id) {
          if (!id.includes("node_modules")) return undefined;
          if (id.includes("@tauri-apps")) return "tauri";
          if (id.includes("svelte")) return "svelte";
          if (id.includes("liquid-glass-js")) return "liquid-glass";
          if (id.includes("node-forge")) return "crypto";
          return "vendor";
        },
      },
    },
  },
});
