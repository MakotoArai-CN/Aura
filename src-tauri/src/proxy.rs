use crate::cache;
use reqwest::header;
use reqwest::cookie::{CookieStore, Jar};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Duration;

/// 进程级共享 tokio Runtime：替代每请求创建 Runtime（创建成本高且产生额外线程）。
pub(crate) fn shared_runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("failed to build shared tokio runtime")
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyConfig {
    pub mode: String, // "system" | "direct" | "manual"
    pub host: Option<String>,
    pub port: Option<u16>,
    pub protocol: Option<String>, // "http" | "socks5" etc.
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            mode: "system".into(),
            host: None,
            port: None,
            protocol: None,
        }
    }
}

fn proxy_config() -> &'static Mutex<ProxyConfig> {
    static PROXY_CONFIG: OnceLock<Mutex<ProxyConfig>> = OnceLock::new();
    PROXY_CONFIG.get_or_init(|| Mutex::new(ProxyConfig::default()))
}

/// 响应正文。磁盘上已有的内容（本地音乐、缓存命中）不进内存：只带一个已经 seek 到
/// 起点的文件句柄和该读多少字节，由 write_stream_response 用固定缓冲区搬到 socket。
///
/// 原先两条路径都是 `vec![0; read_len]` + `read_exact` 把整段读进内存再一次 write_all。
/// 播放器不带 Range 的首个请求 read_len 就是整个文件，一首 FLAC 是几十 MB 的分配 +
/// 整文件 memcpy；而响应带 `Connection: close`、服务端又是每连接一个线程，播放器每次
/// seek 都要把这一整套重来一遍。
#[derive(Debug)]
pub(crate) enum StreamBody {
    Bytes(Vec<u8>),
    File { file: std::fs::File, len: u64 },
}

impl StreamBody {
    pub(crate) fn len(&self) -> u64 {
        match self {
            StreamBody::Bytes(bytes) => bytes.len() as u64,
            StreamBody::File { len, .. } => *len,
        }
    }

    /// 仅供必须交出 `Vec<u8>` 的场合（tauri 自定义协议只接受完整正文）。会把这一段
    /// 真的读进内存，所以别在 TCP 那条路上用。
    pub(crate) fn into_bytes(self) -> Vec<u8> {
        match self {
            StreamBody::Bytes(bytes) => bytes,
            StreamBody::File { mut file, len } => {
                let mut bytes = Vec::with_capacity(len as usize);
                match std::io::Read::by_ref(&mut file)
                    .take(len)
                    .read_to_end(&mut bytes)
                {
                    Ok(_) => bytes,
                    Err(_) => Vec::new(),
                }
            }
        }
    }
}

#[derive(Debug)]
pub(crate) struct StreamResponse {
    pub(crate) status: u16,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: StreamBody,
}

impl StreamResponse {
    fn empty(status: u16) -> Self {
        Self {
            status,
            headers: vec![("Access-Control-Allow-Origin".into(), "*".into())],
            body: StreamBody::Bytes(Vec::new()),
        }
    }
}

fn stream_base_url() -> &'static OnceLock<String> {
    static STREAM_BASE_URL: OnceLock<String> = OnceLock::new();
    &STREAM_BASE_URL
}

fn cookie_jar() -> Arc<Jar> {
    static COOKIE_JAR: OnceLock<Arc<Jar>> = OnceLock::new();
    COOKIE_JAR
        .get_or_init(|| Arc::new(Jar::default()))
        .clone()
}

pub(crate) fn add_cookie_for_url(url: &str, cookie: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| e.to_string())?;
    cookie_jar().add_cookie_str(cookie, &parsed);
    Ok(())
}

#[tauri::command]
pub fn set_backend_cookie(url: String, cookie: String) -> Result<(), String> {
    add_cookie_for_url(&url, &cookie)
}

pub(crate) fn clear_cookie_for_url(url: &str, name: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| e.to_string())?;
    cookie_jar().add_cookie_str(&format!("{name}=; Max-Age=0; Path=/"), &parsed);
    Ok(())
}

#[tauri::command]
pub fn get_backend_cookie(url: String, name: String) -> Result<Option<String>, String> {
    let parsed = reqwest::Url::parse(&url).map_err(|e| e.to_string())?;
    let Some(value) = cookie_jar().cookies(&parsed) else {
        return Ok(None);
    };
    let text = value.to_str().map_err(|e| e.to_string())?;
    Ok(text.split(';').find_map(|part| {
        let (key, value) = part.trim().split_once('=')?;
        (key == name).then(|| value.to_string())
    }))
}

