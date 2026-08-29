use tauri::Manager;
use std::sync::Mutex;
use tauri::Emitter;
use tauri::{AppHandle, WebviewWindow};
use std::collections::HashMap;

#[derive(Default)]
pub struct FloatingLyricState {
    payload: Mutex<Option<String>>,
}

#[derive(Default)]
pub struct FloatingLyricSettingsState {
    payload: Mutex<Option<String>>,
}

struct FloatingLyricSettingsSnapshot {
    enabled: bool,
    hide_when_main_visible: bool,
    /// 锁定状态。锁上之后鼠标事件要穿透到底下的窗口去。
    locked: bool,
}

impl Default for FloatingLyricSettingsSnapshot {
    fn default() -> Self {
        Self {
            enabled: false,
            hide_when_main_visible: true,
            locked: false,
        }
    }
}

fn floating_settings_snapshot(payload: &str) -> FloatingLyricSettingsSnapshot {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(payload) else {
        return FloatingLyricSettingsSnapshot::default();
    };
    let enabled = value
        .get("enableLyricFloatingWindow")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let hide_when_main_visible = value
        .get("hideLyricFloatingWindowWhenMainVisible")
        .and_then(|value| value.as_bool())
        .unwrap_or(true);
    let locked = value
        .get("lyricWindow")
        .and_then(|window| window.get("locked"))
        .and_then(|value| value.as_bool())
        .unwrap_or(false);

    FloatingLyricSettingsSnapshot {
        enabled,
        hide_when_main_visible,
        locked,
    }
}


/// 悬浮歌词窗口被销毁前记下的屏幕位置。
///
/// 位置只存在于窗口自身：拖动时前端是直接 `move_float_window` 调 `set_position`，
/// settings 里没有 x/y。所以 destroy 一次位置就没了。轻量模式必须 destroy 它才能真的
/// 放掉 WebView2 那一组进程（见 `mini::commands::enter_impl`），于是销毁前把位置存到
/// 这里，下次建窗时贴回去，回完整模式后歌词条还在原地。
///
/// 只在进程内有效，本来也只需要撑过「销毁 → 重建」这一个来回。
static FLOAT_POSITION: std::sync::Mutex<Option<(i32, i32)>> = std::sync::Mutex::new(None);

/// 在销毁悬浮歌词窗口之前调用，记下它当前的位置。
#[cfg(desktop)]
pub fn remember_float_position(app: &AppHandle) {
    if let Some(float) = app.get_webview_window("float") {
        if let Ok(pos) = float.outer_position() {
            if let Ok(mut slot) = FLOAT_POSITION.lock() {
                *slot = Some((pos.x, pos.y));
            }
        }
    }
}

/// 锁定时让鼠标事件穿透到底下的窗口去。
///
/// 原来这里是无条件 `ignore=false`，理由是 Tauri 没有 Electron 那个
/// `setIgnoreMouseEvents({ forward: true })`——一旦穿透，浮窗自己就再也收不到 hover，
/// 没法点回来解锁。但锁定状态下工具栏本来就是整体隐藏的（见 FloatingLyric.svelte 的
/// `toolbarVisible = !locked && showToolbar`），窗口上没有任何可交互的东西，不穿透
/// 纯粹是白挡着底下的窗口。解锁的出口在主窗设置里（设置 → 桌面歌词 → 锁定），
/// 不依赖点浮窗本身，所以这里可以放心穿透。
///
/// Windows 上 `set_ignore_cursor_events` 会顺手清掉 `WS_EX_TOPMOST`，所以之后必须
/// 重新置顶，否则锁定一次浮窗就沉到别的窗口后面去了。
#[cfg(desktop)]
fn apply_float_ignore_from_settings(
    app: &AppHandle,
    state: &tauri::State<'_, FloatingLyricSettingsState>,
) {
    let locked = state
        .payload
        .lock()
        .ok()
        .and_then(|payload| payload.clone())
        .map(|payload| floating_settings_snapshot(&payload).locked)
        .unwrap_or(false);
    if let Some(float) = app.get_webview_window("float") {
        let _ = float.set_ignore_cursor_events(locked);
        let _ = float.set_always_on_top(true);
    }
}

