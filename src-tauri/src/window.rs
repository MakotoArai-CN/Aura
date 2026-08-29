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

/// 主窗口"看不见了"的去重状态。false = 前端当前认为自己可见。
#[cfg(desktop)]
static MAIN_HIDDEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// main 的 WebView 重建之后调用（进出一次轻量模式就是一次重建）。
///
/// 新页面里 `windowMinimized` store 的初值是 false，如果这里不拨回同一个起点，
/// 下一次真的隐藏会被 `swap` 判成"状态没变"而不发事件，前端就永远停不下动画。
#[cfg(desktop)]
pub fn reset_main_visibility_state() {
    MAIN_HIDDEN.store(false, std::sync::atomic::Ordering::Relaxed);
}

/// 主窗口可见性的唯一汇聚点：同步桌面歌词显隐，并告诉前端「现在没人看得见你」，
/// 让它停掉持续动画和效果看门狗。
///
/// 判据是"看不见"而不只是"最小化"：关闭到托盘走的是 `hide()`，那一刻
/// `is_minimized()` 仍然是 false，但 WebView2 在无边框透明窗口上不做遮挡检测，
/// 隐藏 HWND 背后的动画照旧合成——正是最小化那套优化要消灭的同一份浪费。
/// 事件名沿用 `main-minimized`（前端 store 叫 windowMinimized），语义按"不可见"读。
///
/// 状态真的翻转时才发：`Resized` 拖一下窗口就来几十条，每帧一条事件的开销比省下的
/// 绘制还大。
#[cfg(desktop)]
pub fn sync_float_visibility_for_main(app: &AppHandle, main_visible: bool) {
    let hidden = !main_visible;
    if MAIN_HIDDEN.swap(hidden, std::sync::atomic::Ordering::Relaxed) != hidden {
        let _ = app.emit_to("main", "main-minimized", hidden);
    }

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
    // 真全屏状态下"最大化"没有任何可见效果（窗口已经铺满整屏），点了像坏了。
    // 这时把它当成"退出全屏"，回到普通的最大化姿态。
    if window.is_fullscreen().unwrap_or(false) {
        let _ = window.set_fullscreen(false);
        return;
    }
    if window.is_maximized().unwrap_or(false) {
        let _ = window.unmaximize();
    } else {
        let _ = window.maximize();
    }
}

/// 真全屏开关：双击展开态的大封面走这里。
///
/// 和最大化不是一回事——最大化只是铺满工作区，任务栏还在；真全屏是独占整个屏幕。
/// 返回切换后的状态，前端要据此改按钮/提示文案。
#[tauri::command]
#[cfg(desktop)]
pub fn window_toggle_fullscreen(window: WebviewWindow) -> Result<bool, String> {
    let next = !window.is_fullscreen().map_err(|e| e.to_string())?;
    window.set_fullscreen(next).map_err(|e| e.to_string())?;
    #[cfg(debug_assertions)]
    eprintln!("[aura] window_toggle_fullscreen -> {next}");
    Ok(next)
}

