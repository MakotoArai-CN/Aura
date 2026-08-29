mod cache;
mod device_tier;
mod download;
mod local_music;
mod mini;
mod proxy;
mod system_stats;
mod update;
mod window;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let stream_base_url = match proxy::ensure_stream_server() {
        Ok(url) => url.to_string(),
        Err(err) => {
            eprintln!("[listen1] failed to start stream server: {err}");
            String::new()
        }
    };
    #[cfg(debug_assertions)]
    eprintln!("[listen1] starting app, stream_base_url={stream_base_url}");

    // 设备分级决定 WebView2 渲染策略。必须在首个 WebView 创建前设置。
    // 与渲染质量无关的省内存开关（OOUI/SmartScreen/独立音频进程）三档通用；
    // 只有非 high 档才额外关掉 GPU 合成走软件渲染——high 档保持 GPU 加速，
    // 视觉效果与 GitHub 原版一致。
    #[cfg(debug_assertions)]
    eprintln!("[listen1] device_tier={:?}", device_tier::detect());

    #[cfg(windows)]
    {
        let mut args = String::from(
            "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection,AudioServiceOutOfProcess",
        );
        if !device_tier::should_enable_gpu_acceleration() {
            args.push_str(" --disable-gpu --disable-gpu-compositing");
        }
        std::env::set_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS", args);
    }

    let builder = tauri::Builder::default()
        .plugin(stream_base_url_plugin(&stream_base_url))
        .plugin(login_window_helper_plugin())
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init());

    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_global_shortcut::Builder::new().build());

    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_autostart::init(
        tauri_plugin_autostart::MacosLauncher::LaunchAgent,
        None,
    ));

    let builder = builder
        .manage(window::FloatingLyricState::default())
        .manage(window::FloatingLyricSettingsState::default())
        .register_asynchronous_uri_scheme_protocol("stream", move |_ctx, request, responder| {
            std::thread::spawn(move || {
                // 复用进程级共享 Runtime，避免每请求创建 tokio Runtime。
                let result = proxy::shared_runtime().block_on(proxy::handle_stream_protocol(request));
                match result {
                    Ok(response) => responder.respond(response),
                    Err(e) => {
                        let body = e.as_bytes().to_vec();
                        let resp = tauri::http::Response::builder()
                            .status(500)
                            .header("Access-Control-Allow-Origin", "*")
                            .body(body)
                            .unwrap();
                        responder.respond(resp);
                    }
                }
            });
        })
        .setup(|app| {
            #[cfg(desktop)]
            {
                setup_tray(app)?;
            }
            #[cfg(debug_assertions)]
            eprintln!("[listen1] setup complete");
            Ok(())
        })
        .on_window_event(|window, event| {
            match event {
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    if window.label() == "main" {
                        // 桌面端：关闭=隐藏到托盘；移动端：直接放行退出（hide 不可用时 unwrap 会 panic）。
                        #[cfg(desktop)]
                        {
                            let app = window.app_handle().clone();
                            let _ = window.hide();
                            crate::window::sync_float_visibility_for_main(&app, false);
                            api.prevent_close();
                        }
                        #[cfg(not(desktop))]
                        {
                            let _ = &api;
                        }
                    } else if window.label() == "login" {
                        #[cfg(desktop)]
                        {
                            let _ = window.hide();
                            api.prevent_close();
                        }
                    }
                }
                // 处理（含任务栏）最小化/还原：自定义标题栏按钮走 window_minimize 命令，
                // 但从任务栏最小化会绕过它，且 WebView 的 visibilitychange 在最小化时不可靠，
                // 故在此根据 is_minimized 同步浮窗显隐，确保主界面消失后桌面歌词能弹出。
                #[cfg(desktop)]
                tauri::WindowEvent::Resized(_) => {
                    if window.label() == "main" {
                        let minimized = window.is_minimized().unwrap_or(false);
                        crate::window::sync_float_visibility_for_main(&window.app_handle(), !minimized);
                    }
                }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            proxy::http_request,
            proxy::audio_stream_url,
            proxy::get_backend_cookie,
            proxy::set_backend_cookie,
            proxy::set_proxy_config,
            proxy::get_proxy_config,
            window::window_minimize,
            window::window_maximize,
            window::window_close,
            window::window_quit,
            window::open_login_window,
            window::close_login_window,
            window::sync_login_cookies,
            window::get_login_cookies,
            window::clear_login_cookies,
            window::show_float_window,
            window::hide_float_window,
            window::close_float_window,
            window::set_float_window_ignore_mouse,
            window::set_float_window_height,
            window::move_float_window,
            window::get_float_window_position,
            window::set_floating_lyric_payload,
            window::get_floating_lyric_payload,
            window::set_floating_lyric_settings,
            window::get_floating_lyric_settings,
            local_music::read_audio_tags,
            local_music::scan_music_directory,
            download::default_music_dir,
            download::download_track,
            cache::default_cache_dir,
            cache::set_cache_config,
            cache::get_cache_stats,
            cache::clear_audio_cache,
            cache::audio_cache_lookup,
            system_stats::get_resource_usage,
            device_tier::get_device_tier,
            device_tier::set_effect_tier_override,
            update::download_and_run_update,
            update::get_update_assets,
            mini::commands::mini_supported,
            mini::commands::mini_fetch_cover,
            mini::commands::mini_precache_audio,
            mini::commands::mini_enter,
            mini::commands::mini_exit,
            mini::commands::mini_load_snapshot,
        ]);

    #[cfg(all(debug_assertions, desktop))]
    let builder = if std::env::var("LISTEN1_ENABLE_MCP").as_deref() == Ok("1") {
        builder
            .plugin(mcp_event_shim())
            .plugin(tauri_plugin_mcp_bridge::init())
    } else {
        builder
    };

    let app = builder
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|_app, event| {
        // 轻量模式下 main 窗口是真被销毁的，不是隐藏。Tauri 默认"没有窗口就退出进程"，
        // 而这时候原生迷你播放器还在另一个线程里放着歌，所以必须拦住。
        // 但托盘「退出」也会走到这儿，那种情况已经立了 shutdown 旗子，不能再拦。
        if let tauri::RunEvent::ExitRequested { api, .. } = &event {
            if mini::is_lite_active() && !mini::is_shutting_down() {
                api.prevent_exit();
            }
        }
    });
}

