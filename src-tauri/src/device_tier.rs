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

/// 看门狗降档结果的落盘位置。格式一行 `<当时检测到的档位>:<降档后的档位>`。
///
/// 为什么要存"当时检测到的档位"：这个文件的意义是"上次实际跑起来发现撑不住"，
/// 而不是"这台机器永远是低档"。换了显卡、插上电源、关掉别的吃 CPU 的程序之后
/// 检测结果会变，那时旧记录就不该再压着 GPU 不放。检测值一变就作废重新评估。
fn runtime_tier_path() -> std::path::PathBuf {
    crate::cache::user_home_dir()
        .join("Listen1")
        .join("effect_tier_runtime")
}

/// 读上一次运行的看门狗降档结果。仅在"当时的检测值"与本次一致时才认。
fn runtime_downgraded_tier() -> Option<String> {
    let raw = std::fs::read_to_string(runtime_tier_path()).ok()?;
    parse_runtime_record(&raw, *detected_tier())
}

/// 档位排序，数字越小效果越好。只用来判断"记录里的档位是不是比检测值更高"。
fn tier_rank(tier: &str) -> u8 {
    match tier {
        "ultra" => 0,
        "high" => 1,
        "mid" => 2,
        _ => 3,
    }
}

/// 解析降档记录。抽成纯函数是为了能直接测——detected_tier() 是 OnceLock，一个进程
/// 里只能取一个值，不抽出来就没法在单元测试里覆盖多个检测档位。
fn parse_runtime_record(raw: &str, detected: DeviceTier) -> Option<String> {
    let (recorded_detected, effective) = raw.trim().split_once(':')?;
    if recorded_detected.trim().to_ascii_lowercase() != detected.as_str() {
        return None;
    }
    let value = effective.trim().to_ascii_lowercase();
    match value.as_str() {
        "ultra" | "high" | "mid" | "low" => {}
        _ => return None,
    }
    // 记录只可能是"实测撑不住所以降下来"，绝不可能比检测值更高。真读到更高的值说明
    // 文件被手改或写坏了，直接作废——否则会出现 GPU 关着、档位却是 high 的组合，
    // 软件渲染 blur(72px) 是所有搭配里最慢的一种。
    if tier_rank(&value) < tier_rank(detected.as_str()) {
        return None;
    }
    Some(value)
}

/// 前端看门狗降档后调用，记给下一次启动看。传 "auto" 表示清除记录
/// （用户把滑条从自动切走时调用，手动锁档时这条记录不该再插手）。
#[tauri::command]
pub fn set_runtime_tier(tier: String) -> Result<(), String> {
    let path = runtime_tier_path();
    let value = tier.trim().to_ascii_lowercase();
    if value == "auto" {
        // 不存在也算成功：清除一个本来就没有的记录不是错误。
        return match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err.to_string()),
        };
    }
    match value.as_str() {
        "ultra" | "high" | "mid" | "low" => {}
        _ => return Err(format!("unknown runtime tier: {tier}")),
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    std::fs::write(&path, format!("{}:{}", detected_tier().as_str(), value))
        .map_err(|err| err.to_string())
}

/// 纯决策：三个输入（手动锁档 / 检测值 / 上次运行记录）→ 要不要开 GPU。
fn decide_gpu(manual: Option<&str>, detected: DeviceTier, record: Option<&str>) -> bool {
    // 手动锁档优先，且这里读到的是上一次运行写下的值——用户改档后 GPU 参数要下次
    // 启动才生效，正是和用户确认过的行为（CSS 立即生效，CPU/GPU 部分下次启动）。
    if let Some(manual) = manual {
        return manual == "ultra" || manual == "high";
    }
    if detected != DeviceTier::High {
        return false;
    }
    // 检测说 High，但上次运行的看门狗把档位压下去了：说明这台机器实测撑不住，
    // 那就别再开 GPU。原先这条链是断的——看门狗只改 CSS 变量，Rust 侧的 GPU
    // 决策永远只看检测值，于是出现"界面已经是低档、GPU 却一直开着"。
    match record {
        Some(effective) => effective == "ultra" || effective == "high",
        None => true,
    }
}

/// 纯决策：三个输入 → 本次该按哪一档渲染。
///
/// 取值域必须落在三档检测值内（前端 detectedTier 不接受 ultra——那一档只能由用户
/// 手动选，走 settings 那条路），所以手动锁 ultra 时这里回 high。
fn decide_tier(manual: Option<&str>, detected: DeviceTier, record: Option<&str>) -> &'static str {
    // 手动锁档必须排在最前面，和 decide_gpu() 的优先级保持一致：用户明确选了档位时，
    // 看门狗的历史记录不该再插手，否则两个函数会给出互相矛盾的答案。
    if let Some(manual) = manual {
        return match manual {
            "ultra" | "high" => DeviceTier::High.as_str(),
            "mid" => DeviceTier::Mid.as_str(),
            _ => DeviceTier::Low.as_str(),
        };
    }
    match record {
        // 记录只可能是"降下来"的结果，比检测值更高一律忽略：parse_runtime_record 已经
        // 在读文件时挡掉了，这里再挡一次是为了让"GPU 关着却渲染 high 档"这个最差搭配
        // 不依赖调用方的输入是否干净——它是决策层自己的不变量。
        Some(value) if tier_rank(value) < tier_rank(detected.as_str()) => detected.as_str(),
        // ultra 只可能来自手动锁档，看门狗不会写；写进来也当 high 处理。
        Some("ultra") | Some("high") => DeviceTier::High.as_str(),
        Some("mid") => DeviceTier::Mid.as_str(),
        Some("low") => DeviceTier::Low.as_str(),
        _ => detected.as_str(),
    }
}