pub fn ensure_stream_server() -> Result<&'static str, String> {
    if let Some(base) = stream_base_url().get() {
        return Ok(base.as_str());
    }

    let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|e| e.to_string())?;
    let addr = listener.local_addr().map_err(|e| e.to_string())?;
    let base_url = format!("http://127.0.0.1:{}/stream/", addr.port());

    thread::Builder::new()
        .name("listen1-stream-server".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let _ = thread::Builder::new()
                    .name("listen1-stream-client".into())
                    .spawn(move || handle_stream_connection(stream));
            }
        })
        .map_err(|e| e.to_string())?;

    let _ = stream_base_url().set(base_url);
    stream_base_url()
        .get()
        .map(String::as_str)
        .ok_or_else(|| "stream server failed to start".to_string())
}

#[tauri::command]
pub fn set_proxy_config(config: ProxyConfig) {
    if let Ok(mut guard) = proxy_config().lock() {
        *guard = config;
    }
}

#[tauri::command]
pub fn get_proxy_config() -> ProxyConfig {
    proxy_config().lock().map(|g| g.clone()).unwrap_or_default()
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HttpRequestOptions {
    pub url: String,
    pub method: Option<String>,
    pub headers: Option<HashMap<String, String>>,
    pub body: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub data: serde_json::Value,
    pub text: Option<String>,
}

const MOBILE_UA: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 14_3 like Mac OS X) AppleWebKit/534.30 (KHTML, like Gecko) Version/4.0 Mobile Safari/534.30";
const DESKTOP_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_14_2) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/72.0.3626.119 Safari/537.36";

pub(crate) fn inject_headers(url: &str, headers: &mut HashMap<String, String>) {
    let mut referer = String::new();
    let mut origin = String::new();
    let mut ua = String::new();
    let mut default_origin = true;

    if url.contains("://music.163.com/") || url.contains("://interface3.music.163.com/") {
        referer = "http://music.163.com/".into();
    }
    if url.contains("://gist.githubusercontent.com/") {
        referer = "https://gist.githubusercontent.com/".into();
    }
    if url.contains(".xiami.com/") {
        referer = "https://www.xiami.com/".into();
    }
    if url.contains("c.y.qq.com/") {
        referer = "https://y.qq.com/".into();
        origin = "https://y.qq.com".into();
    }
    if url.contains("y.qq.com/")
        || url.contains("qpic.y.qq.com/")
        || url.contains("y.gtimg.cn/")
        || url.contains("qqmusic.qq.com/")
        || url.contains("imgcache.qq.com/")
    {
        referer = "https://y.qq.com/".into();
    }
    if url.contains(".kugou.com/") {
        referer = "https://www.kugou.com/".into();
        ua = MOBILE_UA.into();
    }
    if url.contains(".kuwo.cn/") {
        referer = "http://www.kuwo.cn/".into();
    }
    if url.contains(".bilibili.com/")
        || url.contains(".bilivideo.com/")
        || url.contains(".hdslb.com/")
    {
        referer = "https://www.bilibili.com/".into();
        default_origin = false;
    }
    if url.contains(".bilivideo.cn") {
        referer = "https://www.bilibili.com/".into();
        origin = "https://www.bilibili.com/".into();
        default_origin = false;
    }
    if url.contains(".migu.cn") {
        referer = "http://music.migu.cn/v3/music/player/audio?from=migu".into();
    }
    if url.contains("m.music.migu.cn") {
        referer = "https://m.music.migu.cn/".into();
    }
    if url.contains(".taihe.com/") || url.contains("music.91q.com") {
        referer = "https://music.taihe.com/".into();
    }

    if !referer.is_empty() {
        headers.entry("Referer".into()).or_insert(referer.clone());
    }
    if !origin.is_empty() {
        headers.entry("Origin".into()).or_insert(origin);
    } else if default_origin && !referer.is_empty() {
        headers.entry("Origin".into()).or_insert(referer);
    }
    if !ua.is_empty() {
        headers.insert("User-Agent".into(), ua);
    } else {
        headers
            .entry("User-Agent".into())
            .or_insert(DESKTOP_UA.into());
    }
}

pub(crate) fn build_client() -> Result<reqwest::Client, String> {
    let config = proxy_config().lock().map(|g| g.clone()).unwrap_or_default();
    let mode = if config.mode.is_empty() {
        "direct".to_string()
    } else {
        config.mode.clone()
    };

    let mut builder = reqwest::Client::builder()
        .cookie_provider(cookie_jar())
        .connect_timeout(Duration::from_secs(12))
        .timeout(Duration::from_secs(25));

    match mode.as_str() {
        "system" => {
            // use system proxy (reqwest default)
        }
        "manual" => {
            if let (Some(host), Some(port)) = (&config.host, config.port) {
                let protocol = config.protocol.as_deref().unwrap_or("http");
                let proxy_url = format!("{}://{}:{}", protocol, host, port);
                let proxy = reqwest::Proxy::all(&proxy_url).map_err(|e| e.to_string())?;
                builder = builder.proxy(proxy);
            }
        }
        _ => {
            // "direct" — bypass all proxies
            builder = builder.no_proxy();
        }
    }

    builder.build().map_err(|e| e.to_string())
}

