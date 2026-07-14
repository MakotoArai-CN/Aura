#[cfg(desktop)]
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
}

impl Default for FloatingLyricSettingsSnapshot {
    fn default() -> Self {
        Self {
            enabled: false,
            hide_when_main_visible: true,
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

    FloatingLyricSettingsSnapshot {
        enabled,
        hide_when_main_visible,
    }
}


/// 浮窗始终接收鼠标事件（ignore=false）。
/// 锁定穿透由前端用透明像素 + pointer-events 实现：
/// Tauri 没有 Electron 的 setIgnoreMouseEvents({ forward: true })，
/// 一旦 set_ignore_cursor_events(true) 就无法 hover/点击解锁。
fn apply_float_ignore_from_settings(
    app: &AppHandle,
    _state: &tauri::State<'_, FloatingLyricSettingsState>,
) {
    if let Some(float) = app.get_webview_window("float") {
        let _ = float.set_ignore_cursor_events(false);
        let _ = float.set_always_on_top(true);
    }
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
    eprintln!("[listen1] open_login_window {url}");

    let login = app
        .get_webview_window("login")
        .ok_or_else(|| "login window not found".to_string())?;
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
pub fn window_minimize(window: WebviewWindow) {
    #[cfg(debug_assertions)]
    eprintln!("[listen1] window_minimize");
    let app = window.app_handle().clone();
    let is_main = window.label() == "main";
    let _ = window.minimize();
    #[cfg(desktop)]
    if is_main {
        sync_float_visibility_for_main(&app, false);
    }
}

#[tauri::command]
pub fn window_maximize(window: WebviewWindow) {
    #[cfg(debug_assertions)]
    eprintln!("[listen1] window_maximize");
    if window.is_maximized().unwrap_or(false) {
        let _ = window.unmaximize();
    } else {
        let _ = window.maximize();
    }
}

#[tauri::command]
pub fn window_close(window: WebviewWindow) {
    #[cfg(debug_assertions)]
    eprintln!("[listen1] window_close");
    let app = window.app_handle().clone();
    let is_main = window.label() == "main";
    let _ = window.hide();
    #[cfg(desktop)]
    if is_main {
        sync_float_visibility_for_main(&app, false);
    }
}

#[tauri::command]
pub fn window_quit(app: AppHandle) {
    #[cfg(debug_assertions)]
    eprintln!("[listen1] window_quit");
    app.exit(0);
}

#[tauri::command]
#[cfg(desktop)]
pub fn show_float_window(
    app: AppHandle,
    settings_state: tauri::State<'_, FloatingLyricSettingsState>,
) -> Result<(), String> {
    #[cfg(debug_assertions)]
    eprintln!("[listen1] show_float_window");
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
    if app.get_webview_window("float").is_none() {
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
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(true)
        .build()
        .map_err(|e| e.to_string())?;
    }
    if let Some(float) = app.get_webview_window("float") {
        let _ = float.set_always_on_top(true);
        let _ = float.set_shadow(false);
        float.show().map_err(|e| e.to_string())?;
        apply_float_ignore_from_settings(&app, &settings_state);
        #[cfg(debug_assertions)]
        {
            match float.navigate(dev_float_url.clone()) {
                Ok(()) => eprintln!("[listen1] float navigate requested {dev_float_url}"),
                Err(error) => eprintln!("[listen1] float navigate failed: {error}"),
            }
            match float.url() {
                Ok(url) => eprintln!("[listen1] float url={url}"),
                Err(error) => eprintln!("[listen1] float url read failed: {error}"),
            }
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
    eprintln!("[listen1] close_float_window");
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
