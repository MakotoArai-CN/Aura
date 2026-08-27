/**
 * 运行时 CPU 看门狗：启动后持续采样「本进程 + 它派生的 WebView 进程」的 CPU 占用，
 * 持续超过 10% 就把视觉效果降一档。
 *
 * 几条刻意的取舍：
 * - 只降不升。升回去会让刚好卡在阈值附近的机器反复升降，观感比一直低更差；
 *   降档结果只在本次运行内有效，下次启动重新检测。
 * - 逐档降，且要求连续多次超标。换歌解码、封面取色、首屏渲染都会造成单次尖峰，
 *   一次超标就降会误伤。
 * - 用户手动锁档后彻底不插手（连采样都不做，省掉 Rust 侧的进程遍历开销）。
 * - 降到最低档就自行停止，没有继续采样的意义。
 */
import { get } from "svelte/store";
import { applyRuntimeDowngrade, deviceTier } from "./stores/device";
import { settings } from "./stores/settings";
import { getResourceUsage, isTauriRuntime } from "./tauri";

/** 启动后先让首屏渲染和首次解码的峰值过去，再开始采样。 */
const WARMUP_MS = 25_000;
/** 采样间隔。Rust 侧每次要遍历两遍进程表并阻塞 220ms，不能密集轮询。 */
const SAMPLE_MS = 12_000;
/** 阈值：百分比，已按逻辑核心数归一化（和设置页显示的是同一个数）。 */
const CPU_THRESHOLD = 10;
/** 连续几次超标才降一档。 */
const BREACH_STREAK = 3;
/** 降档后的冷静期：等新档位的 CSS 生效、合成层重建完再重新计数。 */
const COOLDOWN_MS = 45_000;

let timer: ReturnType<typeof setTimeout> | null = null;
let breachStreak = 0;
let started = false;

function schedule(delay: number) {
  if (timer !== null) clearTimeout(timer);
  timer = setTimeout(() => {
    timer = null;
    void tick();
  }, delay);
}

function stop() {
  if (timer !== null) clearTimeout(timer);
  timer = null;
}

async function tick() {
  // 手动锁档期间不采样也不停表：用户可能再切回自动。
  if (get(settings).effectTier !== "auto") {
    breachStreak = 0;
    schedule(SAMPLE_MS);
    return;
  }

  if (get(deviceTier) === "low") {
    stop();
    return;
  }

  let cpu: number;
  try {
    cpu = (await getResourceUsage()).cpu_percent;
  } catch {
    // 采样失败（命令未注册、权限受限）就当这一轮没发生，不要据此降档。
    schedule(SAMPLE_MS);
    return;
  }

  if (cpu <= CPU_THRESHOLD) {
    breachStreak = 0;
    schedule(SAMPLE_MS);
    return;
  }

  breachStreak += 1;
  if (breachStreak < BREACH_STREAK) {
    schedule(SAMPLE_MS);
    return;
  }

  breachStreak = 0;
  const next = applyRuntimeDowngrade();
  console.info(`[effectWatchdog] CPU ${cpu.toFixed(1)}% 持续超标，视觉效果降至 ${next}`);
  if (next === "low") {
    stop();
    return;
  }
  schedule(COOLDOWN_MS);
}

/** 只在桌面/移动运行时启动；浏览器里没有进程概念，直接不启动。 */
export function startEffectWatchdog() {
  if (started || !isTauriRuntime()) return;
  started = true;
  schedule(WARMUP_MS);
}
