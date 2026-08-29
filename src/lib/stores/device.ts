import { derived, get, writable } from "svelte/store";
import { settings } from "./settings";

/** Rust 硬件检测的输出档位。自动策略最高只到 high。 */
export type DeviceTier = "high" | "mid" | "low";

/** 实际写进 `data-visual` 的档位。ultra 只能由用户手动选。 */
export type VisualTier = "ultra" | DeviceTier;

/** 从好到差。降档就是朝数组末尾走。 */
const TIER_ORDER: VisualTier[] = ["ultra", "high", "mid", "low"];

/**
 * 设备性能档位（Rust 启动时检测一次）：
 * high — 高性能 CPU + 独显：完整视觉效果（毛玻璃/模糊背景）。
 * mid  — 高性能 CPU 无独显：禁用常驻模糊。
 * low  — 中低端 CPU / 低压 U 无独显 / 低配安卓：纯色 UI，禁用全部透明模糊合成。
 *
 * 档位本身只写 CSS 变量。同一份检测结果还兼任 GPU 加速在**自动模式**下的判据
 * （Rust 侧 `should_enable_gpu_acceleration`：只有 high 才开），用户手动选的
 * `on`/`off` 优先于检测。
 */
export const detectedTier = writable<DeviceTier>("mid");

/**
 * 运行时 CPU 看门狗累计降了几档。只增不减：一次运行内降下去就不再升回来
 * （升回去会在「刚好卡在阈值上」的机器上来回抖动，观感比一直低更差），
 * 下次启动重新检测。手动锁定档位时看门狗不写这里。
 */
export const runtimeDowngrade = writable<number>(0);

function stepDown(tier: VisualTier, steps: number): VisualTier {
  const from = TIER_ORDER.indexOf(tier);
  const index = Math.min(TIER_ORDER.length - 1, (from < 0 ? 1 : from) + Math.max(0, steps));
  return TIER_ORDER[index];
}

/** 两个档位里更差的那个。TIER_ORDER 是好→差，所以下标大的更差。 */
function worseOf(a: VisualTier, b: VisualTier): VisualTier {
  const ia = TIER_ORDER.indexOf(a);
  const ib = TIER_ORDER.indexOf(b);
  return ib > ia ? b : a;
}

/**
 * 本次启动**实际**生效的 GPU 加速状态。
 *
 * 之所以不直接用 `$settings.gpuAcceleration`：那是"下一次启动想要的模式"，而且它
 * 可能是 `auto`——auto 的生效结果只有 Rust 侧的硬件检测知道。分开存才能诚实地告诉
 * 用户"要重启才生效"，也才能在自动模式下显示"检测结果到底开没开"。
 *
 * 初值取 Rust 在页面脚本最前面注入的 `__AURA_GPU_ACTIVE__`（见 lib.rs 的
 * `gpu_active_plugin`），**必须同步拿到**：下面的 `deviceTier` 要用它决定视觉档位，
 * 而 async 读回会让启动头几十毫秒先按"没有 GPU"渲染一遍再翻回来，肉眼能看到一次
 * 观感跳变。非 Tauri 运行时（浏览器里直接开 dist）读不到这个全局，退回 false，
 * 也就是按"没有硬件合成"处理——那正是浏览器里跑一个桌面壳最保守的假设。
 */
function injectedGpuActive(): boolean {
  if (typeof window === "undefined") return false;
  return (window as Window & { __AURA_GPU_ACTIVE__?: boolean }).__AURA_GPU_ACTIVE__ === true;
}

export const activeGpuAcceleration = writable<boolean>(injectedGpuActive());

/**
 * 有效档位：手动选了就用手动的（看门狗与检测都不再插手），"auto" 才走
 * 「检测结果 - 已降档数」。名字保持 deviceTier 不变，消费方无需改动。
 *
 * "auto" 且本次启动没有 GPU 时，档位**不优于 mid**。没有硬件合成时那些常驻滤镜
 * 全落在 CPU 上每帧重跑：`--visual-backdrop: saturate(180%) blur(20px)` 加
 * `LiquidGlassSurface` 的 `backdrop-filter: url(#SVG位移滤镜)`，实测 gpu-process
 * 常驻 66% 一个核，同一台机器把 GPU 打开后总 CPU 掉到 0.3~2.2%。
 *
 * GPU 现在默认就是 auto（跟着同一套硬件检测走），所以"判定高性能却在软件渲染"这个
 * 组合只剩一条来路：用户手动把 GPU 关掉。那是他的选择，但效果档位得跟着务实——
 * 一边关掉硬件合成、一边让 CPU 扛全屏模糊没有道理。
 *
 * 用"不优于 mid"而不是"等于 mid"：检测出 low 的机器不能因为没开 GPU 反而被**升**到
 * mid 去拿更重的效果。看门狗的降档结果同理，仍然能继续往 low 走。
 *
 * 选 mid 不选直接 low：mid 让 `--visual-backdrop` 回到 `:root` 的 `none`（底栏的常驻
 * 毛玻璃没了）、`--glass-blur` 与 `--nav-backdrop-filter` 也是 none，而液态玻璃表面只在
 * high/ultra 启用，所以 `url()` 位移滤镜一并消失；交互动效则一个不少。low 会连图层
 * 提升和阴影一起砍掉，为了省 CPU 一步降到那儿没有必要。
 *
 * 手动锁了 high/ultra 的人是明确要那个效果，不替他降——他知道自己在换什么。
 */
