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
 * 档位只管 CSS。是否给 WebView 开 GPU 加速由 `enableGpuAcceleration` 独立控制，
 * 和这里的检测结果没有任何关系。
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

/**
 * 有效档位：手动选了就用手动的（看门狗与检测都不再插手），"auto" 才走
 * 「检测结果 - 已降档数」。名字保持 deviceTier 不变，消费方无需改动。
 */
export const deviceTier = derived(
  [detectedTier, runtimeDowngrade, settings],
  ([$detected, $downgrade, $settings]): VisualTier => {
    const manual = $settings.effectTier;
    if (manual !== "auto") return manual;
    return stepDown($detected, $downgrade);
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
 * 本次启动**实际**生效的 GPU 加速状态，取自 Rust 侧落盘的开关文件。
 *
 * 之所以不直接用 `$settings.enableGpuAcceleration`：那是"下一次启动想要的值"，
 * 用户刚拨动开关时两者就不一致了。分开存才能诚实地告诉用户"要重启才生效"，
 * 而不是拨完就装作已经生效。
 */
export const activeGpuAcceleration = writable<boolean>(false);

/**
 * 读回本次启动生效的 GPU 开关，然后把设置里的值回写一遍。
 *
 * 回写是自愈：开关文件可能因为升级、清理用户目录而消失，而 WebView2 的
 * `--disable-gpu` 参数只认这个文件，不回写的话用户开过的 GPU 会凭空关掉。
 * 顺序必须是先读后写——写完再读只能读到自己刚写的值。
 */
export async function initGpuAcceleration(): Promise<void> {
  try {
    const { getGpuAcceleration, setGpuAcceleration } = await import("../tauri");
    activeGpuAcceleration.set(await getGpuAcceleration());
    await setGpuAcceleration(get(settings).enableGpuAcceleration);
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
