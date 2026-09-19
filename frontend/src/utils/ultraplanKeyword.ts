/**
 * UltraPlan / UltraReview keyword detection.
 * Ported from Claude Code's utils/ultraplan/keyword.ts logic.
 * Skips false positives: quoted strings, paths, hyphenated words,
 * file extensions, question forms, and slash commands.
 */

export interface KeywordMatch {
  word: string;
  start: number;
  end: number;
  type: 'ultraplan' | 'ultrareview';
}

const ULTRAPLAN_RE = /\bultraplan\b/gi;
const ULTRAREVIEW_RE = /\bultrareview\b/gi;

function isFalsePositive(text: string, start: number, end: number): boolean {
  // Inside quotes
  const before = text.slice(0, start);
  const singleOpen = (before.match(/'/g) || []).length % 2;
  const doubleOpen = (before.match(/"/g) || []).length % 2;
  if (singleOpen || doubleOpen) return true;

  // Part of a path (preceded by / or .)
  const charBefore = start > 0 ? text[start - 1] : '';
  const charAfter = end < text.length ? text[end] : '';
  if (charBefore === '/' || charBefore === '.') return true;

  // Hyphenated word (preceded or followed by -)
  if (charBefore === '-' || charAfter === '-') return true;

  // File extension (followed by .)
  if (charAfter === '.') return true;

  // Slash command (line starts with /)
  const lineStart = text.lastIndexOf('\n', start - 1) + 1;
  const linePrefix = text.slice(lineStart, start).trim();
  if (linePrefix === '/' || linePrefix.startsWith('/')) return true;

  // Question form ("ultraplan?")
  if (charAfter === '?') return true;

  return false;
}

export function findKeywordMatches(text: string): KeywordMatch[] {
  const matches: KeywordMatch[] = [];

  for (const re of [ULTRAPLAN_RE, ULTRAREVIEW_RE]) {
    re.lastIndex = 0;
    let m: RegExpExecArray | null;
    while ((m = re.exec(text)) !== null) {
      const start = m.index;
      const end = start + m[0].length;
      if (!isFalsePositive(text, start, end)) {
        matches.push({
          word: m[0],
          start,
          end,
          type: m[0].toLowerCase().startsWith('ultrareview') ? 'ultrareview' : 'ultraplan',
        });
      }
    }
  }

  return matches.sort((a, b) => a.start - b.start);
}

export function hasUltraplanKeyword(text: string): boolean {
  return findKeywordMatches(text).some(m => m.type === 'ultraplan');
}

export function hasUltrareviewKeyword(text: string): boolean {
  return findKeywordMatches(text).some(m => m.type === 'ultrareview');
}

/** Strip the first ultraplan/ultrareview keyword from text, trimming extra whitespace. */
export function stripKeyword(text: string): string {
  const matches = findKeywordMatches(text);
  if (!matches.length) return text;
  const { start, end } = matches[0];
  return (text.slice(0, start) + text.slice(end)).replace(/\s{2,}/g, ' ').trim();
}

/** Detect which mode a message is in (first match wins). */
export function detectPlanningMode(text: string): 'ultraplan' | 'ultrareview' | null {
  const matches = findKeywordMatches(text);
  return matches[0]?.type ?? null;
}