fn charset_from_content_type(content_type: &str) -> Option<&str> {
    content_type.split(';').find_map(|part| {
        let (key, value) = part.trim().split_once('=')?;
        key.trim()
            .eq_ignore_ascii_case("charset")
            .then(|| value.trim().trim_matches('"'))
    })
}

fn decode_text_response(headers: &HashMap<String, String>, bytes: &[u8]) -> String {
    let (utf8_text, _, had_errors) = encoding_rs::UTF_8.decode(bytes);
    if !had_errors {
        return utf8_text.into_owned();
    }

    let content_type = headers
        .get("content-type")
        .or_else(|| headers.get("Content-Type"))
        .map(String::as_str)
        .unwrap_or("");

    if let Some(charset) = charset_from_content_type(content_type) {
        if let Some(encoding) = encoding_rs::Encoding::for_label(charset.as_bytes()) {
            let (text, _, _) = encoding.decode(bytes);
            return text.into_owned();
        }
    }

    let (gb_text, _, _) = encoding_rs::GB18030.decode(bytes);
    gb_text.into_owned()
}

pub(crate) fn file_url_to_path(url: &str) -> PathBuf {
    let raw = if let Some(path) = url.strip_prefix("file:///") {
        #[cfg(windows)]
        {
            path
        }
        #[cfg(not(windows))]
        {
            return PathBuf::from(format!("/{}", path));
        }
    } else if let Some(path) = url.strip_prefix("file://") {
        path
    } else {
        url
    };

    let decoded = urlencoding::decode(raw)
        .map(|s| s.to_string())
        .unwrap_or_else(|_| raw.to_string());
    PathBuf::from(decoded)
}

/// 本机文件的 Content-Type。除音频外还要认图片：本地音乐的内嵌封面会落盘成
/// jpg/png，而 webview 不允许从 http/tauri 源直接加载 `file://`，只能经这台服务器取。
fn local_content_type(path: &PathBuf) -> &'static str {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "aac" | "m4a" | "mp4" => "audio/mp4",
        "flac" => "audio/flac",
        "mp3" => "audio/mpeg",
        "ogg" | "oga" | "opus" => "audio/ogg",
        "wav" => "audio/wav",
        "aif" | "aiff" => "audio/aiff",
        "webm" => "audio/webm",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        _ => "application/octet-stream",
    }
}

fn parse_range_header(range: &str, len: u64) -> Option<(u64, u64)> {
    if len == 0 {
        return None;
    }

    let value = range.trim().strip_prefix("bytes=")?;
    let first = value.split(',').next()?.trim();
    let (start_raw, end_raw) = first.split_once('-')?;

    if start_raw.is_empty() {
        let suffix_len = end_raw.parse::<u64>().ok()?;
        if suffix_len == 0 {
            return None;
        }
        let start = len.saturating_sub(suffix_len);
        return Some((start, len - 1));
    }

    let start = start_raw.parse::<u64>().ok()?;
    if start >= len {
        return None;
    }

    let end = if end_raw.is_empty() {
        len - 1
    } else {
        end_raw.parse::<u64>().ok()?.min(len - 1)
    };

    if start > end {
        return None;
    }

    Some((start, end))
}

fn range_starts_at_zero(range: Option<&str>) -> bool {
    let Some(range) = range else {
        return false;
    };
    let Some(value) = range.trim().strip_prefix("bytes=") else {
        return false;
    };
    let Some(first) = value.split(',').next() else {
        return false;
    };
    let Some((start, _)) = first.trim().split_once('-') else {
        return false;
    };
    start.trim() == "0"
}

fn should_fetch_full_for_cache(url: &str) -> bool {
    let value = url.to_ascii_lowercase();
    !(value.contains(".bilivideo.com/")
        || value.contains(".bilivideo.cn")
        || value.contains(".mcdn.bilivideo")
        || value.contains(".bilibili.com/")
        || value.contains(".hdslb.com/"))
}

fn cache_expose_headers() -> &'static str {
    "Content-Length, Content-Range, Accept-Ranges, X-Listen1-Cache"
}

/// 响应体是否覆盖了整个资源，因而可以整段落盘。
///
/// 没有 Content-Range 就是完整的 200 响应。哔哩哔哩的音频必须原样转发 Range
/// （见 `should_fetch_full_for_cache`），所以它的首个 `bytes=0-` 请求拿回来的是
/// `206 bytes 0-<len-1>/<len>`——内容其实是全的。只看「有没有 Content-Range」
/// 会把哔哩哔哩永久排除在缓存之外，导致断网后这一路完全不可播。
fn content_range_covers_whole(content_range: Option<&str>) -> bool {
    let Some(value) = content_range.map(str::trim).filter(|v| !v.is_empty()) else {
        return true;
    };
    let spec = value.strip_prefix("bytes").unwrap_or(value).trim();
    let Some((range_part, total_part)) = spec.split_once('/') else {
        return false;
    };
    let Ok(total) = total_part.trim().parse::<u64>() else {
        return false;
    };
    let Some((start_raw, end_raw)) = range_part.trim().split_once('-') else {
        return false;
    };
    let (Ok(start), Ok(end)) = (
        start_raw.trim().parse::<u64>(),
        end_raw.trim().parse::<u64>(),
    ) else {
        return false;
    };
    total > 0 && start == 0 && end.saturating_add(1) == total
}

