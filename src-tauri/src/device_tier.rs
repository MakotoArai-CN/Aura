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

/// 用户手动锁定的档位落盘位置。和音频缓存同一个 Listen1 目录，一行纯文本。
fn override_path() -> std::path::PathBuf {
    crate::cache::user_home_dir()
        .join("Listen1")
        .join("effect_tier")
}

/// 读手动锁定的档位（返回 None 表示自动）。
///
/// 为什么读文件而不读 localStorage：WebView2 的 GPU 启动参数必须在第一个 WebView
/// 创建之前写进环境变量，那一刻前端还没有运行，localStorage 根本读不到。
fn manual_override() -> Option<String> {
    let raw = std::fs::read_to_string(override_path()).ok()?;
    let value = raw.trim().to_ascii_lowercase();
    match value.as_str() {
        "ultra" | "high" | "mid" | "low" => Some(value),
        // "auto" 或文件内容非法 → 当作没锁。
        _ => None,
    }
}

/// 前端改滑条时调用。写的是"下一次启动"的 GPU 决策依据，本次运行只有 CSS 会立刻变。
#[tauri::command]
pub fn set_effect_tier_override(tier: String) -> Result<(), String> {
    let value = tier.trim().to_ascii_lowercase();
    match value.as_str() {
        "auto" | "ultra" | "high" | "mid" | "low" => {}
        _ => return Err(format!("unknown effect tier: {tier}")),
    }

    let path = override_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    std::fs::write(&path, value).map_err(|err| err.to_string())
}

/// 供 WebView2 启动参数决策：仅 High 档（或手动锁到 high/ultra）启用 GPU。
pub fn should_enable_gpu_acceleration() -> bool {
    // 手动锁档优先，且这里读到的是上一次运行写下的值——用户改档后 GPU 参数要下次
    // 启动才生效，正是和用户确认过的行为（CSS 立即生效，CPU/GPU 部分下次启动）。
    if let Some(manual) = manual_override() {
        return manual == "ultra" || manual == "high";
    }
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

    // 安卓：GPU 一律是集成的，"有没有独显"在这里没有意义（原先那套 /sys/class/drm
    // 判定在安卓上还会因为读不到目录、`!` 一取反而误判成"有独显"）。移动 GPU 上
    // 常驻 backdrop-filter 的代价远高于桌面，所以自动最高只到 Mid：交互动效全部
    // 保留，只去掉常驻模糊纹理。想要更好效果的用户可以自己把滑条拉上去。
    #[cfg(target_os = "android")]
    {
        return if total_gb >= 6 && logical_cores >= 8 {
            DeviceTier::Mid
        } else {
            DeviceTier::Low
        };
    }

    #[cfg(not(target_os = "android"))]
    {
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

        let cpu_tier = if modern_brand
            || (logical_cores >= 8 && clock_mhz >= 2800)
            || (physical_cores >= 6 && clock_mhz >= 2400)
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

        // 低压 U 后缀（i7-1165U / 7730U 之类，TDP 15W 上下）：型号名能进上面的
        // modern_brand 白名单，但持续跑毛玻璃合成时会一直卡在降频状态，实测比标压
        // 低两档还难受。没有独显就一律最低档；真接了独显（外接显卡坞之类）则不额外
        // 惩罚，按下面的常规逻辑走。
        if is_low_voltage_cpu(&brand) && !has_dgpu {
            return DeviceTier::Low;
        }

        match (cpu_tier, has_dgpu) {
            (2, true) => DeviceTier::High,
            (2, false) => DeviceTier::Mid,
            // 中端 CPU 一律走纯色 UI 档（用户分级：仅高性能 CPU 享受前两档）。
            _ => DeviceTier::Low,
        }
    }
}

/// 低压 CPU 判定：型号尾缀 U（`i5-8250u` / `ryzen 7 7730u`）。
///
/// 刻意不做 `brand.contains("u")` 这种事——白名单里就有 `ultra` 和 `ryzen ai`，
/// 品牌串里还满是 `cpu` / `genuine`。这里要求 u 的前一个字符是数字（型号数字尾缀），
/// 后一个字符不是字母数字（或者已经到串尾），把 `core ultra 7 155h` 排除掉，
/// 同时保住真正的 `155u`。
#[cfg(not(target_os = "android"))]
fn is_low_voltage_cpu(brand: &str) -> bool {
    let bytes = brand.as_bytes();
    bytes.iter().enumerate().any(|(i, &b)| {
        if !b.eq_ignore_ascii_case(&b'u') {
            return false;
        }
        let prev_is_digit = i > 0 && bytes[i - 1].is_ascii_digit();
        let next_is_boundary = bytes
            .get(i + 1)
            .map_or(true, |next| !next.is_ascii_alphanumeric());
        prev_is_digit && next_is_boundary
    })
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

#[cfg(all(not(windows), not(target_os = "android")))]
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

#[cfg(all(test, not(target_os = "android")))]
mod tests {
    use super::is_low_voltage_cpu;

    #[test]
    fn detects_u_suffix_models() {
        assert!(is_low_voltage_cpu("intel(r) core(tm) i5-8250u cpu @ 1.60ghz"));
        assert!(is_low_voltage_cpu("intel(r) core(tm) i7-1165u"));
        assert!(is_low_voltage_cpu("amd ryzen 7 7730u with radeon graphics"));
        assert!(is_low_voltage_cpu("intel(r) core(tm) ultra 5 155u"));
    }

    #[test]
    fn ignores_incidental_u() {
        // 白名单词本身带 u，绝不能因此判低压
        assert!(!is_low_voltage_cpu("intel(r) core(tm) ultra 7 155h"));
        assert!(!is_low_voltage_cpu("amd ryzen ai 9 hx 370"));
        assert!(!is_low_voltage_cpu("intel(r) core(tm) i9-14900k cpu"));
        assert!(!is_low_voltage_cpu("amd ryzen 9 7950x 16-core processor"));
        // 数字后面跟 u 但 u 后还有字母 → 不是尾缀
        assert!(!is_low_voltage_cpu("some 8265ux part"));
        assert!(!is_low_voltage_cpu(""));
    }
}
