import { languageForPath } from "../../lib/monacoPresentation";

export type EmbeddedDiffStatus = "modified" | "added" | "deleted" | "renamed";

export interface EmbeddedDiffFile {
  id?: string;
  path: string;
  originalPath?: string;
  original?: string;
  modified?: string;
  patch?: string;
  additions?: number;
  deletions?: number;
  status?: EmbeddedDiffStatus;
  language?: string;
}

export interface ParsedUnifiedDiff {
  preamble?: string;
  files: EmbeddedDiffFile[];
}

export const MAX_MONACO_CHARACTERS = 240_000;
export const MAX_MONACO_LINES = 8_000;
export const MAX_LIGHTWEIGHT_PREVIEW_CHARACTERS = 36_000;
export const MAX_LIGHTWEIGHT_PREVIEW_LINES = 260;

function cleanHeaderPath(value: string): string {
  const path = value.trim().split("\t", 1)[0].replace(/^"|"$/g, "");
  if (path === "/dev/null") return path;
  return path.replace(/^[ab]\//, "");
}

function pathFromGitHeader(line: string): string | undefined {
  const match = line.match(/^diff --git (?:a\/)?(.+?) (?:b\/)?(.+)$/);
  return match ? cleanHeaderPath(match[2]) : undefined;
}

function countPatchLines(lines: string[]): { additions: number; deletions: number } {
  let additions = 0;
  let deletions = 0;
  let inHunk = false;

  for (const line of lines) {
    if (line.startsWith("@@")) {
      inHunk = true;
      continue;
    }
    if (!inHunk) continue;
    if (line.startsWith("+") && !line.startsWith("+++")) additions += 1;
    if (line.startsWith("-") && !line.startsWith("---")) deletions += 1;
  }

  return { additions, deletions };
}

function diffStatus(originalPath: string | undefined, path: string): EmbeddedDiffStatus {
  if (originalPath === "/dev/null") return "added";
  if (path === "/dev/null") return "deleted";
  if (originalPath && originalPath !== path) return "renamed";
  return "modified";
}

export function parseUnifiedDiff(text: string): ParsedUnifiedDiff {
  const normalized = text.replace(/\r\n?/g, "\n").trimEnd();
  if (!normalized.trim()) return { files: [] };

  const lines = normalized.split("\n");
  const preamble: string[] = [];
  const files: EmbeddedDiffFile[] = [];
  let currentLines: string[] = [];
  let currentPath: string | undefined;
  let currentOriginalPath: string | undefined;
  let sawFileHeader = false;

  const flush = () => {
    if (!currentLines.length) return;
    const patch = currentLines.join("\n");
    const fallbackPath =
      currentPath && currentPath !== "/dev/null"
        ? currentPath
        : currentOriginalPath && currentOriginalPath !== "/dev/null"
          ? currentOriginalPath
          : `changes-${files.length + 1}.diff`;
    const stats = countPatchLines(currentLines);

    files.push({
      id: `${fallbackPath}:${files.length}`,
      path: fallbackPath,
      originalPath:
        currentOriginalPath && currentOriginalPath !== fallbackPath
          ? currentOriginalPath
          : undefined,
      patch,
      additions: stats.additions,
      deletions: stats.deletions,
      status: diffStatus(currentOriginalPath, currentPath || fallbackPath),
      language: languageForPath(fallbackPath),
    });
    currentLines = [];
    currentPath = undefined;
    currentOriginalPath = undefined;
  };

  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index];

    if (line.startsWith("diff --git ")) {
      flush();
      currentPath = pathFromGitHeader(line);
      currentLines.push(line);
      sawFileHeader = true;
      continue;
    }

    const next = lines[index + 1];
    const beginsHeaderPair = line.startsWith("--- ") && next?.startsWith("+++ ");
    if (beginsHeaderPair) {
      if (currentLines.some((entry) => entry.startsWith("@@"))) flush();
      currentOriginalPath = cleanHeaderPath(line.slice(4));
      currentPath = cleanHeaderPath(next.slice(4));
      currentLines.push(line, next);
      sawFileHeader = true;
      index += 1;
      continue;
    }

    if (sawFileHeader) currentLines.push(line);
    else preamble.push(line);
  }

  flush();

  if (!files.length) {
    const stats = countPatchLines(lines);
    files.push({
      id: "changes.diff:0",
      path: "changes.diff",
      patch: normalized,
      additions: stats.additions,
      deletions: stats.deletions,
      status: "modified",
      language: "diff",
    });
    return { files };
  }

  return {
    preamble: preamble.join("\n").trim() || undefined,
    files,
  };
}

export function lineCount(value: string): number {
  if (!value) return 1;
  return value.split("\n").length;
}

export function canUseMonaco(values: Array<string | undefined>): boolean {
  let characters = 0;
  let lines = 0;

  for (const value of values) {
    if (!value) continue;
    characters += value.length;
    lines += lineCount(value);
    if (characters > MAX_MONACO_CHARACTERS || lines > MAX_MONACO_LINES) return false;
  }

  return true;
}

export function editorHeightFor(
  values: Array<string | undefined>,
  minimum = 170,
  maximum = 520,
): number {
  const lines = Math.max(1, ...values.map((value) => lineCount(value || "")));
  return Math.min(maximum, Math.max(minimum, lines * 19 + 30));
}

export function lightweightPreview(value: string): {
  text: string;
  truncated: boolean;
} {
  const lines = value.split("\n");
  const lineLimited = lines.slice(0, MAX_LIGHTWEIGHT_PREVIEW_LINES).join("\n");
  const text = lineLimited.slice(0, MAX_LIGHTWEIGHT_PREVIEW_CHARACTERS);
  return {
    text,
    truncated: text.length < value.length,
  };
}

function safeModelSegment(value: string): string {
  return encodeURIComponent(value.replace(/^\/+/, "") || "untitled");
}

export function monacoModelUri(
  scope: string,
  role: "code" | "original" | "modified",
  path: string,
): string {
  return `inmemory://flamel/tool-widget/${safeModelSegment(scope)}/${role}/${safeModelSegment(path)}`;
}

export function normalizeDiffFile(
  file: EmbeddedDiffFile,
  index: number,
): EmbeddedDiffFile {
  const path = file.path || `change-${index + 1}.diff`;
  const patchStats = file.patch ? countPatchLines(file.patch.split("\n")) : undefined;
  const original = file.original;
  const modified = file.modified;

  return {
    ...file,
    id: file.id || `${path}:${index}`,
    path,
    language: file.language || languageForPath(path),
    additions:
      file.additions ??
      patchStats?.additions ??
      (original === undefined || modified === undefined
        ? 0
        : Math.max(0, lineCount(modified) - lineCount(original))),
    deletions:
      file.deletions ??
      patchStats?.deletions ??
      (original === undefined || modified === undefined
        ? 0
        : Math.max(0, lineCount(original) - lineCount(modified))),
    status:
      file.status ||
      (original === "" && modified !== ""
        ? "added"
        : original !== "" && modified === ""
          ? "deleted"
          : "modified"),
  };
}
