//! 前端调用的轻量模式接口。
//!
//! 这里只做三件事：把封面下到本地、把音频预热进磁盘缓存、切换到原生窗口。
//! 前两件是"切换前的准备"，第三件是不可逆的一步——WebView 一销毁，
//! provider 解析、歌词抓取、封面代理就全没了，所以准备必须先做完。

use crate::mini::{snapshot, MiniSnapshot};

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36";

/// 单曲预缓存上限。超过这个尺寸的多半不是歌，别把磁盘塞满。
const MAX_PRECACHE_BYTES: usize = 100 * 1024 * 1024;

/// 轻量模式是否可用。目前只有 Windows 有原生实现，其他平台前端应该隐藏入口。
#[tauri::command]
pub fn mini_supported() -> bool {
    cfg!(target_os = "windows")
}

/// 取回上次的快照，取完即删。原生窗口退出时会把最新的进度和队列写进去，
/// 完整模式重新起来之后靠这个接着播，不然那份进度写了也没人看。
///
/// 一次性语义是故意的：里面的进度只对"刚从轻量模式回来"这一次有效，
/// 留着的话下次冷启动会莫名其妙地跳回上次的位置。
#[tauri::command]
pub fn mini_load_snapshot() -> Option<MiniSnapshot> {
    let snapshot = snapshot::load();
    if snapshot.is_some() {
        snapshot::clear();
    }
    snapshot
}

/// 关掉原生窗口，回到完整模式。
///
/// 这个命令不是可有可无的：轻量模式下 `ExitRequested` 被无条件拦住了
/// （见 `lib.rs`），不给一条主动关窗的路，进程就再也退不掉。
#[tauri::command]
pub fn mini_exit() {
    crate::mini::request_close();
}

fn image_extension(url: &str) -> &'static str {
    let path = url.split(['?', '#']).next().unwrap_or(url).to_ascii_lowercase();
    if path.ends_with(".png") {
        "png"
    } else if path.ends_with(".webp") {
        "webp"
    } else if path.ends_with(".bmp") {
        "bmp"
    } else {
        // 绝大多数封面是 jpg，而且 WIC 按内容嗅探解码，扩展名猜错也能画出来。
        "jpg"
    }
}

/// 把封面下到 `~/Listen1/mini/covers/`，返回绝对路径。
///
/// 原生窗口没有 WebView 的图片加载能力，也没法走前端的封面代理，
/// 所以远端地址必须在切换前变成本地文件。同一张封面只下一次。
#[tauri::command]
pub async fn mini_fetch_cover(url: String) -> Result<Option<String>, String> {
    let url = url.trim().to_string();
    if url.is_empty() {
        return Ok(None);
    }

    // 已经是本地文件就直接用，不必绕一趟网络。
    if let Some(stripped) = url.strip_prefix("file:///").or_else(|| url.strip_prefix("file://")) {
        let decoded = urlencoding::decode(stripped).map(|s| s.into_owned()).unwrap_or_else(|_| stripped.to_string());
        return Ok(Some(decoded));
    }

    let dir = snapshot::covers_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let name = format!("{}.{}", crate::cache::cache_key_for(None, &url), image_extension(&url));
    let path = dir.join(&name);
    if path.exists() {
        return Ok(Some(path.to_string_lossy().to_string()));
    }

    let client = crate::proxy::build_client()?;
    let resp = client
        .get(&url)
        .header("user-agent", UA)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("封面下载失败：HTTP {}", resp.status().as_u16()));
    }
    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    if bytes.is_empty() {
        return Ok(None);
    }

    // 先写临时文件再改名：半个文件留在磁盘上会让原生侧解码失败且再也不会重下。
    let temp = path.with_extension("part");
    std::fs::write(&temp, &bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&temp, &path).map_err(|e| e.to_string())?;

    Ok(Some(path.to_string_lossy().to_string()))
}

/// 把整首歌拉进磁盘缓存，返回落盘路径。
///
/// 直链带时效签名，代码里没有任何地方会去续期。轻量模式下前端已经死了，
/// 签名一过期就再也解析不出新地址，所以这一步不是优化而是前提。
#[tauri::command]
pub async fn mini_precache_audio(cache_id: String, url: String) -> Result<Option<String>, String> {
    precache_audio(&cache_id, &url).await
}