export const deviceTier = derived(
  [detectedTier, runtimeDowngrade, settings, activeGpuAcceleration],
  ([$detected, $downgrade, $settings, $gpuActive]): VisualTier => {
    const manual = $settings.effectTier;
    if (manual !== "auto") return manual;
    // 先把"没有 GPU"这个封顶压在**基准档**上，再让看门狗从那里往下降。
    // 反过来（先降档、最后封顶）会白吃一轮降档：high 机器降一档到 mid，封顶后还是
    // mid，看门狗以为自己降成功了，得再等一轮冷静期才真的动到 low。
    const base = $gpuActive ? $detected : worseOf($detected, "mid");
    return stepDown(base, $downgrade);
  },
);

/** 看门狗调用：再降一档。已经手动锁档或已经到底时什么都不做。 */
export function applyRuntimeDowngrade(): VisualTier {
  const current = get(deviceTier);
  if (get(settings).effectTier !== "auto" || current === "low") return current;
  runtimeDowngrade.update((n) => n + 1);
  return get(deviceTier);
}

/**
 * 主窗口是否**不可见**。由 Rust 的 `main-minimized` 事件推过来。
 *
 * 名字里的 minimized 是历史包袱，实际语义是"没人看得见"：最小化、关闭到托盘
 * （`hide()`，此时 `is_minimized()` 仍是 false）都算。WebView2 在无边框透明窗口上
 * 不做遮挡检测，隐藏的窗口背后动画照旧合成，所以两种情况要一视同仁。
 *
 * 不用 `document.visibilityState`：WebView2 在窗口最小化时不保证把页面标成 hidden，
 * 从任务栏最小化也绕过自定义标题栏的那个按钮。Rust 侧才有唯一靠得住的信号，
 * 且只在状态真的翻转时才发事件，所以这里不会被拖动窗口刷爆。
 *
 * 消费方的原则是"看不见就别干活"：暂停持续动画、跳过 CPU 采样。**不包括**播放本身
 * 和进度计时——最小化听歌是最正常的用法。
 */
export const windowMinimized = writable<boolean>(false);

/**
 * 读回本次启动生效的 GPU 开关，然后把设置里的值回写一遍。
 *
 * 读回是兜底：正常情况下注入的 `__AURA_GPU_ACTIVE__` 已经给出同一个值（见
 * `activeGpuAcceleration`），这里再问一次只为覆盖注入失败的场景，`get_gpu_acceleration`
 * 返回的是同一个启动快照，两者不会打架。
 *
 * 回写是自愈：开关文件可能因为升级、清理用户目录而消失，而 WebView2 的
 * `--disable-gpu` 参数只认这个文件，不回写的话用户开过的 GPU 会凭空关掉。
 * 顺序必须是先读后写——写完再读只能读到自己刚写的值。
 */
export async function initGpuAcceleration(): Promise<void> {
  try {
    const { getGpuAcceleration, setGpuAccelerationMode } = await import("../tauri");
    activeGpuAcceleration.set(await getGpuAcceleration());
    await setGpuAccelerationMode(get(settings).gpuAcceleration);
  } catch (error) {
    // 非桌面运行时没有这个概念；读写失败也只是下次启动仍用旧值，不该打断启动。
    console.warn("[device] 同步 GPU 加速开关失败", error);
  }
}

export async function initDeviceTier(): Promise<DeviceTier> {
  try {
    const { getDeviceTier } = await import("../tauri");
    const tier = (await getDeviceTier()) as DeviceTier;
    if (tier === "high" || tier === "mid" || tier === "low") {
      detectedTier.set(tier);
      return tier;
    }
  } catch {
    // 非桌面运行时或检测失败，保持默认 mid（软件渲染，兼容性最好）。
  }
  return "mid";
}