fn stream_base_url_plugin<R: tauri::Runtime>(base_url: &str) -> tauri::plugin::TauriPlugin<R> {
    let value = serde_json::to_string(base_url).unwrap_or_else(|_| "\"\"".to_string());
    tauri::plugin::Builder::new("listen1-stream-base")
        .js_init_script(format!(
            "Object.defineProperty(window, '__LISTEN1_STREAM_BASE_URL__', {{ value: {value} }});"
        ))
        .build()
}

fn login_window_helper_plugin<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("listen1-login-helper")
        .js_init_script(
            r#"
            (function() {
                if (window.__LISTEN1_LOGIN_HELPER__) return;
                Object.defineProperty(window, '__LISTEN1_LOGIN_HELPER__', { value: true });

                function isLoginWindow() {
                    return window.name === 'listen1-login' || location.pathname.endsWith('/login.html');
                }

                function sameWindow(url) {
                    if (!isLoginWindow() || !url) return false;
                    try {
                        location.href = new URL(url, location.href).href;
                        return true;
                    } catch (_) {
                        return false;
                    }
                }

                var nativeOpen = window.open;
                window.open = function(url, target, features) {
                    if (sameWindow(url)) return null;
                    return nativeOpen ? nativeOpen.call(window, url, target, features) : null;
                };

                window.addEventListener('click', function(event) {
                    if (!isLoginWindow()) return;
                    var target = event.target;
                    var anchor = target && target.closest ? target.closest('a[target="_blank"], a[target="blank"]') : null;
                    if (!anchor || !anchor.href) return;
                    event.preventDefault();
                    event.stopPropagation();
                    sameWindow(anchor.href);
                }, true);

                window.addEventListener('submit', function(event) {
                    if (!isLoginWindow()) return;
                    var form = event.target;
                    if (!form || !form.target || form.target === '_self') return;
                    form.target = '_self';
                }, true);
            }());
            "#,
        )
        .build()
}

