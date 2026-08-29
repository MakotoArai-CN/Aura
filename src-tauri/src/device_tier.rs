//! 设备性能分级：启动时检测一次，决定**视觉效果**的档位。
//!
//! High：现代高性能 CPU + 独立显卡 → 完整视觉效果（毛玻璃/模糊背景）。
//! Mid：高性能 CPU、无独显（核显/虚拟显卡）→ 视觉降级。
//! Low：中低端 CPU（或内存不足）→ 纯色 UI，禁用全部模糊/透明合成效果。
//!
//! 档位同时也是 GPU 加速在**自动模式**下的判据（High 才开）。用户手动选了
//! 开启/关闭则一律优先，见 `should_enable_gpu_acceleration`。

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

/// GPU 加速开关的落盘位置。一行 `auto` / `on` / `off`。
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

/// GPU 加速的三态。
///
/// `Auto` 跟随硬件检测（High 才开），`On` / `Off` 是用户的显式选择，永远优先。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuMode {
    Auto,
    On,
    Off,
}

impl GpuMode {
    fn as_str(self) -> &'static str {
        match self {
            GpuMode::Auto => "auto",
            GpuMode::On => "on",
            GpuMode::Off => "off",
        }
    }
}

/// 解析开关文件。抽成纯函数是为了能直接测——真正的读盘路径依赖用户目录，
/// 单元测试里没法稳定构造。
///
/// 缺失、空文件、内容非法一律按 `Auto`。
///
/// 这条默认值语义是从"一律按关"翻过来的。原来的理由是"不能让一个写坏的文件把 GPU
/// 悄悄打开"，那时默认就是关；现在默认是自动，读不出来就该交给硬件检测。
///
/// 在**Windows**上安全性质没有丢：`detect()` 读不到内存、读不到 CPU 列表、注册表打不开
/// 时都落到 `Low` 或 `Mid`，而 auto 模式只有 `High` 才开 GPU——"什么都测不出来"依然等于
/// "不开 GPU"。而 `--disable-gpu` 整段本来就在 `#[cfg(windows)]` 里，所以这个结论覆盖了
/// 唯一真正消费这个值的平台。
///
/// 别把这句读成"所有平台上读不到硬件都落 Low"：非 Windows 桌面的 `has_discrete_gpu()`
/// 是 fail-open（读不到 `/sys/class/drm` 就当作有独显），那条路上"测不出来"会偏向 High。
/// 等 macOS / Linux 真的开始消费这个开关时，得先把那个判定改成 fail-closed。
fn parse_gpu_mode(raw: &str) -> GpuMode {
    match raw.trim().to_ascii_lowercase().as_str() {
        "on" | "true" | "1" | "yes" => GpuMode::On,
        "off" | "false" | "0" | "no" => GpuMode::Off,
        _ => GpuMode::Auto,
    }
}

/// 前端改开关时调用。和档位一样，写的是"下一次启动"的决策依据。
#[tauri::command]
pub fn set_gpu_acceleration_mode(mode: String) -> Result<(), String> {
    // 刻意不复用 parse_gpu_mode：那个函数把认不出来的内容当 auto，用在这里会把前端
    // 传错的值静默存成 auto。写入口要能报错，读入口才需要宽容。
    let value = match mode.trim().to_ascii_lowercase().as_str() {
        "auto" => GpuMode::Auto,
        "on" => GpuMode::On,
        "off" => GpuMode::Off,
        _ => return Err(format!("unknown gpu acceleration mode: {mode}")),
    };

    let path = gpu_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    std::fs::write(&path, value.as_str()).map_err(|err| err.to_string())
}

/// **本次启动实际生效**的 GPU 开关值，供设置页回显「重启后生效」那句提示。
///
/// 刻意不重新读盘：前端一挂载就会把"下一次启动想要的值"写回同一个文件
/// （`device.ts` 的 `initGpuAcceleration`，先读后写是那边的自愈逻辑），此后盘上
/// 的内容就是待生效值而不是生效值了。再读盘会让第二次挂载起把待生效值当成已生效，
/// 提示直接从「重启后生效」翻成「本次启动已交给 GPU 合成」——而中间并没有重启。
/// 第二次挂载不是罕见路径：进出一次轻量模式就会销毁并重建主 WebView。
#[tauri::command]
pub fn get_gpu_acceleration() -> bool {
    should_enable_gpu_acceleration()
}