fn cached_stream_response(url: &str, range: Option<&str>, cache_state: &str, cache_id: Option<&str>) -> Option<StreamResponse> {
    let cached = cache::try_read(url, range, cache_id)?;
    let mut headers = cached.headers;
    headers.retain(|(name, _)| !name.eq_ignore_ascii_case("x-listen1-cache"));
    headers.push(("X-Listen1-Cache".into(), cache_state.into()));
    Some(StreamResponse {
        status: cached.status,
        headers,
        body: cached.body,
    })
}

fn should_cache_stream_response(status: u16, content_type: &str, bytes: &[u8]) -> bool {
    // 206 也可能是整段内容（哔哩哔哩必须转发 Range），是否整段由
    // content_range_covers_whole 判定，这里只排除明显不可缓存的状态码。
    if !matches!(status, 200 | 206) || bytes.len() < 1024 {
        return false;
    }

    let normalized = content_type.to_ascii_lowercase();
    if normalized.contains("text/")
        || normalized.contains("html")
        || normalized.contains("json")
        || normalized.contains("xml")
    {
        return false;
    }

    let prefix_len = bytes.len().min(128);
    let prefix = String::from_utf8_lossy(&bytes[..prefix_len]).to_ascii_lowercase();
    !prefix.contains("<html")
        && !prefix.contains("<!doctype")
        && !prefix.contains("\"code\"")
        && !prefix.contains("\"error\"")
}

fn local_file_stream_response(
    url: &str,
    range_header: Option<&str>,
) -> Result<StreamResponse, String> {
    let path = file_url_to_path(url);
    let mut file = match std::fs::File::open(&path) {
        Ok(file) => file,
        Err(_) => {
            return Ok(StreamResponse::empty(404));
        }
    };

    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    let content_type = local_content_type(&path);
    let range = range_header.and_then(|v| parse_range_header(v, len));

    let (status, start, end) = match range {
        Some((start, end)) => (206, start, end),
        None => (200, 0, len.saturating_sub(1)),
    };

    let read_len = if len == 0 { 0 } else { end - start + 1 };
    if read_len > 0 {
        file.seek(SeekFrom::Start(start))
            .map_err(|e| e.to_string())?;
    }

    let mut headers = vec![
        ("Content-Type".into(), content_type.into()),
        ("Accept-Ranges".into(), "bytes".into()),
        ("Content-Length".into(), read_len.to_string()),
        ("Access-Control-Allow-Origin".into(), "*".into()),
        (
            "Access-Control-Expose-Headers".into(),
            cache_expose_headers().into(),
        ),
        ("X-Listen1-Cache".into(), "local".into()),
    ];

    if status == 206 {
        headers.push((
            "Content-Range".into(),
            format!("bytes {}-{}/{}", start, end, len),
        ));
    }

    Ok(StreamResponse {
        status,
        headers,
        body: StreamBody::File {
            file,
            len: read_len,
        },
    })
}

fn tauri_response_from_stream_response(response: StreamResponse) -> tauri::http::Response<Vec<u8>> {
    let mut builder = tauri::http::Response::builder().status(response.status);
    for (name, value) in response.headers {
        builder = builder.header(name, value);
    }
    builder.body(response.body.into_bytes()).unwrap()
}

fn local_file_response(
    url: &str,
    request: &tauri::http::Request<Vec<u8>>,
) -> Result<tauri::http::Response<Vec<u8>>, String> {
    let range = request
        .headers()
        .get("range")
        .or_else(|| request.headers().get("Range"))
        .and_then(|v| v.to_str().ok());

    local_file_stream_response(url, range).map(tauri_response_from_stream_response)
}

fn handle_stream_connection(mut stream: TcpStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];

    loop {
        match stream.read(&mut chunk) {
            Ok(0) => return,
            Ok(n) => {
                buffer.extend_from_slice(&chunk[..n]);
                if buffer.windows(4).any(|w| w == b"\r\n\r\n") || buffer.len() > 64 * 1024 {
                    break;
                }
            }
            Err(_) => return,
        }
    }

    let request = String::from_utf8_lossy(&buffer);
    let mut lines = request.lines();
    let first_line = lines.next().unwrap_or("");
    let mut first_parts = first_line.split_whitespace();
    let method = first_parts.next().unwrap_or("");
    let target = first_parts.next().unwrap_or("");

    let mut headers = HashMap::new();
    for line in lines {
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }

    if method == "OPTIONS" {
        write_stream_response(&mut stream, StreamResponse::empty(204));
        return;
    }

    if method != "GET" && method != "HEAD" {
        write_stream_response(&mut stream, StreamResponse::empty(405));
        return;
    }

    let response = stream_response_for_request(target, headers.get("range").map(String::as_str))
        .unwrap_or_else(|_| StreamResponse::empty(500));

    if method == "HEAD" {
        write_stream_response(
            &mut stream,
            StreamResponse {
                body: StreamBody::Bytes(Vec::new()),
                ..response
            },
        );
    } else {
        write_stream_response(&mut stream, response);
    }
}