/// 供 WebView2 启动参数决策：仅 High 档（或手动锁到 high/ultra）启用 GPU。
pub fn should_enable_gpu_acceleration() -> bool {
    decide_gpu(
        manual_override().as_deref(),
        *detected_tier(),
        runtime_downgraded_tier().as_deref(),
    )
}

/// 返回的是"本次该按哪一档渲染"，不是纯检测值：上次运行看门狗压下去的结果要一起
/// 算进来。否则会出现 CSS 按 high 画、GPU 却因为同一条记录被关掉的组合。
#[tauri::command]
pub fn get_device_tier() -> String {
    decide_tier(
        manual_override().as_deref(),
        *detected_tier(),
        runtime_downgraded_tier().as_deref(),
    )
    .to_string()
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
    use super::{decide_gpu, decide_tier, is_low_voltage_cpu, parse_runtime_record, DeviceTier};

    /// 用户要求「中低机型直接不启用GPU」的七种状态组合。
    /// 元组含义：(手动锁档, 检测值, 降档记录) → (该不该开 GPU, 该按哪一档渲染)
    #[test]
    fn gpu_and_tier_agree_across_states() {
        let cases: &[(Option<&str>, DeviceTier, Option<&str>, bool, &str)] = &[
            // (a)(b) 检测就是中低档 → 不开 GPU
            (None, DeviceTier::Low, None, false, "low"),
            (None, DeviceTier::Mid, None, false, "mid"),
            // (c) 检测 high 且没有降档记录 → 开
            (None, DeviceTier::High, None, true, "high"),
            // (d) 检测 high 但上次实测撑不住 → 不开（本次修的就是这条）
            (None, DeviceTier::High, Some("low"), false, "low"),
            (None, DeviceTier::High, Some("mid"), false, "mid"),
            // 记录说仍是 high（看门狗还没降过）→ 照常开
            (None, DeviceTier::High, Some("high"), true, "high"),
            // (f) 手动锁低档 → 不开，且记录不再插手
            (Some("low"), DeviceTier::High, Some("high"), false, "low"),
            (Some("mid"), DeviceTier::High, Some("low"), false, "mid"),
            // 手动锁 high/ultra → 开；ultra 要收敛成 high 以符合前端类型契约
            (Some("high"), DeviceTier::Low, Some("low"), true, "high"),
            (Some("ultra"), DeviceTier::Low, None, true, "high"),
        ];
        for &(manual, detected, record, want_gpu, want_tier) in cases {
            assert_eq!(
                decide_gpu(manual, detected, record),
                want_gpu,
                "gpu for {manual:?}/{detected:?}/{record:?}"
            );
            assert_eq!(
                decide_tier(manual, detected, record),
                want_tier,
                "tier for {manual:?}/{detected:?}/{record:?}"
            );
        }
    }

    /// 最差搭配是「GPU 关着但档位是 high」——软件渲染 blur(72px)。穷举所有输入组合，
    /// 确认它一次都出不来。
    #[test]
    fn never_pairs_software_rendering_with_high_tier() {
        let manuals = [None, Some("ultra"), Some("high"), Some("mid"), Some("low")];
        let records = [None, Some("ultra"), Some("high"), Some("mid"), Some("low")];
        for manual in manuals {
            for detected in [DeviceTier::High, DeviceTier::Mid, DeviceTier::Low] {
                for record in records {
                    let gpu = decide_gpu(manual, detected, record);
                    let tier = decide_tier(manual, detected, record);
                    assert!(
                        gpu || tier != "high",
                        "GPU 关闭却渲染 high 档：{manual:?}/{detected:?}/{record:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn runtime_record_requires_matching_detected_tier() {
        // 检测值一致才认
        assert_eq!(
            parse_runtime_record("high:low", DeviceTier::High).as_deref(),
            Some("low")
        );
        // 写记录时的检测值和现在不一样 → 作废（换了显卡/插上电源之后要重新评估）
        assert_eq!(parse_runtime_record("mid:low", DeviceTier::High), None);
        // 空白与大小写
        assert_eq!(
            parse_runtime_record("  HIGH : LOW \n", DeviceTier::High).as_deref(),
            Some("low")
        );
        // 记录比检测值更高 → 只可能是手改或写坏，作废
        assert_eq!(parse_runtime_record("mid:high", DeviceTier::Mid), None);
        assert_eq!(parse_runtime_record("low:ultra", DeviceTier::Low), None);
        // 同档位合法
        assert_eq!(
            parse_runtime_record("mid:mid", DeviceTier::Mid).as_deref(),
            Some("mid")
        );
        // 畸形内容
        assert_eq!(parse_runtime_record("", DeviceTier::High), None);
        assert_eq!(parse_runtime_record("high", DeviceTier::High), None);
        assert_eq!(parse_runtime_record("high:", DeviceTier::High), None);
        assert_eq!(parse_runtime_record("high:banana", DeviceTier::High), None);
        assert_eq!(parse_runtime_record(":low", DeviceTier::High), None);
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