/// 预缓存的实现本体，Rust 内部也要用。
///
/// 原生侧换成 rodio 之后只能播本地文件，播到预缓存窗口之外的曲目时要现下载一份，
/// 走的就是这里（见 `mini::resolve_playable_file`）。所以它不能只是个 tauri command。
pub(crate) async fn precache_audio(cache_id: &str, url: &str) -> Result<Option<String>, String> {
    let cache_id = cache_id.trim().to_string();
    if cache_id.is_empty() || cache_id.ends_with(':') {
        return Ok(None);
    }
    if let Some(hit) = crate::cache::audio_cache_lookup(cache_id.clone()) {
        return Ok(Some(hit));
    }

    let url = url.trim();
    if url.is_empty() {
        return Ok(None);
    }

    let client = crate::proxy::build_client()?;
    let resp = client
        .get(url)
        .header("user-agent", UA)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("预缓存失败：HTTP {}", resp.status().as_u16()));
    }

    if let Some(len) = resp.content_length() {
        if len as usize > MAX_PRECACHE_BYTES {
            return Ok(None);
        }
    }

    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("audio/mpeg")
        .to_string();

    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    if bytes.is_empty() || bytes.len() > MAX_PRECACHE_BYTES {
        return Ok(None);
    }

    crate::cache::write(url, &content_type, &bytes, Some(&cache_id));
    Ok(crate::cache::audio_cache_lookup(cache_id))
}

/// 进入轻量模式：先把快照落盘，再开原生窗口，最后才销毁 WebView。
///
/// 顺序是刻意的。原生窗口建不起来时要能原地放弃，用户还留在完整模式里；
/// 要是反过来先销毁窗口，一旦建窗失败就只剩托盘图标了。
#[tauri::command]
pub async fn mini_enter(app: tauri::AppHandle, snapshot: MiniSnapshot) -> Result<(), String> {
    crate::mini::clear_shutdown();
    let mut snapshot = snapshot;
    snapshot.normalize();
    snapshot::save(&snapshot)?;
    enter_impl(app, snapshot)
}

#[cfg(target_os = "windows")]
fn enter_impl(app: tauri::AppHandle, snapshot: MiniSnapshot) -> Result<(), String> {
    use tauri::Manager;

    let handle = app.clone();
    crate::mini::win::open(snapshot, move |latest| {
        // 原生窗口退出前把最新进度落盘，这样即使接下来建窗失败也不会丢位置。
        let _ = snapshot::save(&latest);
        let target = handle.clone();
        let _ = handle.run_on_main_thread(move || {
            // 托盘点「退出」走的也是关窗这条路（原生侧要先把进度写完），
            // 所以这里得分清是"回完整模式"还是"收尾退出"。
            if crate::mini::is_shutting_down() {
                target.exit(0);
                return;
            }
            if let Err(err) = restore_main_window(&target) {
                eprintln!("[listen1] 回到完整模式失败：{err}");
            }
            // 回到完整模式了，托盘那条改回「切换轻量模式」。
            crate::sync_lite_menu_label(&target, false);
        });
    })?;

    // 原生窗口确实活着了，托盘条目改成「退出轻量模式」。
    // 放在 win::open 之后：建窗失败时不能改文案，否则托盘会谎报状态。
    crate::sync_lite_menu_label(&app, true);

    // 原生窗口确实活着了，这才销毁 WebView——轻量模式的"0 webui"就是在这一步实现的。
    if let Some(main) = app.get_webview_window("main") {
        main.destroy().map_err(|e| e.to_string())?;
    }
    // 悬浮歌词也是一个 WebView，留着它等于白省：WebView2 的浏览器 / 渲染器 / GPU /
    // 网络服务那一组进程只要还有一个 WebView 活着就全都在，实测就是两百多 MB。
    //
    // 这里必须 destroy——close_float_window 只是 hide，窗口不可见但 WebView 还在，
    // 进程一个都不会退，轻量模式的内存优势就全没了。回完整模式之后 App.svelte 的
    // onMount 会按 enableLyricFloatingWindow 重新把它建出来。
    if let Some(float) = app.get_webview_window("float") {
        // 位置只活在窗口自身（拖动是直接 set_position 的，settings 里没有 x/y），
        // 销毁前先记下来，重建时贴回去，不然回完整模式歌词条就跑回默认位置了。
        crate::window::remember_float_position(&app);
        let _ = float.destroy();
    }
    Ok(())
}

/// 重建 main 窗口。直接用 tauri.conf.json 里的原始配置，
/// 免得手写一份尺寸/装饰参数，日后配置改了这边忘了跟。
#[cfg(target_os = "windows")]
fn restore_main_window(app: &tauri::AppHandle) -> Result<(), String> {
    use tauri::Manager;

    if let Some(existing) = app.get_webview_window("main") {
        let _ = existing.show();
        let _ = existing.unminimize();
        let _ = existing.set_focus();
        return Ok(());
    }

    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|w| w.label == "main")
        .cloned()
        .ok_or_else(|| "tauri.conf.json 里找不到 main 窗口配置".to_string())?;

    let window = tauri::WebviewWindowBuilder::from_config(app, &config)
        .map_err(|e| e.to_string())?
        .build()
        .map_err(|e| e.to_string())?;
    let _ = window.show();
    let _ = window.set_focus();
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn enter_impl(_app: tauri::AppHandle, _snapshot: MiniSnapshot) -> Result<(), String> {
    Err("轻量模式目前只在 Windows 上有原生实现".to_string())
}