/// 供 WebView2 启动参数决策，也供前端回显。三态：`On`/`Off` 是用户的显式选择，
/// `Auto`（默认）跟随硬件检测——只有 `High` 档才开 GPU。
///
/// 值在进程内只读一次就定格（`OnceLock`）。这不是性能优化，是正确性要求：WebView2
/// 的 `--disable-gpu` 只在首个 WebView 创建前生效，所以"本次启动到底有没有 GPU"是
/// 启动那一刻就固定下来的事实。而盘上的文件会被前端改（用户拨开关、以及挂载时的
/// 回写自愈），若每次都读盘，同一次运行里这个函数会前后给出不同答案。第一个调用点
/// 在 `lib.rs` 设置 `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` 的地方，早于任何写入，
/// 因此定格下来的就是真正生效的那个值。
///
/// 自动模式的判据**只能是启动时的静态硬件检测**，绝不能是运行时的 CPU 看门狗。
/// 历史教训：`a0ff44e` 曾把前端看门狗的降档结果落盘，让它参与下一次启动的 GPU 决策
/// （`973095c` 已 revert）。那是自我强化循环——一次偶发卡顿把 GPU 永久关掉，GPU 一关
/// 软件渲染更慢、看门狗又更容易降档。`detect()` 只看 CPU 型号/核数/频率与独显存在性，
/// 同一台机器上稳定，不会被一次卡顿改写。
pub fn should_enable_gpu_acceleration() -> bool {
    static ACTIVE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ACTIVE.get_or_init(|| {
        let mode = std::fs::read_to_string(gpu_path())
            .map(|raw| parse_gpu_mode(&raw))
            .unwrap_or(GpuMode::Auto);
        match mode {
            GpuMode::On => true,
            GpuMode::Off => false,
            GpuMode::Auto => *detected_tier() == DeviceTier::High,
        }
    })
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
    use super::{is_low_voltage_cpu, parse_gpu_mode, GpuMode};

    #[test]
    fn gpu_mode_defaults_to_auto() {
        // 缺失文件走的是 read_to_string 的 Err 分支（那里直接给 Auto），这里覆盖
        // "文件在但内容说不清"：空、空白、乱码全都必须落到 Auto，交给硬件检测。
        assert_eq!(parse_gpu_mode(""), GpuMode::Auto);
        assert_eq!(parse_gpu_mode("   \n"), GpuMode::Auto);
        assert_eq!(parse_gpu_mode("garbage"), GpuMode::Auto);
        // 半个词不能算数——"onward" 不是 on，"offer" 不是 off。
        assert_eq!(parse_gpu_mode("onward"), GpuMode::Auto);
        assert_eq!(parse_gpu_mode("offer"), GpuMode::Auto);
        // 显式写 auto 当然也是 Auto。
        assert_eq!(parse_gpu_mode("auto"), GpuMode::Auto);
    }

    #[test]
    fn gpu_mode_reads_the_written_forms_and_common_synonyms() {
        // set_gpu_acceleration_mode 写的是这三个词，必须能往返。
        assert_eq!(parse_gpu_mode("auto"), GpuMode::Auto);
        assert_eq!(parse_gpu_mode("on"), GpuMode::On);
        assert_eq!(parse_gpu_mode("off"), GpuMode::Off);
        // 手改文件的人多半会写这些，认了不吃亏。
        assert_eq!(parse_gpu_mode("true"), GpuMode::On);
        assert_eq!(parse_gpu_mode("1"), GpuMode::On);
        assert_eq!(parse_gpu_mode("yes"), GpuMode::On);
        assert_eq!(parse_gpu_mode("false"), GpuMode::Off);
        assert_eq!(parse_gpu_mode("0"), GpuMode::Off);
        assert_eq!(parse_gpu_mode("no"), GpuMode::Off);
        // 大小写和前后空白都不该影响判断。
        assert_eq!(parse_gpu_mode("  ON  \r\n"), GpuMode::On);
        assert_eq!(parse_gpu_mode("Off\n"), GpuMode::Off);
        assert_eq!(parse_gpu_mode(" AUTO "), GpuMode::Auto);
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