#[tauri::command]
#[cfg(not(desktop))]
pub fn window_toggle_fullscreen(_window: WebviewWindow) -> Result<bool, String> {
    Ok(false)
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

        FLOAT_READY.store(false, std::sync::atomic::Ordering::Release);

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
        // 先建成隐藏的：页面渲染出来之前显示它，等于在桌面上摆一块看不见的挡板
        // （透明 + 置顶 + 1000x112，鼠标点击全被它吃掉，而关闭按钮就在那张没渲染
        // 出来的页面里）。`float_window_ready` 收到报到才 show，见 FLOAT_READY。
        .visible(false)
        .build()
        .map_err(|e| e.to_string())?;

        // 贴回销毁前记下的位置，否则会回到默认位置——轻量模式来回一趟歌词条就跑了。
        // 窗口这会儿还是隐藏的，先摆好位置再显示，用户看不到中间那一跳。
        if let Some((x, y)) = FLOAT_POSITION.lock().ok().and_then(|slot| *slot) {
            let _ = _float.set_position(tauri::PhysicalPosition::new(x, y));
        }

        // 新的一扇窗 = 新的一条命。必须在起看门狗之前 +1，好让它捕到的是这一代。
        FLOAT_GENERATION.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        spawn_float_ready_watchdog(app.clone());
        return Ok(());
    }
    if let Some(float) = app.get_webview_window("float") {
        let _ = float.set_always_on_top(true);
        let _ = float.set_shadow(false);
        // 窗口在、但页面从来没报到过：这就是那块隐形挡板的来源（dev server 重启会
        // 把它的连接弄断）。重新加载并让看门狗接手，别直接 show。
        if !FLOAT_READY.load(std::sync::atomic::Ordering::Acquire) {
            #[cfg(debug_assertions)]
            eprintln!("[aura] float window exists but never reported ready, reloading");
            #[cfg(debug_assertions)]
            let _ = float.navigate(dev_float_url.clone());
            #[cfg(not(debug_assertions))]
            let _ = float.eval("location.reload()");
            spawn_float_ready_watchdog(app.clone());
            return Ok(());
        }
        float.show().map_err(|e| e.to_string())?;
        apply_float_ignore_from_settings(&app, &settings_state);
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

/// 浮窗页面是否已经挂载完成（`float_window_ready` 被调过）。
///
/// 存在的意义是：浮窗是 `transparent` + `decorations: false` + `always_on_top` 的
/// 1000x112 窗口，页面一旦没渲染出来（dev server 重启把它的连接弄断、导航失败），
/// 剩下的就是一块**看不见但吃光鼠标点击**的矩形压在桌面上，用户完全无从下手——
/// 关闭按钮就在那张没渲染出来的页面里。所以窗口先建成隐藏的，等页面自己报到再显示。
#[cfg(desktop)]
static FLOAT_READY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 浮窗页面挂载完成后自己调这个。
///
/// 显不显示交回 `sync_float_visibility_for_main` 统一决定，不能无脑 show：页面加载
/// 期间用户可能已经把主窗口调回前台，那时按设置浮窗本该是藏着的。
#[tauri::command]
#[cfg(desktop)]
pub fn float_window_ready(app: AppHandle) {
    #[cfg(debug_assertions)]
    eprintln!("[aura] float_window_ready");
    FLOAT_READY.store(true, std::sync::atomic::Ordering::Release);
    let main_visible = !MAIN_HIDDEN.load(std::sync::atomic::Ordering::Relaxed);
    sync_float_visibility_for_main(&app, main_visible);
}

#[tauri::command]
#[cfg(not(desktop))]
pub fn float_window_ready(_app: AppHandle) {}

/// 看门狗是否已经在跑。`show_float_window` 在"窗口在但没报到"这条分支上每次调用都会
/// 想起一个看门狗，而这条分支会被 `sync_float_visibility_for_main` 的每次可见性翻转
/// 撞到——不设闸的话会攒下一串线程，各自 reload 一次、各自 destroy 一次。
#[cfg(desktop)]
static FLOAT_WATCHDOG: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 浮窗的"第几条命"。每次新建、以及每次被别人主动销毁都 +1。
///
/// 看门狗线程睡着的那 6 秒里窗口可能已经被别人处理掉了（进轻量模式会主动 destroy 浮窗，
/// 见 `mini/commands.rs`）。线程不可取消，所以醒来后必须自己判断"我等的还是那一扇窗吗"：
/// 代数变了就闭嘴退出。否则它会替一次正常的销毁去发 `float-lyric-closed`，把用户的
/// 桌面歌词开关静默关掉。
#[cfg(desktop)]
static FLOAT_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// 浮窗被本模块之外的代码销毁时调用，作废当前那条看门狗。
#[cfg(desktop)]
pub fn note_float_destroyed() {
    FLOAT_GENERATION.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
}

/// 页面迟迟不报到就自救：先重新导航一次，还是不行就把窗口销毁。
///
/// 宁可"桌面歌词没开出来"，也不能留一块隐形挡板。销毁后顺带通知主窗，
/// 让设置里的开关回到关闭状态，用户下次点开才是一次真正的重试。
#[cfg(desktop)]
fn spawn_float_ready_watchdog(app: AppHandle) {
    use std::sync::atomic::Ordering;

    if FLOAT_WATCHDOG.swap(true, Ordering::AcqRel) {
        return;
    }
    let generation = FLOAT_GENERATION.load(Ordering::Acquire);

    std::thread::spawn(move || {
        // 无论从哪个分支退出都要放闸，否则这个窗口这辈子再也等不到第二次自救。
        struct Guard;
        impl Drop for Guard {
            fn drop(&mut self) {
                FLOAT_WATCHDOG.store(false, Ordering::Release);
            }
        }
        let _guard = Guard;

        // 醒来后先确认这扇窗还是我等的那一扇：报到了、代数变了（别人销毁并/或重建过）、
        // 或者窗口干脆已经没了，三种情况都不该再动手。
        let still_ours = |app: &AppHandle| {
            !FLOAT_READY.load(Ordering::Acquire)
                && FLOAT_GENERATION.load(Ordering::Acquire) == generation
                && app.get_webview_window("float").is_some()
        };

        std::thread::sleep(std::time::Duration::from_millis(2500));
        if !still_ours(&app) {
            return;
        }
        if let Some(float) = app.get_webview_window("float") {
            #[cfg(debug_assertions)]
            eprintln!("[aura] float window not ready in 2.5s, reloading");
            let _ = float.eval("location.reload()");
        }

        std::thread::sleep(std::time::Duration::from_millis(3500));
        if !still_ours(&app) {
            return;
        }
        eprintln!("[aura] 桌面歌词页面没能加载出来，销毁浮窗以免留下隐形挡板");
        if let Some(float) = app.get_webview_window("float") {
            let _ = float.destroy();
        }
        // id 必须每次都不一样：主窗那边（`LyricSync.svelte` 的 `handleFloatLyricClosed`）
        // 把 id 当一次性 token 去重，字面量会让第二次及以后的超时被整条吞掉——开关停在
        // 开启、窗口却没有，然后每次主窗可见性翻转都来一轮 6 秒的建/销。
        let id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| format!("float-load-failed-{}", d.as_nanos()))
            .unwrap_or_else(|_| "float-load-failed".to_string());
        let payload = serde_json::json!({ "id": id, "reason": "load-timeout" });
        let _ = app.emit_to("main", "float-lyric-closed", payload);
    });
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