#[cfg(all(debug_assertions, desktop))]
fn mcp_event_shim<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("mcp-event-shim")
        .js_init_script(
            r#"
            (function() {
                function install() {
                    try {
                        var tauri = window.__TAURI__;
                        if (!tauri || !tauri.core || !tauri.core.invoke) {
                            setTimeout(install, 50);
                            return;
                        }

                        var eventApi = tauri.event || {};
                        var nativeEmit = eventApi.emit;

                        if (!eventApi.__listen1McpShim) {
                            eventApi.emit = function(event, payload) {
                                if (event === '__script_result' && payload && payload.exec_id) {
                                    return tauri.core.invoke('plugin:mcp-bridge|script_result', {
                                        execId: payload.exec_id,
                                        success: payload.success,
                                        data: payload.data,
                                        error: payload.error
                                    });
                                }

                                if (nativeEmit) {
                                    return nativeEmit(event, payload);
                                }

                                return tauri.core.invoke('plugin:event|emit', {
                                    event: event,
                                    payload: payload
                                });
                            };
                            eventApi.__listen1McpShim = true;
                            tauri.event = eventApi;
                        }
                    } catch (_) {
                        setTimeout(install, 50);
                    }
                }

                install();
            }());
            "#,
        )
        .build()
}

#[cfg(desktop)]
fn main_window_is_shown(window: &tauri::WebviewWindow) -> bool {
    let visible = window.is_visible().unwrap_or(false);
    let minimized = window.is_minimized().unwrap_or(false);
    visible && !minimized
}

#[cfg(desktop)]
fn show_main_window(window: &tauri::WebviewWindow) {
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
    crate::window::sync_float_visibility_for_main(&window.app_handle(), true);
}

#[cfg(desktop)]
fn toggle_main_window(window: &tauri::WebviewWindow) {
    if main_window_is_shown(window) {
        let app = window.app_handle().clone();
        let _ = window.hide();
        crate::window::sync_float_visibility_for_main(&app, false);
    } else {
        show_main_window(window);
    }
}

#[cfg(desktop)]
fn dispatch_tray_action(
    app_handle: &tauri::AppHandle,
    window: &tauri::WebviewWindow,
    action: &str,
) {
    use tauri::Emitter;

    let action_id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| format!("{action}-{}", duration.as_nanos()))
        .unwrap_or_else(|_| action.to_string());
    let payload = serde_json::json!({
        "action": action,
        "id": action_id,
    });

    let _ = app_handle.emit_to("main", "tray-action", payload.clone());
    let _ = app_handle.emit("tray-action", payload.clone());

    if let Ok(action_json) = serde_json::to_string(&payload) {
        let script = format!(
            "window.dispatchEvent(new CustomEvent('listen1-tray-action', {{ detail: {action_json} }}));"
        );
        let _ = window.eval(&script);
    }
}

/// 托盘点「显示/隐藏」。
///
/// 不能把 `WebviewWindow` 提前捕获住：轻量模式会真的销毁 main 窗口再重建，
/// 旧句柄从此指向一个死窗口，显隐会静默失效。所以每次都按 label 重新查。
#[cfg(desktop)]
fn reveal_main_window(app_handle: &tauri::AppHandle) {
    use tauri::Manager;

    if crate::mini::is_lite_active() {
        crate::mini::focus_lite();
        return;
    }
    if let Some(win) = app_handle.get_webview_window("main") {
        toggle_main_window(&win);
    }
}

/// 托盘的播放控制。轻量模式下 JS 那条路（emit + eval）已经没有接收方了，
/// 得改发窗口消息给原生播放器。
#[cfg(desktop)]
fn tray_transport(app_handle: &tauri::AppHandle, action: &str) {
    use tauri::Manager;

    if crate::mini::is_lite_active() {
        let mapped = match action {
            "play_pause" => Some(crate::mini::LiteAction::PlayPause),
            "prev" => Some(crate::mini::LiteAction::Prev),
            "next" => Some(crate::mini::LiteAction::Next),
            _ => None,
        };
        if let Some(mapped) = mapped {
            crate::mini::lite_action(mapped);
        }
        return;
    }
    if let Some(win) = app_handle.get_webview_window("main") {
        dispatch_tray_action(app_handle, &win, action);
    }
}

/// 托盘那个「切换/退出轻量模式」条目的句柄。
///
/// 托盘菜单没有「即将弹出」的回调，文案改不了是因为拿不到条目——所以建的时候留一份。
#[cfg(desktop)]
static LITE_MENU_ITEM: std::sync::OnceLock<tauri::menu::MenuItem<tauri::Wry>> =
    std::sync::OnceLock::new();

