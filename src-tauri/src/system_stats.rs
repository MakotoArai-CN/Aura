use serde::Serialize;
use std::sync::{Mutex, OnceLock};
use sysinfo::{Pid, ProcessesToUpdate, System};

#[derive(Debug, Serialize)]
pub struct ResourceUsage {
    memory_bytes: u64,
    cpu_percent: f32,
    process_count: usize,
    webview_count: usize,
}

fn is_aura_webview(process_name: &str, command: &str, parent: Option<Pid>, app_pid: Pid) -> bool {
    let name = process_name.to_ascii_lowercase();
    let cmd = command.to_ascii_lowercase();
    if parent == Some(app_pid) && (name.contains("webview") || name.contains("msedgewebview2")) {
        return true;
    }
    name.contains("msedgewebview2") && cmd.contains("webview-exe-name=aura.exe")
}

/// 持久 System 单例：复用跨调用状态，避免每次 System::new_all() 全量枚举所有子系统。
/// CPU 使用率依赖两次进程刷新的差值，持久实例让相邻采样更稳定。
fn shared_system() -> &'static Mutex<System> {
    static SYSTEM: OnceLock<Mutex<System>> = OnceLock::new();
    SYSTEM.get_or_init(|| Mutex::new(System::new()))
}

#[tauri::command]
pub fn get_resource_usage() -> ResourceUsage {
    let mut system = shared_system().lock().unwrap_or_else(|e| e.into_inner());

    // 刷新 CPU 列表以取得真实核心数（System::new() 初始为空，不刷则 cpus() 为空）。
    system.refresh_cpu_all();
    // 两次刷新（间隔 ~220ms）以取得 CPU 差值；只刷进程，不重建整个 System。
    system.refresh_processes(ProcessesToUpdate::All, true);
    std::thread::sleep(std::time::Duration::from_millis(220));
    system.refresh_processes(ProcessesToUpdate::All, true);

    let app_pid = Pid::from_u32(std::process::id());
    // CPU 归一化到 0-100%：sysinfo 的 cpu_usage() 是相对单核的值（多核可 >100%），
    // 除以核心数得到「占总算力比例」，与任务管理器默认口径一致。
    let cpu_count = system.cpus().len().max(1) as f32;

    let mut memory_bytes = 0_u64;
    let mut cpu_raw = 0_f32;
    let mut process_count = 0_usize;
    let mut webview_count = 0_usize;

    for (pid, process) in system.processes() {
        let name = process.name().to_string_lossy();
        let command = process
            .cmd()
            .iter()
            .map(|part| part.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ");
        let is_app = *pid == app_pid;
        let is_webview = is_aura_webview(&name, &command, process.parent(), app_pid);
        if !is_app && !is_webview {
            continue;
        }

        process_count += 1;
        if is_webview {
            webview_count += 1;
        }
        memory_bytes = memory_bytes.saturating_add(process.memory());
        cpu_raw += process.cpu_usage();
    }

    let cpu_percent = (cpu_raw / cpu_count).clamp(0.0, 100.0);

    ResourceUsage {
        memory_bytes,
        cpu_percent,
        process_count,
        webview_count,
    }
}
