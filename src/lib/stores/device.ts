import { writable } from "svelte/store";

export type DeviceTier = "high" | "mid" | "low";

/**
 * 设备性能档位（Rust 启动时检测一次）：
 * high — 高性能 CPU + 独显：GPU 加速 + 完整视觉效果（毛玻璃/模糊背景）。
 * mid  — 高性能 CPU 无独显：软件渲染，禁用常驻模糊。
 * low  — 中低端 CPU：纯色 UI，禁用全部透明/模糊合成效果。
 */
export const deviceTier = writable<DeviceTier>("mid");

export async function initDeviceTier(): Promise<DeviceTier> {
  try {
    const { getDeviceTier } = await import("../tauri");
    const tier = (await getDeviceTier()) as DeviceTier;
    if (tier === "high" || tier === "mid" || tier === "low") {
      deviceTier.set(tier);
      return tier;
    }
  } catch {
    // 非桌面运行时或检测失败，保持默认 mid（软件渲染，兼容性最好）。
  }
  return "mid";
}