fn stream_response_for_request(
    target: &str,
    range: Option<&str>,
) -> Result<StreamResponse, String> {
    let path = target.split('?').next().unwrap_or(target);
    let query = target.split_once('?').map(|(_, q)| q).unwrap_or("");
    let no_cache_write = query
        .split('&')
        .any(|pair| matches!(pair, "no_cache_write=1" | "no_cache_write=true"));
    let cache_id = query
        .split('&')
        .find_map(|pair| pair.strip_prefix("cache_key="))
        .and_then(|value| urlencoding::decode(value).ok().map(|v| v.to_string()));
    let encoded = match path.strip_prefix("/stream/") {
        Some(value) if !value.is_empty() => value,
        _ => return Ok(StreamResponse::empty(404)),
    };

    let real_url = urlencoding::decode(encoded)
        .map(|value| value.to_string())
        .map_err(|e| e.to_string())?;

    if real_url.starts_with("file://") {
        return local_file_stream_response(&real_url, range);
    }

    remote_stream_response(&real_url, range, no_cache_write, cache_id.as_deref())
}

fn remote_stream_response(url: &str, range: Option<&str>, no_cache_write: bool, cache_id: Option<&str>) -> Result<StreamResponse, String> {
    if let Some(cached) = cached_stream_response(url, range, "hit", cache_id) {
        return Ok(cached);
    }

    let rt = shared_runtime();

    rt.block_on(async move {
        let client = build_client()?;
        let mut headers_map = HashMap::new();
        inject_headers(url, &mut headers_map);

        let fetch_full_for_cache = range_starts_at_zero(range) && should_fetch_full_for_cache(url);
        if let Some(range) = range.filter(|_| !fetch_full_for_cache) {
            headers_map.insert("Range".into(), range.to_string());
        }

        let mut header_map = header::HeaderMap::new();
        for (k, v) in &headers_map {
            if let (Ok(name), Ok(value)) = (
                header::HeaderName::from_bytes(k.as_bytes()),
                header::HeaderValue::from_str(v),
            ) {
                header_map.insert(name, value);
            }
        }

        let resp = client
            .get(url)
            .headers(header_map)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        let status = resp.status().as_u16();
        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_string();
        let content_range = resp
            .headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok())
            .map(|v| v.to_string());
        let bytes = resp.bytes().await.map_err(|e| e.to_string())?.to_vec();

        if !no_cache_write
            && content_range_covers_whole(content_range.as_deref())
            && should_cache_stream_response(status, &content_type, &bytes)
        {
            cache::write(url, &content_type, &bytes, cache_id);
            // 直接用已持有的字节构造响应，避免写盘后立刻重读整个文件。
            return Ok(full_body_stream_response(status, &content_type, content_range.as_deref(), bytes, "miss"));
        }

        Ok(full_body_stream_response(
            status,
            &content_type,
            content_range.as_deref(),
            bytes,
            "miss",
        ))
    })
}

fn full_body_stream_response(
    status: u16,
    content_type: &str,
    content_range: Option<&str>,
    body: Vec<u8>,
    cache_state: &str,
) -> StreamResponse {
    let mut headers = vec![
        ("Content-Type".into(), content_type.to_string()),
        ("Content-Length".into(), body.len().to_string()),
        ("Cache-Control".into(), "public, max-age=86400".into()),
        ("Access-Control-Allow-Origin".into(), "*".into()),
        (
            "Access-Control-Expose-Headers".into(),
            cache_expose_headers().into(),
        ),
        ("X-Listen1-Cache".into(), cache_state.into()),
    ];

    if let Some(content_range) = content_range {
        if !content_range.is_empty() {
            headers.push(("Content-Range".into(), content_range.to_string()));
            headers.push(("Accept-Ranges".into(), "bytes".into()));
        }
    }

    StreamResponse {
        status,
        headers,
        body: StreamBody::Bytes(body),
    }
}

fn status_text(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        206 => "Partial Content",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        _ => "OK",
    }
}

