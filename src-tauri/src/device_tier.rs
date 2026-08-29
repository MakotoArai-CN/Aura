//! 设备性能分级：启动时检测一次，决定**视觉效果**的档位。
//!
//! High：现代高性能 CPU + 独立显卡 → 完整视觉效果（毛玻璃/模糊背景）。
//! Mid：高性能 CPU、无独显（核显/虚拟显卡）→ 视觉降级。
//! Low：中低端 CPU（或内存不足）→ 纯色 UI，禁用全部模糊/透明合成效果。
//!
//! 注意档位**不再决定 GPU**。GPU 加速现在是一个独立开关（默认关闭），
//! 见 `should_enable_gpu_acceleration`。档位只影响 CSS 那一层。

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

/// 前端改滑条时调用，把用户锁定的档位落盘。
///
/// 注意：Rust 侧现在**不读**这个文件了。它原来的唯一用途是喂
/// `should_enable_gpu_acceleration`，而 GPU 已经改由独立开关决定（见下面那个函数的
/// 注释）。渲染档位现在纯粹由前端按 localStorage 里的 `effectTier` 决定 CSS。
/// 保留这次写盘是因为它是用户选择的持久记录，删掉会连带改动前端三处调用点。
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

/// GPU 加速开关的落盘位置。一行 `on` / `off`。
///
/// 和 `effect_tier` 一样必须落盘，不能读 localStorage：WebView2 的 GPU 启动参数要在
/// 第一个 WebView 创建之前写进环境变量，那一刻前端还没有运行。
///
/// 目录沿用 `Listen1`（不是 `Aura`）——旁边的 `effect_tier` 已经在那儿了，换目录会让
/// 用户已有的锁档设置凭空消失。
fn gpu_path() -> std::path::PathBuf {
    crate::cache::user_home_dir()
        .join("Listen1")
        .join("gpu_acceleration")
}

/// 解析开关文件。抽成纯函数是为了能直接测——真正的读盘路径依赖用户目录，
/// 单元测试里没法稳定构造。
///
/// 缺失、内容非法、空文件都按**关闭**处理：这个开关默认关，而"读不出来"和
/// "用户没开过"应该是同一个结果，不能让一个写坏的文件把 GPU 悄悄打开。
fn parse_gpu_flag(raw: &str) -> bool {
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "on" | "true" | "1" | "yes"
    )
}

/// 前端改开关时调用。和档位一样，写的是"下一次启动"的决策依据。
#[tauri::command]
pub fn set_gpu_acceleration(enabled: bool) -> Result<(), String> {
    let path = gpu_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let value = if enabled { "on" } else { "off" };
    std::fs::write(&path, value).map_err(|err| err.to_string())
}

/// 当前落盘的 GPU 开关值，供前端回显。
#[tauri::command]
pub fn get_gpu_acceleration() -> bool {
    should_enable_gpu_acceleration()
}

/// 供 WebView2 启动参数决策：**只看这个开关，默认关闭**。
///
/// 这里刻意不再参考渲染档位。以前是"仅 High 档开 GPU"，也就是 GPU 跟着视觉档位走；
/// 用户要的是一个独立开关，而且默认不开 GPU，所以现在开关是唯一输入，档位对 GPU
/// 没有任何影响了。要改 GPU 只有这一个地方，不存在"档位动了 GPU 跟着变"这种隐式联动。
///
/// 历史教训：`a0ff44e` 曾把前端看门狗的降档结果落盘，让它参与下一次启动的 GPU 决策
/// （`973095c` 已 revert）。那条路是自动推断——一次偶发卡顿就能把 GPU 永久关掉，
/// 而 GPU 一关软件渲染更慢、看门狗又更容易降档，自我强化。显式开关不会有这个问题。
pub fn should_enable_gpu_acceleration() -> bool {
    std::fs::read_to_string(gpu_path())
        .map(|raw| parse_gpu_flag(&raw))
        .unwrap_or(false)
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
    use super::{is_low_voltage_cpu, parse_gpu_flag};

    #[test]
    fn gpu_flag_defaults_to_off() {
        // 缺失文件走的是 read_to_string 的 Err 分支，这里覆盖"文件在但内容说不清"的情况：
        // 空、空白、乱码、以及明确的 off，全都必须是关。
        assert!(!parse_gpu_flag(""));
        assert!(!parse_gpu_flag("   \n"));
        assert!(!parse_gpu_flag("off"));
        assert!(!parse_gpu_flag("false"));
        assert!(!parse_gpu_flag("0"));
        assert!(!parse_gpu_flag("garbage"));
        // 半个词也不能算开——避免 "onward" 之类的内容误判。
        assert!(!parse_gpu_flag("onward"));
    }

    #[test]
    fn gpu_flag_accepts_the_written_form_and_common_synonyms() {
        // set_gpu_acceleration 写的是 "on"/"off"，这一对必须能往返。
        assert!(parse_gpu_flag("on"));
        assert!(!parse_gpu_flag("off"));
        // 手改文件的人多半会写这些，认了不吃亏。
        assert!(parse_gpu_flag("true"));
        assert!(parse_gpu_flag("1"));
        assert!(parse_gpu_flag("yes"));
        // 大小写和前后空白都不该影响判断。
        assert!(parse_gpu_flag("  ON  \r\n"));
        assert!(parse_gpu_flag("True"));
    }

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
