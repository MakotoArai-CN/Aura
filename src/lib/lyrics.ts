export interface LyricLine {
  seconds: number;
  content: string;
  translationFlag: boolean;
  lineNumber: number;
  translations?: string[];
  variants?: LyricVariant[];
}

export interface LyricVariant {
  text: string;
  kind: "translation" | "phonetic";
}

export type LyricVariantMode = "off" | LyricVariant["kind"];

export interface LyricVariantAvailability {
  hasTranslation: boolean;
  hasPhonetic: boolean;
  variantCount?: number;
}

const INLINE_VARIANT_WINDOW_MS = 1300;

const HTML_ENTITIES: Record<string, string> = {
  "&amp;": "&",
  "&lt;": "<",
  "&gt;": ">",
  "&quot;": '"',
  "&#39;": "'",
  "&apos;": "'",
};

function firstText(...values: unknown[]) {
  for (const value of values) {
    const text = normalizeLyricText(value);
    if (text) return text;
  }
  return "";
}

function formatLyricTimestamp(value: unknown) {
  if (value === null || value === undefined || value === "") return "";
  if (typeof value === "string" && /^\d{1,3}:\d{2}(?:\.\d{1,3})?$/.test(value.trim())) {
    return value.trim();
  }
  const seconds = typeof value === "number"
    ? value
    : Number(String(value).trim());
  if (!Number.isFinite(seconds) || seconds < 0) return "";
  const minutes = Math.floor(seconds / 60);
  const rest = seconds - minutes * 60;
  return `${String(minutes).padStart(2, "0")}:${rest.toFixed(2).padStart(5, "0")}`;
}

function normalizeLyricLineObject(value: Record<string, unknown>) {
  const text = firstText(
    value.lineLyric,
    value.lyric,
    value.lrc,
    value.lyrics,
    value.content,
    value.text,
    value.sentence,
    value.words
  );
  if (!text || text === "[object Object]") return "";
  if (/^\[\d{2,}:\d{2}(?:\.\d{1,3})?\]/.test(text)) return text;
  const timestamp = formatLyricTimestamp(value.time ?? value.startTime ?? value.start ?? value.offset ?? value.timestamp);
  return timestamp ? `[${timestamp}]${text}` : text;
}

export function normalizeLyricText(value: unknown): string {
  if (typeof value === "string") {
    const text = value.trim() === "[object Object]" ? "" : value;
    return text;
  }
  if (value === null || value === undefined || typeof value === "boolean") return "";
  if (typeof value === "number") return Number.isFinite(value) ? String(value) : "";
  if (Array.isArray(value)) {
    return value
      .map((item) => normalizeLyricText(item))
      .filter(Boolean)
      .join("\n");
  }
  if (typeof value !== "object") return "";

  const data = value as Record<string, unknown>;
  const lineObjectText = normalizeLyricLineObject(data);
  if (lineObjectText) return lineObjectText;

  const arrayText = firstText(data.lrclist, data.lines, data.list, data.sentences, data.items);
  if (arrayText) return arrayText;

  const direct = firstText(data.lyric, data.lrc, data.lyrics, data.content, data.text, data.data, data.result);
  if (direct) return direct;

  return "";
}

function rpad(s: string, n: number) {
  return s + "0".repeat(Math.max(0, n - s.length));
}