fn write_stream_response(stream: &mut TcpStream, mut response: StreamResponse) {
    let has_length = response
        .headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("content-length"));
    if !has_length {
        response
            .headers
            .push(("Content-Length".into(), response.body.len().to_string()));
    }
    response
        .headers
        .push(("Access-Control-Allow-Headers".into(), "Range".into()));
    response.headers.push(("Connection".into(), "close".into()));

    let mut head = format!(
        "HTTP/1.1 {} {}\r\n",
        response.status,
        status_text(response.status)
    );
    for (name, value) in response.headers {
        let safe_value = value.replace(['\r', '\n'], " ");
        head.push_str(&format!("{name}: {safe_value}\r\n"));
    }
    head.push_str("\r\n");

    if stream.write_all(head.as_bytes()).is_err() {
        return;
    }
    match response.body {
        StreamBody::Bytes(bytes) => {
            let _ = stream.write_all(&bytes);
        }
        // 固定 64KB 缓冲区搬运，内存占用与文件大小无关。播放器中途放弃（换歌、seek）
        // 时写入会失败，这里直接停下——原先是整段已经读进内存了，白读的部分省不掉。
        StreamBody::File { file, len } => {
            let mut reader = std::io::BufReader::with_capacity(64 * 1024, file.take(len));
            let _ = std::io::copy(&mut reader, stream);
        }
    }
    let _ = stream.flush();
}

#[tauri::command]
pub async fn http_request(options: HttpRequestOptions) -> Result<HttpResponse, String> {
    let client = build_client()?;

    let method = match options
        .method
        .as_deref()
        .unwrap_or("GET")
        .to_uppercase()
        .as_str()
    {
        "POST" => reqwest::Method::POST,
        "PUT" => reqwest::Method::PUT,
        "DELETE" => reqwest::Method::DELETE,
        "PATCH" => reqwest::Method::PATCH,
        _ => reqwest::Method::GET,
    };

    let mut req_headers = options.headers.unwrap_or_default();
    inject_headers(&options.url, &mut req_headers);

    let mut builder = client.request(method, &options.url);

    let mut header_map = header::HeaderMap::new();
    for (k, v) in &req_headers {
        if let (Ok(name), Ok(value)) = (
            header::HeaderName::from_bytes(k.as_bytes()),
            header::HeaderValue::from_str(v),
        ) {
            header_map.insert(name, value);
        }
    }
    builder = builder.headers(header_map);

    if let Some(body) = options.body {
        builder = builder.body(body);
    }

    let resp = builder.send().await.map_err(|e| e.to_string())?;
    let status = resp.status().as_u16();

    let mut resp_headers = HashMap::new();
    for (k, v) in resp.headers() {
        resp_headers.insert(k.to_string(), v.to_str().unwrap_or("").to_string());
    }

    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    let text = decode_text_response(&resp_headers, &bytes);
    let data = serde_json::from_str::<serde_json::Value>(&text)
        .unwrap_or(serde_json::Value::String(text.clone()));

    Ok(HttpResponse {
        status,
        headers: resp_headers,
        data,
        text: Some(text),
    })
}

#[tauri::command]
pub async fn audio_stream_url(url: String, no_cache_write: Option<bool>) -> Result<String, String> {
    let base_url = ensure_stream_server()?;
    let encoded = urlencoding::encode(&url);
    let suffix = if no_cache_write.unwrap_or(false) {
        "?no_cache_write=1"
    } else {
        ""
    };
    Ok(format!("{base_url}{encoded}{suffix}"))
}

/// Handles the stream custom protocol and proxies audio with correct headers.
pub async fn handle_stream_protocol(
    request: tauri::http::Request<Vec<u8>>,
) -> Result<tauri::http::Response<Vec<u8>>, String> {
    use tauri::http::Response;

    let uri = request.uri().to_string();
    let no_cache_write = request
        .uri()
        .query()
        .map(|query| {
            query
                .split('&')
                .any(|pair| matches!(pair, "no_cache_write=1" | "no_cache_write=true"))
        })
        .unwrap_or(false);
    let cache_id = request
        .uri()
        .query()
        .and_then(|query| {
            query
                .split('&')
                .find_map(|pair| pair.strip_prefix("cache_key="))
        })
        .and_then(|value| urlencoding::decode(value).ok().map(|v| v.to_string()));
    // URI format: http://stream.localhost/<encoded_url>
    let uri_without_query = uri.split('?').next().unwrap_or(&uri);
    let real_url = uri_without_query
        .strip_prefix("http://stream.localhost/")
        .or_else(|| uri_without_query.strip_prefix("stream://localhost/"))
        .unwrap_or("");

    let real_url = match urlencoding::decode(real_url) {
        Ok(u) => u.to_string(),
        Err(_) => {
            return Ok(Response::builder().status(400).body(Vec::new()).unwrap());
        }
    };

    if real_url.is_empty() {
        return Ok(Response::builder().status(400).body(Vec::new()).unwrap());
    }

    if real_url.starts_with("file://") {
        return local_file_response(&real_url, &request);
    }

    let range_header = request
        .headers()
        .get("range")
        .or_else(|| request.headers().get("Range"))
        .and_then(|value| value.to_str().ok());

    if let Some(cached) = cached_stream_response(&real_url, range_header, "hit", cache_id.as_deref()) {
        let mut builder = Response::builder().status(cached.status);
        for (name, value) in cached.headers {
            builder = builder.header(name, value);
        }
        return Ok(builder.body(cached.body.into_bytes()).unwrap());
    }

    let client = match build_client() {
        Ok(c) => c,
        Err(_) => {
            return Ok(Response::builder().status(500).body(Vec::new()).unwrap());
        }
    };

    let mut headers_map = HashMap::new();
    inject_headers(&real_url, &mut headers_map);

    let fetch_full_for_cache = range_starts_at_zero(range_header) && should_fetch_full_for_cache(&real_url);
    // Forward Range header for seeking support. For an initial bytes=0- miss, fetch the full
    // object once so the cache can serve later seeks.
    if let Some(range) = range_header.filter(|_| !fetch_full_for_cache) {
        headers_map.insert("Range".into(), range.to_string());
    }

    let mut header_map = header::HeaderMap::new();
    for (k, v) in &headers_map {
        if let (Ok(name), Ok(value)) = (
            header::HeaderName::from_bytes(k.as_bytes()),
            header::HeaderValue::from_str(v),
        ) {
            header_map.insert(name, value);
        }
    }

    let resp = match client.get(&real_url).headers(header_map).send().await {
        Ok(r) => r,
        Err(_) => {
            return Ok(Response::builder().status(502).body(Vec::new()).unwrap());
        }
    };

    let status = resp.status().as_u16();
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();
    let content_range = resp
        .headers()
        .get("content-range")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let bytes = resp.bytes().await.unwrap_or_default().to_vec();

    if !no_cache_write
        && content_range_covers_whole(Some(content_range.as_str()))
        && should_cache_stream_response(status, &content_type, &bytes)
    {
        cache::write(&real_url, &content_type, &bytes, cache_id.as_deref());
        // 直接用已持有的字节构造响应，避免写盘后立刻重读整个文件。
    }

    let response = full_body_stream_response(
        status,
        &content_type,
        Some(content_range.as_str()),
        bytes,
        "miss",
    );
    Ok(tauri_response_from_stream_response(response))
}

