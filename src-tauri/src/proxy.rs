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

#[derive(Debug)]
pub(crate) struct StreamResponse {
    pub(crate) status: u16,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
}

impl StreamResponse {
    fn empty(status: u16) -> Self {
        Self {
            status,
            headers: vec![("Access-Control-Allow-Origin".into(), "*".into())],
            body: Vec::new(),
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

fn audio_content_type(path: &PathBuf) -> &'static str {
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
    if status != 200 || bytes.len() < 1024 {
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
    let content_type = audio_content_type(&path);
    let range = range_header.and_then(|v| parse_range_header(v, len));

    let (status, start, end) = match range {
        Some((start, end)) => (206, start, end),
        None => (200, 0, len.saturating_sub(1)),
    };

    let read_len = if len == 0 { 0 } else { end - start + 1 };
    let mut bytes = vec![0; read_len as usize];
    if read_len > 0 {
        file.seek(SeekFrom::Start(start))
            .map_err(|e| e.to_string())?;
        file.read_exact(&mut bytes).map_err(|e| e.to_string())?;
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
        body: bytes,
    })
}

fn tauri_response_from_stream_response(response: StreamResponse) -> tauri::http::Response<Vec<u8>> {
    let mut builder = tauri::http::Response::builder().status(response.status);
    for (name, value) in response.headers {
        builder = builder.header(name, value);
    }
    builder.body(response.body).unwrap()
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
                body: Vec::new(),
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

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;

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

        if !no_cache_write && content_range.is_none() && should_cache_stream_response(status, &content_type, &bytes) {
            cache::write(url, &content_type, &bytes, cache_id);
            if let Some(cached) = cached_stream_response(url, range, "write", cache_id) {
                return Ok(cached);
            }
        }

        let mut headers = vec![
            ("Content-Type".into(), content_type),
            ("Content-Length".into(), bytes.len().to_string()),
            ("Cache-Control".into(), "public, max-age=86400".into()),
            ("Access-Control-Allow-Origin".into(), "*".into()),
            (
                "Access-Control-Expose-Headers".into(),
                cache_expose_headers().into(),
            ),
            ("X-Listen1-Cache".into(), "miss".into()),
        ];

        if let Some(content_range) = content_range {
            headers.push(("Content-Range".into(), content_range));
            headers.push(("Accept-Ranges".into(), "bytes".into()));
        }

        Ok(StreamResponse {
            status,
            headers,
            body: bytes,
        })
    })
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

    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&response.body);
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
        return Ok(builder.body(cached.body).unwrap());
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
    let content_length = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let content_range = resp
        .headers()
        .get("content-range")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let bytes = resp.bytes().await.unwrap_or_default().to_vec();

    if !no_cache_write && content_range.is_empty() && should_cache_stream_response(status, &content_type, &bytes) {
        cache::write(&real_url, &content_type, &bytes, cache_id.as_deref());
        if let Some(cached) = cached_stream_response(&real_url, range_header, "write", cache_id.as_deref()) {
            let mut builder = Response::builder().status(cached.status);
            for (name, value) in cached.headers {
                builder = builder.header(name, value);
            }
            return Ok(builder.body(cached.body).unwrap());
        }
    }

    let mut builder = Response::builder()
        .status(status)
        .header("Content-Type", &content_type)
        .header("Cache-Control", "public, max-age=86400")
        .header("Access-Control-Allow-Origin", "*")
        .header("Access-Control-Expose-Headers", cache_expose_headers())
        .header("X-Listen1-Cache", "miss");

    if !content_length.is_empty() {
        builder = builder.header("Content-Length", &content_length);
    }
    if !content_range.is_empty() {
        builder = builder.header("Content-Range", &content_range);
        builder = builder.header("Accept-Ranges", "bytes");
    }

    Ok(builder.body(bytes).unwrap())
}
