export interface TypedRevealOptions {
  charsPerSecond?: number;
  minStep?: number;
  maxStep?: number;
  snapBacklog?: number;
}

export interface RevealFrameInput extends Required<TypedRevealOptions> {
  currentLength: number;
  targetLength: number;
  deltaMs: number;
}

export const DEFAULT_TYPED_REVEAL: Required<TypedRevealOptions> = {
  charsPerSecond: 900,
  minStep: 1,
  maxStep: 96,
  snapBacklog: 2400,
};

export function normalizeTypedRevealOptions(options: TypedRevealOptions = {}): Required<TypedRevealOptions> {
  return {
    charsPerSecond: Math.max(1, options.charsPerSecond ?? DEFAULT_TYPED_REVEAL.charsPerSecond),
    minStep: Math.max(1, options.minStep ?? DEFAULT_TYPED_REVEAL.minStep),
    maxStep: Math.max(1, options.maxStep ?? DEFAULT_TYPED_REVEAL.maxStep),
    snapBacklog: Math.max(1, options.snapBacklog ?? DEFAULT_TYPED_REVEAL.snapBacklog),
  };
}

export function shouldSnapTypedReveal(currentLength: number, targetLength: number, snapBacklog: number): boolean {
  return Math.max(0, targetLength - currentLength) >= snapBacklog;
}

export function nextTypedRevealLength(input: RevealFrameInput): number {
  const { currentLength, targetLength, deltaMs, charsPerSecond, minStep, maxStep, snapBacklog } = input;
  if (currentLength >= targetLength) return targetLength;
  if (shouldSnapTypedReveal(currentLength, targetLength, snapBacklog)) return targetLength;

  const remaining = targetLength - currentLength;
  const timeStep = Math.ceil((charsPerSecond * Math.max(1, deltaMs)) / 1000);
  const adaptiveStep = Math.ceil(remaining * 0.08);
  const step = Math.min(maxStep, Math.max(minStep, timeStep, adaptiveStep));
  return Math.min(targetLength, currentLength + step);
}

export function clampRevealLength(currentLength: number, targetLength: number): number {
  if (currentLength < 0) return 0;
  return Math.min(currentLength, Math.max(0, targetLength));
}

function countFenceMarkers(markdown: string): number {
  return markdown.split('\n').filter(line => /^\s*(```|~~~)/.test(line)).length;
}

function countUnescapedBackticks(markdown: string): number {
  let count = 0;
  for (let index = 0; index < markdown.length; index += 1) {
    if (markdown[index] !== '`') continue;
    let backslashes = 0;
    for (let cursor = index - 1; cursor >= 0 && markdown[cursor] === '\\'; cursor -= 1) {
      backslashes += 1;
    }
    if (backslashes % 2 === 0) count += 1;
  }
  return count;
}

function stripFencedCode(markdown: string): string {
  return markdown.replace(/(^|\n)\s*(```|~~~)[\s\S]*?(\n\s*\2)/g, '\n');
}

export function stabilizePartialMarkdown(markdown: string): string {
  let output = markdown;
  if (countFenceMarkers(output) % 2 === 1) {
    output += output.endsWith('\n') ? '```' : '\n```';
  }
  if (countUnescapedBackticks(stripFencedCode(output)) % 2 === 1) {
    output += '`';
  }
  return output;
}