#[cfg(test)]
mod tests {
    use super::{
        content_range_covers_whole, ensure_stream_server, local_content_type,
        should_cache_stream_response, should_fetch_full_for_cache,
    };
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::path::PathBuf;

    /// 发一个裸 HTTP 请求给本机流服务器，返回 (状态码, 响应头, 正文)。
    /// 不用 reqwest：这里要验的正是自己手写的那套 HTTP 输出，用高层客户端会把
    /// Content-Length 与正文长度不一致之类的问题吞掉。
    fn raw_get(base: &str, encoded_target: &str, range: Option<&str>, method: &str) -> (u16, Vec<(String, String)>, Vec<u8>) {
        let addr = base
            .trim_start_matches("http://")
            .split('/')
            .next()
            .expect("base url has an authority");
        let mut stream = TcpStream::connect(addr).expect("connect to stream server");
        let mut request = format!(
            "{method} /stream/{encoded_target} HTTP/1.1\r\nHost: {addr}\r\n"
        );
        if let Some(range) = range {
            request.push_str(&format!("Range: {range}\r\n"));
        }
        request.push_str("Connection: close\r\n\r\n");
        stream.write_all(request.as_bytes()).expect("write request");
        stream.flush().expect("flush request");

        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).expect("read response");

        let split = raw
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .expect("response has a header terminator");
        let head = String::from_utf8_lossy(&raw[..split]).to_string();
        let body = raw[split + 4..].to_vec();

