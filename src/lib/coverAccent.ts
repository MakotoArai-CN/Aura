// 从封面图提取主题色，供底部播放器 / 沉浸式播放器动态配色复用。
// 主色 = 加权平均后的鲜明色；副色 = 主色色相偏移得到的和谐配色，用于多色渐变。

export type Rgb = { r: number; g: number; b: number };

function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, value));
}

export function rgbToHsl(r: number, g: number, b: number): [number, number, number] {
  r /= 255; g /= 255; b /= 255;
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  let h = 0;
  let s = 0;
  const l = (max + min) / 2;
  if (max !== min) {
    const d = max - min;
    s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
    if (max === r) h = (g - b) / d + (g < b ? 6 : 0);
    else if (max === g) h = (b - r) / d + 2;
    else h = (r - g) / d + 4;
    h /= 6;
  }
  return [h, s, l];
}

export function hslToRgb(h: number, s: number, l: number): Rgb {
  const hue2rgb = (p: number, q: number, t: number) => {
    if (t < 0) t += 1;
    if (t > 1) t -= 1;
    if (t < 1 / 6) return p + (q - p) * 6 * t;
    if (t < 1 / 2) return q;
    if (t < 2 / 3) return p + (q - p) * (2 / 3 - t) * 6;
    return p;
  };
  if (s === 0) {
    const v = Math.round(l * 255);
    return { r: v, g: v, b: v };
  }
  const q = l < 0.5 ? l * (1 + s) : l + s - l * s;
  const p = 2 * l - q;
  return {
    r: Math.round(hue2rgb(p, q, h + 1 / 3) * 255),
    g: Math.round(hue2rgb(p, q, h) * 255),
    b: Math.round(hue2rgb(p, q, h - 1 / 3) * 255),
  };
}

/** 加权平均出封面的主色（鲜明、亮度适中）。返回 null 表示取色失败。 */
export async function extractAccentFromCover(url: string): Promise<Rgb | null> {
  if (!url) return null;
  const image = new Image();
  image.crossOrigin = "anonymous";
  image.decoding = "async";
  const loaded = new Promise<void>((resolve, reject) => {
    image.onload = () => resolve();
    image.onerror = () => reject(new Error("cover image load failed"));
  });
  image.src = url;
  await loaded;

  const canvas = document.createElement("canvas");
  const size = 32;
  canvas.width = size;
  canvas.height = size;
  const ctx = canvas.getContext("2d", { willReadFrequently: true });
  if (!ctx) return null;
  ctx.drawImage(image, 0, 0, size, size);
  const data = ctx.getImageData(0, 0, size, size).data;

  let total = 0;
  let r = 0;
  let g = 0;
  let b = 0;
  for (let i = 0; i < data.length; i += 4) {
    const alpha = data[i + 3];
    if (alpha < 180) continue;
    const pr = data[i];
    const pg = data[i + 1];
    const pb = data[i + 2];
    const [, s, l] = rgbToHsl(pr, pg, pb);
    if (l < 0.08 || l > 0.94) continue;
    const weight = 0.2 + s * 1.8 + (0.5 - Math.abs(l - 0.52)) * 0.7;
    total += weight;
    r += pr * weight;
    g += pg * weight;
    b += pb * weight;
  }
  if (total <= 0) return null;
  const [h, s, l] = rgbToHsl(r / total, g / total, b / total);
  return hslToRgb(h, clamp(Math.max(s, 0.46), 0.36, 0.78), clamp(l, 0.38, 0.58));
}

/** 由主色派生一个色相偏移的和谐副色，用于沉浸式播放器的多色渐变。 */
export function deriveSecondaryAccent(primary: Rgb, hueShift = 0.12): Rgb {
  const [h, s, l] = rgbToHsl(primary.r, primary.g, primary.b);
  const nextH = (h + hueShift + 1) % 1;
  return hslToRgb(nextH, clamp(s * 0.92, 0.34, 0.8), clamp(l + 0.06, 0.4, 0.64));
}

/** 从封面同时得到主色 + 副色（多色渐变用）。取色失败返回 null。 */
export async function extractAccentPair(url: string): Promise<{ primary: Rgb; secondary: Rgb } | null> {
  const primary = await extractAccentFromCover(url);
  if (!primary) return null;
  return { primary, secondary: deriveSecondaryAccent(primary) };
}
