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
 * high — 高性能 CPU + 独显：GPU 加速 + 完整视觉效果（毛玻璃/模糊背景）。
 * mid  — 高性能 CPU 无独显：软件渲染，禁用常驻模糊。
 * low  — 中低端 CPU / 低压 U 无独显 / 低配安卓：纯色 UI，禁用全部透明模糊合成。
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