        let mut lines = head.lines();
        let status = lines
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|code| code.parse::<u16>().ok())
            .expect("response has a status code");
        let headers = lines
            .filter_map(|line| line.split_once(':'))
            .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_string()))
            .collect();
        (status, headers, body)
    }

    fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
        headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    /// 本机文件按范围流式输出：正文一个字节都不能错，Content-Length 必须与实际长度一致。
    ///
    /// 这条用例是为"正文改成不进内存、按 64KB 缓冲区直接搬到 socket"兜底的：文件故意
    /// 取 300KB，跨好几轮缓冲区，只测小文件的话一轮就搬完，缓冲区边界的错位看不出来。
    #[test]
    fn streams_local_file_bytes_and_ranges_exactly() {
        let base = ensure_stream_server().expect("stream server starts");

        let dir = std::env::temp_dir().join("aura-stream-body-test");
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("sample.mp3");
        // 伪随机但可复现的内容：全 0 或递增字节会让"错位一段"仍然巧合通过。
        let expected: Vec<u8> = (0..300_000u32)
            .map(|i| (i.wrapping_mul(2654435761) >> 13) as u8)
            .collect();
        std::fs::write(&path, &expected).expect("write temp file");

        let file_url = format!("file:///{}", path.to_string_lossy().replace('\\', "/"));
        let encoded = urlencoding::encode(&file_url).to_string();

        // 1) 不带 Range → 200 + 整个文件
        let (status, headers, body) = raw_get(base, &encoded, None, "GET");
        assert_eq!(status, 200);
        assert_eq!(
            header_value(&headers, "content-length"),
            Some(expected.len().to_string().as_str())
        );
        assert_eq!(header_value(&headers, "accept-ranges"), Some("bytes"));
        assert_eq!(body.len(), expected.len(), "整段长度");
        assert_eq!(body, expected, "整段内容");

        // 2) 中段 Range → 206 + 恰好那一段（跨越 64KB 缓冲区边界）
        let (start, end) = (1000usize, 200_047usize);
        let (status, headers, body) = raw_get(base, &encoded, Some("bytes=1000-200047"), "GET");
        assert_eq!(status, 206);
        assert_eq!(
            header_value(&headers, "content-range"),
            Some(format!("bytes {}-{}/{}", start, end, expected.len()).as_str())
        );
        assert_eq!(body, expected[start..=end], "中段内容");

        // 3) 后缀 Range（播放器探测尾部元数据时会用）
        let (status, _, body) = raw_get(base, &encoded, Some("bytes=-100"), "GET");
        assert_eq!(status, 206);
        assert_eq!(body, expected[expected.len() - 100..], "尾部 100 字节");

        // 4) HEAD → 正文为空，但 Content-Length 仍要报整段长度
        let (status, headers, body) = raw_get(base, &encoded, None, "HEAD");
        assert_eq!(status, 200);
        assert!(body.is_empty(), "HEAD 不该带正文");
        assert_eq!(
            header_value(&headers, "content-length"),
            Some(expected.len().to_string().as_str())
        );

        // 5) 起点越界 → 当作无 Range 处理，仍然给整段（parse_range_header 返回 None）
        let (status, _, body) = raw_get(base, &encoded, Some("bytes=999999999-"), "GET");
        assert_eq!(status, 200);
        assert_eq!(body.len(), expected.len());

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn missing_local_file_is_a_404() {
        let base = ensure_stream_server().expect("stream server starts");
        let missing = std::env::temp_dir().join("aura-stream-body-test-does-not-exist.mp3");
        let file_url = format!("file:///{}", missing.to_string_lossy().replace('\\', "/"));
        let (status, _, body) = raw_get(base, &urlencoding::encode(&file_url), None, "GET");
        assert_eq!(status, 404);
        assert!(body.is_empty());
    }

    #[test]
    fn serves_local_covers_as_images() {
        // 内嵌封面落盘后要经本机服务器取，Content-Type 必须是图片，
        // 否则 <img> 只能靠内容嗅探，行为随 webview 版本漂移。
        assert_eq!(local_content_type(&PathBuf::from("a/cover.JPG")), "image/jpeg");
        assert_eq!(local_content_type(&PathBuf::from("a/cover.png")), "image/png");
        assert_eq!(local_content_type(&PathBuf::from("a/cover.webp")), "image/webp");
        // 音频映射不能被改坏。
        assert_eq!(local_content_type(&PathBuf::from("a/song.flac")), "audio/flac");
        assert_eq!(local_content_type(&PathBuf::from("a/song.mp3")), "audio/mpeg");
    }

    #[test]
    fn no_content_range_is_a_whole_body() {
        assert!(content_range_covers_whole(None));
        assert!(content_range_covers_whole(Some("")));
        assert!(content_range_covers_whole(Some("   ")));
    }

    #[test]
    fn full_span_206_is_a_whole_body() {
        // 哔哩哔哩首个 bytes=0- 请求的典型回包。
        assert!(content_range_covers_whole(Some("bytes 0-5242879/5242880")));
        assert!(content_range_covers_whole(Some("  bytes  0-1023/1024  ")));
    }

    #[test]
    fn partial_span_is_not_a_whole_body() {
        assert!(!content_range_covers_whole(Some("bytes 0-1023/5242880")));
        assert!(!content_range_covers_whole(Some("bytes 1024-5242879/5242880")));
        assert!(!content_range_covers_whole(Some("bytes 0-1023/*")));
        assert!(!content_range_covers_whole(Some("bytes */5242880")));
        assert!(!content_range_covers_whole(Some("garbage")));
        assert!(!content_range_covers_whole(Some("bytes 0-0/0")));
    }

    #[test]
    fn partial_content_status_is_cacheable() {
        let body = vec![7u8; 2048];
        assert!(should_cache_stream_response(206, "audio/mp4", &body));
        assert!(should_cache_stream_response(200, "audio/mpeg", &body));
        assert!(!should_cache_stream_response(302, "audio/mpeg", &body));
        assert!(!should_cache_stream_response(200, "audio/mpeg", &body[..512]));
        assert!(!should_cache_stream_response(200, "application/json", &body));
    }

    #[test]
    fn bilibili_hosts_keep_forwarding_range() {
        assert!(!should_fetch_full_for_cache(
            "https://cn-hk-eq-01.bilivideo.com/upgcxcode/x.m4s?deadline=1"
        ));
        assert!(should_fetch_full_for_cache("https://m701.music.126.net/x.mp3"));
    }
}