#[cfg(not(desktop))]
fn apply_float_ignore_from_settings(
    _app: &AppHandle,
    _state: &tauri::State<'_, FloatingLyricSettingsState>,
) {
}

#[cfg(desktop)]
pub fn sync_float_visibility_for_main(app: &AppHandle, main_visible: bool) {
    let settings_state = app.state::<FloatingLyricSettingsState>();
    let settings = settings_state
        .payload
        .lock()
        .ok()
        .and_then(|payload| payload.clone())
        .map(|payload| floating_settings_snapshot(&payload))
        .unwrap_or_default();

    if !settings.enabled || (settings.hide_when_main_visible && main_visible) {
        if let Some(float) = app.get_webview_window("float") {
            let _ = float.hide();
        }
        return;
    }

    let _ = show_float_window(app.clone(), settings_state);
}

#[tauri::command]
pub fn set_floating_lyric_payload(
    app: AppHandle,
    state: tauri::State<'_, FloatingLyricState>,
    payload: String,
) -> Result<(), String> {
    *state.payload.lock().map_err(|e| e.to_string())? = Some(payload.clone());
    // 唯一歌词通道：eval 注入浮窗（listen1-native-lyric-update）。高频，避开 Tauri emit 序列化开销。
    if let Some(float) = app.get_webview_window("float") {
        if let Ok(payload_json) = serde_json::to_string(&payload) {
            let script = format!(
                "window.dispatchEvent(new CustomEvent('listen1-native-lyric-update', {{ detail: {payload_json} }}));"
            );
            let _ = float.eval(&script);
        }
    }
    Ok(())
}

#[tauri::command]
pub fn get_floating_lyric_payload(
    state: tauri::State<'_, FloatingLyricState>,
) -> Result<Option<String>, String> {
    Ok(state.payload.lock().map_err(|e| e.to_string())?.clone())
}

#[tauri::command]
pub fn set_floating_lyric_settings(
    app: AppHandle,
    state: tauri::State<'_, FloatingLyricSettingsState>,
    payload: String,
) -> Result<(), String> {
    *state.payload.lock().map_err(|e| e.to_string())? = Some(payload.clone());
    // 唯一设置通道：Tauri emit（低频）。浮窗端 listen("lyric-settings-update") 接收。
    let _ = app.emit_to("float", "lyric-settings-update", payload.clone());
    // 确保浮窗可交互（锁定穿透由前端透明像素处理）。
    apply_float_ignore_from_settings(&app, &state);
    Ok(())
}

#[tauri::command]
pub fn get_floating_lyric_settings(
    state: tauri::State<'_, FloatingLyricSettingsState>,
) -> Result<Option<String>, String> {
    Ok(state.payload.lock().map_err(|e| e.to_string())?.clone())
}

