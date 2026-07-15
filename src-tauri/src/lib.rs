mod cache;
mod download;
mod local_music;
mod proxy;
mod system_stats;
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
                let rt = tokio::runtime::Runtime::new().unwrap();
                let result = rt.block_on(proxy::handle_stream_protocol(request));
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
                let main_window = app.get_webview_window("main").unwrap();
                setup_tray(app, main_window)?;
            }
            #[cfg(debug_assertions)]
            eprintln!("[listen1] setup complete");
            Ok(())
        })
        .on_window_event(|window, event| {
            match event {
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    if window.label() == "main" {
                        #[cfg(desktop)]
                        let app = window.app_handle().clone();
                        window.hide().unwrap();
                        #[cfg(desktop)]
                        crate::window::sync_float_visibility_for_main(&app, false);
                        api.prevent_close();
                    } else if window.label() == "login" {
                        let _ = window.hide();
                        api.prevent_close();
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
            system_stats::get_resource_usage,
        ]);

    #[cfg(all(debug_assertions, desktop))]
    let builder = if std::env::var("LISTEN1_ENABLE_MCP").as_deref() == Ok("1") {
        builder
            .plugin(mcp_event_shim())
            .plugin(tauri_plugin_mcp_bridge::init())
    } else {
        builder
    };

    builder
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
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

#[cfg(desktop)]
fn setup_tray(
    app: &mut tauri::App,
    main_window: tauri::WebviewWindow,
) -> Result<(), Box<dyn std::error::Error>> {
    use tauri::menu::{MenuBuilder, MenuItemBuilder};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let quit = MenuItemBuilder::with_id("quit", "退出").build(app)?;
    let show = MenuItemBuilder::with_id("show", "显示/隐藏").build(app)?;
    let play_pause = MenuItemBuilder::with_id("play_pause", "播放/暂停").build(app)?;
    let prev = MenuItemBuilder::with_id("prev", "上一首").build(app)?;
    let next = MenuItemBuilder::with_id("next", "下一首").build(app)?;

    let menu = MenuBuilder::new(app)
        .item(&show)
        .separator()
        .item(&play_pause)
        .item(&prev)
        .item(&next)
        .separator()
        .item(&quit)
        .build()?;

    let win_clone = main_window.clone();
    let playback_window = main_window.clone();
    let app_handle = app.handle().clone();

    TrayIconBuilder::new()
        .icon(app.default_window_icon().unwrap().clone())
        .menu(&menu)
        .on_menu_event(move |_app, event| {
            match event.id().as_ref() {
                "quit" => {
                    #[cfg(debug_assertions)]
                    eprintln!("[listen1] tray quit");
                    app_handle.exit(0);
                }
                "show" => {
                    #[cfg(debug_assertions)]
                    eprintln!("[listen1] tray show");
                    toggle_main_window(&win_clone);
                }
                "play_pause" => {
                    #[cfg(debug_assertions)]
                    eprintln!("[listen1] tray play_pause");
                    dispatch_tray_action(&app_handle, &playback_window, "play_pause");
                }
                "prev" => {
                    #[cfg(debug_assertions)]
                    eprintln!("[listen1] tray prev");
                    dispatch_tray_action(&app_handle, &playback_window, "prev");
                }
                "next" => {
                    #[cfg(debug_assertions)]
                    eprintln!("[listen1] tray next");
                    dispatch_tray_action(&app_handle, &playback_window, "next");
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
                let win = tray.app_handle().get_webview_window("main").unwrap();
                toggle_main_window(&win);
            }
        })
        .build(app)?;

    Ok(())
}
