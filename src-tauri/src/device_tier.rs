//! 设备性能分级：启动时检测一次，决定渲染策略。
//!
//! High：现代高性能 CPU + 独立显卡 → GPU 加速 + 完整视觉效果（毛玻璃/模糊背景）。
//! Mid：高性能 CPU、无独显（核显/虚拟显卡）→ 软件渲染（--disable-gpu），视觉降级。
//! Low：中低端 CPU（或内存不足）→ 纯色 UI，禁用全部模糊/透明合成效果。

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceTier {
    High,
    Mid,
    Low,
}

impl DeviceTier {
    pub fn as_str(self) -> &'static str {
        match self {
            DeviceTier::High => "high",
            DeviceTier::Mid => "mid",
            DeviceTier::Low => "low",
        }
    }
}

fn detected_tier() -> &'static DeviceTier {
    static TIER: std::sync::OnceLock<DeviceTier> = std::sync::OnceLock::new();
    TIER.get_or_init(detect)
}

/// 供 WebView2 启动参数决策：仅 High 档启用 GPU。
pub fn should_enable_gpu_acceleration() -> bool {
    *detected_tier() == DeviceTier::High
}

#[tauri::command]
pub fn get_device_tier() -> String {
    detected_tier().as_str().to_string()
}

pub(crate) fn detect() -> DeviceTier {
    // 手动覆盖口：LISTEN1_DEVICE_TIER=high|mid|low（调试或用户强制指定）。
    if let Ok(force) = std::env::var("LISTEN1_DEVICE_TIER") {
        match force.to_ascii_lowercase().as_str() {
            "high" => return DeviceTier::High,
            "mid" => return DeviceTier::Mid,
            "low" => return DeviceTier::Low,
            _ => {}
        }
    }

    use sysinfo::System;

    let mut sys = System::new();
    sys.refresh_memory();
    sys.refresh_cpu_all();

    // 内存不足直接判低端，避免多进程 WebView 压垮系统。
    let total_gb = sys.total_memory() / (1024 * 1024 * 1024);
    if total_gb < 4 {
        return DeviceTier::Low;
    }

    let logical_cores = sys.cpus().len().max(1);
    let physical_cores = sys
        .physical_core_count()
        .map(|n| n.max(1))
        .unwrap_or(logical_cores / 2);
    let clock_mhz = sys.cpus().first().map(|cpu| cpu.frequency()).unwrap_or(0);
    let brand = sys
        .cpus()
        .first()
        .map(|cpu| cpu.brand().to_ascii_lowercase())
        .unwrap_or_default();

    // 品牌后缀兜底：现代中高端型号即使核心数/频率被虚拟化误报也归为高性能。
    let modern_brand = ["ultra", "ryzen 9", "ryzen 7", "ryzen ai", "core i9", "core i7"]
        .iter()
        .any(|tag| brand.contains(tag));

    let cpu_tier = if modern_brand || (logical_cores >= 8 && clock_mhz >= 2800) || (physical_cores >= 6 && clock_mhz >= 2400)
    {
        2 // 高性能
    } else if logical_cores >= 4 && clock_mhz >= 1600 {
        1 // 中端
    } else {
        0 // 低端
    };

    if cpu_tier == 0 {
        return DeviceTier::Low;
    }

    let has_dgpu = has_discrete_gpu();
    match (cpu_tier, has_dgpu) {
        (2, true) => DeviceTier::High,
        (2, false) => DeviceTier::Mid,
        // 中端 CPU 一律走纯色 UI 档（用户分级：仅高性能 CPU 享受前两档）。
        _ => DeviceTier::Low,
    }
}

/// 独显判定：优先用 DirectX 注册表项的 DedicatedVideoMemory（独显 ≥ 2GB 专用显存，
/// 核显/虚拟显卡共享内存该项极小），避免脆弱的名称启发式。
#[cfg(windows)]
fn has_discrete_gpu() -> bool {
    use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ};
    use winreg::RegKey;

    const DEDICATED_VRAM_THRESHOLD: u64 = 2 * 1024 * 1024 * 1024; // 2GB

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let Ok(dx) = hklm.open_subkey_with_flags(r"SOFTWARE\Microsoft\DirectX", KEY_READ) else {
        return false;
    };
    for subkey in dx.enum_keys().flatten() {
        let Ok(adapter) = dx.open_subkey_with_flags(&subkey, KEY_READ) else {
            continue;
        };
        // WARP/软件渲染适配器不算独显。
        let name = adapter
            .get_value::<String, _>("Name")
            .unwrap_or_default()
            .to_ascii_lowercase();
        if name.contains("basic render") || name.contains("warp") {
            continue;
        }
        if let Ok(vram) = adapter.get_value::<u64, _>("DedicatedVideoMemory") {
            if vram >= DEDICATED_VRAM_THRESHOLD {
                return true;
            }
        }
    }
    false
}

#[cfg(not(windows))]
fn has_discrete_gpu() -> bool {
    // macOS / Linux 桌面默认具备硬件加速；VMware/VirtualBox 显卡名称含 vmware/virtualbox。
    !std::fs::read_dir("/sys/class/drm")
        .map(|entries| {
            entries.flatten().any(|entry| {
                let vendor = entry.path().join("device/vendor");
                std::fs::read_to_string(vendor)
                    .map(|v| v.trim() == "0x15ad")
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}