#[tauri::command]
#[cfg(desktop)]
pub fn open_login_window(app: AppHandle, url: String) -> Result<(), String> {
    let parsed = tauri::Url::parse(&url).map_err(|e| e.to_string())?;
    #[cfg(debug_assertions)]
    eprintln!("[aura] open_login_window {url}");

    // 惰性创建登录窗口：平时不驻留 WebView 进程，节省内存。
    let login = match app.get_webview_window("login") {
        Some(login) => login,
        None => {
            #[cfg(debug_assertions)]
            let login_url = tauri::WebviewUrl::External(
                app.config()
                    .build
                    .dev_url
                    .clone()
                    .unwrap_or_else(|| tauri::Url::parse("http://localhost:1420").unwrap())
                    .join("login.html")
                    .map_err(|e| e.to_string())?,
            );
            #[cfg(not(debug_assertions))]
            let login_url = tauri::WebviewUrl::App("login.html".into());
            tauri::WebviewWindowBuilder::new(&app, "login", login_url)
                .title("Aura 登录")
                .inner_size(985.0, 700.0)
                .min_inner_size(760.0, 560.0)
                .resizable(true)
                .maximizable(true)
                .minimizable(true)
                .center()
                .visible(false)
                .build()
                .map_err(|e| e.to_string())?
        }
    };
    let mut login_url = app
        .get_webview_window("main")
        .and_then(|main| main.url().ok())
        .unwrap_or_else(|| tauri::Url::parse("tauri://localhost/").unwrap());
    login_url.set_path("login.html");
    login_url.set_query(Some(&format!("target={}", urlencoding::encode(parsed.as_str()))));
    login_url.set_fragment(None);
    login.navigate(login_url).map_err(|e| e.to_string())?;
    login.show().map_err(|e| e.to_string())?;
    login.set_focus().map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
#[cfg(not(desktop))]
pub fn open_login_window(_app: AppHandle, _url: String) -> Result<(), String> {
    Ok(())
}

#[tauri::command]
#[cfg(desktop)]
pub fn close_login_window(app: AppHandle) {
    if let Some(login) = app.get_webview_window("login") {
        let _ = login.hide();
    }
}

#[tauri::command]
#[cfg(not(desktop))]
pub fn close_login_window(_app: AppHandle) {}

#[tauri::command]
#[cfg(desktop)]
pub async fn sync_login_cookies(app: AppHandle, urls: Vec<String>) -> Result<usize, String> {
    let Some(login) = app.get_webview_window("login").or_else(|| app.get_webview_window("main")) else {
        return Ok(0);
    };

    let mut count = 0usize;
    for url in urls {
        let parsed = match tauri::Url::parse(&url) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let cookies = login.cookies_for_url(parsed.clone()).map_err(|e| e.to_string())?;
        for cookie in cookies {
            let mut value = format!("{}={}", cookie.name(), cookie.value());
            if let Some(path) = cookie.path() {
                value.push_str(&format!("; Path={path}"));
            }
            if let Some(domain) = cookie.domain() {
                value.push_str(&format!("; Domain={domain}"));
            }
            if cookie.secure().unwrap_or(false) {
                value.push_str("; Secure");
            }
            let _ = crate::proxy::add_cookie_for_url(parsed.as_str(), &value);
            count += 1;
        }
    }
    Ok(count)
}

#[tauri::command]
#[cfg(not(desktop))]
pub async fn sync_login_cookies(_app: AppHandle, _urls: Vec<String>) -> Result<usize, String> {
    Ok(0)
}

#[tauri::command]
#[cfg(desktop)]
pub async fn get_login_cookies(app: AppHandle, url: String) -> Result<HashMap<String, String>, String> {
    let Some(login) = app.get_webview_window("login").or_else(|| app.get_webview_window("main")) else {
        return Ok(HashMap::new());
    };
    let parsed = tauri::Url::parse(&url).map_err(|e| e.to_string())?;
    let cookies = login.cookies_for_url(parsed).map_err(|e| e.to_string())?;
    Ok(cookies
        .into_iter()
        .map(|cookie| (cookie.name().to_string(), cookie.value().to_string()))
        .collect())
}

#[tauri::command]
#[cfg(not(desktop))]
pub async fn get_login_cookies(_app: AppHandle, _url: String) -> Result<HashMap<String, String>, String> {
    Ok(HashMap::new())
}

#[tauri::command]
#[cfg(desktop)]
pub async fn clear_login_cookies(app: AppHandle, url: String, names: Vec<String>) -> Result<(), String> {
    let parsed = tauri::Url::parse(&url).map_err(|e| e.to_string())?;
    if let Some(login) = app.get_webview_window("login").or_else(|| app.get_webview_window("main")) {
        let cookies = login.cookies_for_url(parsed.clone()).map_err(|e| e.to_string())?;
        for name in &names {
            for cookie in cookies.iter().filter(|cookie| cookie.name() == name) {
                let _ = login.delete_cookie(cookie.clone());
            }
            let _ = crate::proxy::clear_cookie_for_url(parsed.as_str(), name);
        }
    } else {
        for name in &names {
            let _ = crate::proxy::clear_cookie_for_url(parsed.as_str(), name);
        }
    }
    Ok(())
}

#[tauri::command]
#[cfg(not(desktop))]
pub async fn clear_login_cookies(_app: AppHandle, _url: String, _names: Vec<String>) -> Result<(), String> {
    Ok(())
}

#[tauri::command]
#[cfg(desktop)]
pub fn window_minimize(window: WebviewWindow) {
    #[cfg(debug_assertions)]
    eprintln!("[aura] window_minimize");
    let app = window.app_handle().clone();
    let is_main = window.label() == "main";
    let _ = window.minimize();
    if is_main {
        sync_float_visibility_for_main(&app, false);
    }
}

#[tauri::command]
#[cfg(not(desktop))]
pub fn window_minimize(_window: WebviewWindow) {}

#[tauri::command]
#[cfg(desktop)]
pub fn window_maximize(window: WebviewWindow) {
    #[cfg(debug_assertions)]
    eprintln!("[aura] window_maximize");
    if window.is_maximized().unwrap_or(false) {
        let _ = window.unmaximize();
    } else {
        let _ = window.maximize();
    }
}

#[tauri::command]
#[cfg(not(desktop))]
pub fn window_maximize(_window: WebviewWindow) {}

#[tauri::command]
#[cfg(desktop)]
pub fn window_close(window: WebviewWindow) {
    #[cfg(debug_assertions)]
    eprintln!("[aura] window_close");
    let app = window.app_handle().clone();
    let is_main = window.label() == "main";
    let _ = window.hide();
    if is_main {
        sync_float_visibility_for_main(&app, false);
    }
}

#[tauri::command]
#[cfg(not(desktop))]
pub fn window_close(_window: WebviewWindow) {}

#[tauri::command]
pub fn window_quit(app: AppHandle) {
    #[cfg(debug_assertions)]
    eprintln!("[aura] window_quit");
    app.exit(0);
}

#[tauri::command]
#[cfg(desktop)]
pub fn show_float_window(
    app: AppHandle,
    settings_state: tauri::State<'_, FloatingLyricSettingsState>,
) -> Result<(), String> {
    #[cfg(debug_assertions)]
    eprintln!("[aura] show_float_window");
    #[cfg(debug_assertions)]
    let dev_float_url = app
        .config()
        .build
        .dev_url
        .as_ref()
        .cloned()
        .unwrap_or_else(|| tauri::Url::parse("http://localhost:1420").unwrap())
        .join("float.html")
        .map_err(|e| e.to_string())?;

    // Create float window if it doesn't exist yet
    let just_created = app.get_webview_window("float").is_none();
    if just_created {
        #[cfg(debug_assertions)]
        let float_url = tauri::WebviewUrl::External(dev_float_url.clone());
        #[cfg(not(debug_assertions))]
        let float_url = tauri::WebviewUrl::App("float.html".into());

        let _float = tauri::WebviewWindowBuilder::new(
            &app,
            "float",
            float_url,
        )
        .title("Aura Lyrics")
        .inner_size(1000.0, 112.0)
        .min_inner_size(640.0, 72.0)
        .max_inner_size(1920.0, 220.0)
        .decorations(false)
        .shadow(false)
        // 必须开透明：锁定状态下前端会把 --float-bg-alpha 设成 0 来实现「只剩歌词」，
        // 窗口要是不透明，透出来的就是 WebView 默认的白底，而歌词文字本身是白的，
        // 白底白字，看起来就是整个窗口全白、什么都没有。
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(true)
        .build()
        .map_err(|e| e.to_string())?;
    }
    if let Some(float) = app.get_webview_window("float") {
        let _ = float.set_always_on_top(true);
        let _ = float.set_shadow(false);
        // 刚建出来的窗口贴回销毁前记下的位置，否则会回到默认位置——轻量模式来回一趟
        // 歌词条就跑了。没有记录（本次启动第一次开）就保持默认。
        if just_created {
            if let Some((x, y)) = FLOAT_POSITION.lock().ok().and_then(|slot| *slot) {
                let _ = float.set_position(tauri::PhysicalPosition::new(x, y));
            }
        }
        float.show().map_err(|e| e.to_string())?;
        apply_float_ignore_from_settings(&app, &settings_state);
        // 只有复用已经存在的窗口时才重新导航——dev server 重启过的话它可能还停在
        // 一个已经没人监听的地址上。刚 build 出来的窗口正在加载同一个地址，这时候再
        // navigate 一次会把那次导航打断，WebView2 可能就停在 about:blank 上不动了。
        #[cfg(debug_assertions)]
        if !just_created {
            match float.navigate(dev_float_url.clone()) {
                Ok(()) => eprintln!("[aura] float navigate requested {dev_float_url}"),
                Err(error) => eprintln!("[aura] float navigate failed: {error}"),
            }
        }
        #[cfg(debug_assertions)]
        match float.url() {
            Ok(url) => eprintln!("[aura] float url={url} just_created={just_created}"),
            Err(error) => eprintln!("[aura] float url read failed: {error}"),
        }
    }
    Ok(())
}

#[tauri::command]
#[cfg(not(desktop))]
pub fn show_float_window(
    _app: AppHandle,
    _settings_state: tauri::State<'_, FloatingLyricSettingsState>,
) -> Result<(), String> {
    Ok(())
}

#[tauri::command]
#[cfg(desktop)]
pub fn hide_float_window(app: AppHandle) {
    if let Some(float) = app.get_webview_window("float") {
        let _ = float.hide();
    }
}

#[tauri::command]
#[cfg(not(desktop))]
pub fn hide_float_window(_app: AppHandle) {}

#[tauri::command]
#[cfg(desktop)]
pub fn close_float_window(app: AppHandle) -> Result<(), String> {
    #[cfg(debug_assertions)]
    eprintln!("[aura] close_float_window");
    if let Some(float) = app.get_webview_window("float") {
        let _ = float.set_ignore_cursor_events(false);
        float.hide().map_err(|e| e.to_string())?;
    }
    let close_id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| format!("native-close-{}", duration.as_millis()))
        .unwrap_or_else(|_| "native-close".to_string());
    let payload = serde_json::json!({ "id": close_id });
    let _ = app.emit_to("main", "float-lyric-closed", payload.clone());
    let _ = app.emit("float-lyric-closed", payload);
    Ok(())
}