/// 按当前是否处在轻量模式刷新托盘条目的文案。
///
/// 进/出轻量模式时各调一次。菜单项的改动必须回主线程做——macOS 上在别的线程动菜单
/// 会直接崩，Windows 上虽然能过，但没有理由两个平台走不同的路。
#[cfg(desktop)]
pub fn sync_lite_menu_label(app: &tauri::AppHandle, lite_active: bool) {
    let Some(item) = LITE_MENU_ITEM.get() else {
        return;
    };
    let item = item.clone();
    let text = if lite_active {
        "退出轻量模式"
    } else {
        "切换轻量模式"
    };
    let _ = app.run_on_main_thread(move || {
        let _ = item.set_text(text);
    });
}

/// 托盘点「切换轻量模式」。
///
/// 进轻量模式没法只在 Rust 侧完成：快照得让前端先把封面、音频、歌词都落盘
/// （直链带签名会过期，歌词也没有磁盘缓存），所以这一半只发事件，由 WebView 自己走
/// `enterLiteMode`。反方向是纯原生的——请求关掉原生窗口，main 窗口随后会重建。
#[cfg(desktop)]
fn toggle_lite_mode(app_handle: &tauri::AppHandle) {
    use tauri::Manager;

    if crate::mini::is_lite_active() {
        crate::mini::request_close();
        return;
    }
    if let Some(win) = app_handle.get_webview_window("main") {
        // 窗口藏着的时候先显出来：切换要走网络，成功与否都得让用户看见反馈。
        show_main_window(&win);
        dispatch_tray_action(app_handle, &win, "lite_mode");
    }
}

#[cfg(desktop)]
fn setup_tray(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    use tauri::menu::{MenuBuilder, MenuItemBuilder};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let quit = MenuItemBuilder::with_id("quit", "退出").build(app)?;
    let show = MenuItemBuilder::with_id("show", "显示/隐藏").build(app)?;
    let play_pause = MenuItemBuilder::with_id("play_pause", "播放/暂停").build(app)?;
    let prev = MenuItemBuilder::with_id("prev", "上一首").build(app)?;
    let next = MenuItemBuilder::with_id("next", "下一首").build(app)?;
    // 一个条目管两个方向，文案跟着状态走：完整模式下是「切换轻量模式」，
    // 进去之后由 sync_lite_menu_label 改成「退出轻量模式」。
    let lite_mode = MenuItemBuilder::with_id("lite_mode", "切换轻量模式").build(app)?;
    // 留一份句柄，否则状态变化时没有东西可以 set_text。
    let _ = LITE_MENU_ITEM.set(lite_mode.clone());

    let menu = MenuBuilder::new(app)
        .item(&show)
        .separator()
        .item(&play_pause)
        .item(&prev)
        .item(&next)
        .separator()
        .item(&lite_mode)
        .separator()
        .item(&quit)
        .build()?;

    let app_handle = app.handle().clone();

    TrayIconBuilder::new()
        .icon(app.default_window_icon().unwrap().clone())
        .menu(&menu)
        .on_menu_event(move |_app, event| {
            match event.id().as_ref() {
                "quit" => {
                    #[cfg(debug_assertions)]
                    eprintln!("[listen1] tray quit");
                    if crate::mini::is_lite_active() {
                        // 轻量模式下 exit(0) 会被 ExitRequested 拦住。先立旗子表明这次
                        // 关窗是要退进程，再请求关窗——原生侧写完进度后自己调 exit(0)。
                        crate::mini::begin_shutdown();
                        crate::mini::request_close();
                    } else {
                        app_handle.exit(0);
                    }
                }
                "show" => {
                    #[cfg(debug_assertions)]
                    eprintln!("[listen1] tray show");
                    reveal_main_window(&app_handle);
                }
                "play_pause" => {
                    #[cfg(debug_assertions)]
                    eprintln!("[listen1] tray play_pause");
                    tray_transport(&app_handle, "play_pause");
                }
                "prev" => {
                    #[cfg(debug_assertions)]
                    eprintln!("[listen1] tray prev");
                    tray_transport(&app_handle, "prev");
                }
                "next" => {
                    #[cfg(debug_assertions)]
                    eprintln!("[listen1] tray next");
                    tray_transport(&app_handle, "next");
                }
                "lite_mode" => {
                    #[cfg(debug_assertions)]
                    eprintln!("[listen1] tray lite_mode");
                    toggle_lite_mode(&app_handle);
                }
                _ => {}
            }
        })
        .on_tray_icon_event(move |tray, event| {
            // 仅在鼠标「抬起」时响应，否则按下+抬起会各触发一次，导致窗口反复显隐抽搐。
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                reveal_main_window(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}