function decodeLyricText(text: string) {
  return text.replace(/&(?:amp|lt|gt|quot|#39|apos);/g, (x) => HTML_ENTITIES[x] ?? x);
}

function parseTimedLine(line: string, dest: LyricLine[], isTranslation: boolean, lineNumber: number) {
  const re = /\[(\d{2,}):(\d{2})(?:\.(\d{1,3}))?\]/g;
  let match: RegExpExecArray | null;
  while ((match = re.exec(line)) !== null) {
    const content = decodeLyricText(line.replace(/\[(\d{2,}):(\d{2})(?:\.(\d{1,3}))?\]/g, ""));
    const ms =
      parseInt(match[1]) * 60000 +
      parseInt(match[2]) * 1000 +
      (match[3] ? parseInt(rpad(match[3], 3)) : 0);
    dest.push({ seconds: ms, content, translationFlag: isTranslation, lineNumber });
  }
}

export function parseLyric(lyric: unknown, tlyric: unknown = ""): LyricLine[] {
  const lyricText = normalizeLyricText(lyric);
  const translatedLyricText = normalizeLyricText(tlyric);
  const lines: LyricLine[] = [];
  const translations: LyricLine[] = [];

  lyricText.split("\n").forEach((line, i) => parseTimedLine(line, lines, false, i));
  if (translatedLyricText) translatedLyricText.split("\n").forEach((line, i) => parseTimedLine(line, translations, true, i));

  if (!lines.length && lyricText.trim()) {
    lyricText
      .split(/\r?\n/)
      .map((line) => decodeLyricText(line.trim()))
      .filter(Boolean)
      .forEach((content, i) => lines.push({ seconds: i * 4000, content, translationFlag: false, lineNumber: i }));
  }

  const orderedRawLines = lines
    .sort((a, b) =>
      a.seconds !== b.seconds
        ? a.seconds - b.seconds
        : a.lineNumber - b.lineNumber
    );

  const primaryBySecond = new Map<number, LyricLine>();
  const inlineVariantsBySecond = new Map<number, LyricVariant[]>();
  let previousPrimary: LyricLine | null = null;
  orderedRawLines.forEach((line) => {
    const content = line.content.trim();
    if (!content) return;
    const existing = primaryBySecond.get(line.seconds);
    if (!existing) {
      if (
        previousPrimary &&
        line.seconds > previousPrimary.seconds &&
        line.seconds - previousPrimary.seconds <= INLINE_VARIANT_WINDOW_MS &&
        isLikelyInlineVariant(previousPrimary.content, content)
      ) {
        addInlineVariant(inlineVariantsBySecond, previousPrimary.seconds, {
          text: content,
          kind: detectVariantKind(previousPrimary.content, content),
        });
        return;
      }
      const primary = { ...line, content, translationFlag: false };
      primaryBySecond.set(line.seconds, primary);
      previousPrimary = primary;
      return;
    }
    // 同一时间戳的后续行：仅当确为双语/音译时折叠为变体；
    // 否则（如同在 [00:00] 的 作词/作曲/编曲 等元数据）保留为独立歌词行，
    // 微调时间戳错开，避免被误当成上一行的译文。
    if (isLikelyInlineVariant(existing.content, content)) {
      addInlineVariant(inlineVariantsBySecond, line.seconds, {
        text: content,
        kind: detectVariantKind(existing.content, content),
      });
      return;
    }
    let slot = line.seconds;
    while (primaryBySecond.has(slot)) slot += 1;
    const primary = { ...line, seconds: slot, content, translationFlag: false };
    primaryBySecond.set(slot, primary);
    previousPrimary = primary;
  });

  const orderedLines = Array.from(primaryBySecond.values())
    .sort((a, b) => a.seconds - b.seconds)
    .map((line, i) => ({ ...line, translationFlag: false, lineNumber: i }));

  const variantsBySecond = new Map<number, LyricVariant[]>();
  translations.forEach((line) => {
    const content = line.content.trim();
    if (!content) return;
    const primary = primaryBySecond.get(line.seconds)?.content ?? "";
    const existing = variantsBySecond.get(line.seconds) ?? [];
    if (!existing.some((item) => item.text === content)) {
      existing.push({ text: content, kind: detectVariantKind(primary, content) });
    }
    variantsBySecond.set(line.seconds, existing);
  });

  return orderedLines.map((line) => ({
    ...line,
    translations: [
      ...(variantsBySecond.get(line.seconds) ?? []),
      ...(inlineVariantsBySecond.get(line.seconds) ?? []),
    ].map((item) => item.text),
    variants: [
      ...(variantsBySecond.get(line.seconds) ?? []),
      ...(inlineVariantsBySecond.get(line.seconds) ?? []),
    ],
  }));
}

function addInlineVariant(target: Map<number, LyricVariant[]>, seconds: number, variant: LyricVariant) {
  const variants = target.get(seconds) ?? [];
  if (!variants.some((item) => item.text === variant.text)) variants.push(variant);
  target.set(seconds, variants);
}

function detectVariantKind(primary: string, text: string): LyricVariant["kind"] {
  const hasCjkPrimary = /[\u3400-\u9fff\u3040-\u30ff\uac00-\ud7af]/.test(primary);
  const latinLetters = (text.match(/[A-Za-z]/g) ?? []).length;
  const cjkLetters = (text.match(/[\u3400-\u9fff\u3040-\u30ff\uac00-\ud7af]/g) ?? []).length;
  const isMostlyLatin = latinLetters >= 3 && latinLetters >= cjkLetters * 2;
  return hasCjkPrimary && isMostlyLatin ? "phonetic" : "translation";
}

function isLikelyInlineVariant(primary: string, text: string): boolean {
  if (!primary.trim() || !text.trim() || primary.trim() === text.trim()) return false;

  const primaryHasKana = /[\u3040-\u30ff]/.test(primary);
  const textHasKana = /[\u3040-\u30ff]/.test(text);
  const primaryHasHangul = /[\uac00-\ud7af]/.test(primary);
  const textHasHangul = /[\uac00-\ud7af]/.test(text);
  const primaryLatin = (primary.match(/[A-Za-z]/g) ?? []).length;
  const textLatin = (text.match(/[A-Za-z]/g) ?? []).length;
  const primaryCjk = (primary.match(/[\u3400-\u9fff]/g) ?? []).length;
  const textCjk = (text.match(/[\u3400-\u9fff]/g) ?? []).length;

  if ((primaryHasKana || primaryHasHangul) && textCjk > 0 && !textHasKana && !textHasHangul) return true;
  if (primaryLatin >= 3 && textCjk >= 2 && textLatin < primaryLatin) return true;
  if (primaryCjk >= 2 && textLatin >= 3 && !primaryHasKana && !primaryHasHangul) return true;
  return false;
}

function normalizeVariantMode(showTranslation: boolean | LyricVariantMode): LyricVariantMode | "all" {
  if (showTranslation === false || showTranslation === "off") return "off";
  if (showTranslation === true) return "all";
  return showTranslation;
}

export function getLineVariants(line: LyricLine | null | undefined): LyricVariant[] {
  return line?.variants?.length
    ? line.variants
    : line?.translations?.map((text) => ({ text, kind: "translation" as const })) ?? [];
}

export function getLineVariantAvailability(line: LyricLine | null | undefined) {
  const variants = getLineVariants(line);
  return {
    hasTranslation: variants.some((variant) => variant.kind === "translation"),
    hasPhonetic: variants.some((variant) => variant.kind === "phonetic"),
    variantCount: variants.length,
  };
}

export function getLyricsVariantAvailability(lines: LyricLine[]) {
  let hasTranslation = false;
  let hasPhonetic = false;
  let variantCount = 0;
  for (const line of lines) {
    const availability = getLineVariantAvailability(line);
    hasTranslation = hasTranslation || availability.hasTranslation;
    hasPhonetic = hasPhonetic || availability.hasPhonetic;
    variantCount += availability.variantCount;
    if (hasTranslation && hasPhonetic && variantCount > 0) break;
  }
  return { hasTranslation, hasPhonetic, variantCount };
}

export function getAvailableLyricVariantModes(availability: LyricVariantAvailability): LyricVariantMode[] {
  const modes: LyricVariantMode[] = [
    ...(availability.hasTranslation ? ["translation" as const] : []),
    ...(availability.hasPhonetic ? ["phonetic" as const] : []),
    "off",
  ];
  return modes;
}

export function normalizeLyricVariantMode(mode: LyricVariantMode | undefined, availability: LyricVariantAvailability): LyricVariantMode {
  const modes = getAvailableLyricVariantModes(availability);
  if (mode && modes.includes(mode)) return mode;
  return modes[0] ?? "off";
}

export function getNextLyricVariantMode(mode: LyricVariantMode, availability: LyricVariantAvailability): LyricVariantMode {
  const modes = getAvailableLyricVariantModes(availability);
  if (modes.length <= 1) return "off";
  const normalized = normalizeLyricVariantMode(mode, availability);
  const currentIndex = modes.indexOf(normalized);
  return modes[(currentIndex + 1) % modes.length] ?? modes[0] ?? "off";
}

export function isLyricVariantModeActive(mode: LyricVariantMode, availability: LyricVariantAvailability): boolean {
  return (
    (mode === "translation" && availability.hasTranslation) ||
    (mode === "phonetic" && availability.hasPhonetic)
  );
}

export function lyricVariantButtonLabel(mode: LyricVariantMode, availability: LyricVariantAvailability, bracketed = false) {
  const label = mode === "phonetic" || (!availability.hasTranslation && availability.hasPhonetic) ? "音" : "译";
  return bracketed ? `[${label}]` : label;
}

export function lyricVariantButtonTitle(mode: LyricVariantMode, availability: LyricVariantAvailability) {
  if (!availability.hasTranslation && !availability.hasPhonetic) return "无译文或音标";
  if (mode === "translation") return availability.hasPhonetic ? "切换到音标" : "关闭译文";
  if (mode === "phonetic") return "关闭音标";
  return availability.hasTranslation ? "显示译文" : "显示音标";
}

export function getLineTranslation(line: LyricLine | null | undefined, translationIndex = 0, showTranslation: boolean | LyricVariantMode = true) {
  return getLineVariant(line, translationIndex, showTranslation)?.text ?? "";
}

export function getLineVariant(line: LyricLine | null | undefined, translationIndex = 0, showTranslation: boolean | LyricVariantMode = true): LyricVariant | null {
  const variants = line?.variants?.length
    ? line.variants
    : line?.translations?.map((text) => ({ text, kind: "translation" as const })) ?? [];
  const mode = normalizeVariantMode(showTranslation);
  if (mode === "off" || !variants.length) return null;
  const candidates = mode === "all" ? variants : variants.filter((variant) => variant.kind === mode);
  if (!candidates.length) return null;
  const index = ((translationIndex % candidates.length) + candidates.length) % candidates.length;
  return candidates[index] ?? null;
}

export function getActiveLyricPayload(lines: LyricLine[], positionSeconds: number, showTranslation: boolean | LyricVariantMode = true, translationIndex = 0) {
  if (!lines.length) return null;
  const posMs = positionSeconds * 1000;
  let idx = -1;
  for (let i = 0; i < lines.length; i++) {
    if (!lines[i]) continue;
    if (lines[i].seconds <= posMs) idx = i;
    else break;
  }
  const line = idx >= 0 ? lines[idx] : null;
  if (!line || line.translationFlag) return null;
  const variant = getLineVariant(line, translationIndex, showTranslation);
  const availability = getLyricsVariantAvailability(lines);
  const mode = normalizeVariantMode(showTranslation);
  const payloadMode: LyricVariantMode = mode === "all" ? variant?.kind ?? "translation" : mode;
  return {
    index: idx,
    lyric: line.content,
    tlyric: variant?.text ?? "",
    variantKind: variant?.kind ?? (mode === "phonetic" ? "phonetic" : "translation"),
    variantMode: payloadMode,
    hasTranslation: availability.hasTranslation,
    hasPhonetic: availability.hasPhonetic,
    translationCount: line.variants?.length ?? line.translations?.length ?? 0,
  };
}

export function getActiveTwoLineLyricPayload(lines: LyricLine[], positionSeconds: number, showTranslation: boolean | LyricVariantMode = true, translationIndex = 0) {
  const active = getActiveLyricPayload(lines, positionSeconds, showTranslation, translationIndex);
  if (!active) return null;

  let nextIndex = -1;
  for (let i = active.index + 1; i < lines.length; i++) {
    if (lines[i] && !lines[i].translationFlag && lines[i].content.trim()) {
      nextIndex = i;
      break;
    }
  }

  const nextLine = nextIndex >= 0 ? lines[nextIndex] : null;

  return {
    ...active,
    nextLyric: nextLine?.content ?? "",
    nextTlyric: getLineVariant(nextLine, translationIndex, showTranslation)?.text ?? "",
    nextVariantKind: getLineVariant(nextLine, translationIndex, showTranslation)?.kind ?? "translation",
    nextTranslationCount: nextLine?.variants?.length ?? nextLine?.translations?.length ?? 0,
  };
}