#[tauri::command]
#[cfg(not(desktop))]
pub fn close_float_window(_app: AppHandle) -> Result<(), String> {
    Ok(())
}

#[tauri::command]
#[cfg(desktop)]
pub fn set_float_window_ignore_mouse(app: AppHandle, ignore: bool) {
    if let Some(float) = app.get_webview_window("float") {
        let _ = float.set_ignore_cursor_events(ignore);
        // Windows: set_ignore_cursor_events 会导致 WS_EX_TOPMOST 丢失，需重新置顶
        let _ = float.set_always_on_top(true);
    }
}

#[tauri::command]
#[cfg(not(desktop))]
pub fn set_float_window_ignore_mouse(_app: AppHandle, _ignore: bool) {}

#[tauri::command]
#[cfg(desktop)]
pub fn set_float_window_height(app: AppHandle, height: f64) {
    if let Some(float) = app.get_webview_window("float") {
        let bounded = height.clamp(72.0, 220.0);
        let width = float
            .inner_size()
            .map(|size| size.width as f64 / float.scale_factor().unwrap_or(1.0))
            .unwrap_or(1000.0)
            .clamp(640.0, 1920.0);
        let _ = float.set_size(tauri::Size::Logical(tauri::LogicalSize::new(width, bounded)));
    }
}

#[tauri::command]
#[cfg(not(desktop))]
pub fn set_float_window_height(_app: AppHandle, _height: f64) {}

#[tauri::command]
#[cfg(desktop)]
pub fn move_float_window(app: AppHandle, x: i32, y: i32) {
    if let Some(float) = app.get_webview_window("float") {
        let _ = float.set_position(tauri::PhysicalPosition::new(x, y));
    }
}

#[tauri::command]
#[cfg(not(desktop))]
pub fn move_float_window(_app: AppHandle, _x: i32, _y: i32) {}

#[tauri::command]
#[cfg(desktop)]
pub fn get_float_window_position(app: AppHandle) -> Result<(i32, i32), String> {
    if let Some(float) = app.get_webview_window("float") {
        let pos = float.outer_position().map_err(|e| e.to_string())?;
        Ok((pos.x, pos.y))
    } else {
        Err("float window not found".to_string())
    }
}

#[tauri::command]
#[cfg(not(desktop))]
pub fn get_float_window_position(_app: AppHandle) -> Result<(i32, i32), String> {
    Ok((0, 0))
}
