import { capabilityForTool } from "../../lib/toolCapabilities";
import type { ToolArtifact, ToolCall } from "../../lib/messageProcessor";
import type { EmbeddedDiffFile } from "./codePresentation";
import { workspaceToolSummary } from "./workspaceToolSummary";
import { shellToolSummary } from "./shellToolSummary";

export type EngineeringWidgetKind =
  "workspace" | "code" | "source-control" | "shell" | "diagnostics";

export type EngineeringWidgetStatus =
  "pending" | "running" | "success" | "error" | "empty" | "unknown";

export interface EngineeringWidgetEntry {
  id: string;
  primary: string;
  secondary?: string;
  trailing?: string;
}

export interface EngineeringWidgetCheck {
  id: string;
  name: string;
  status: "passed" | "failed" | "running" | "queued" | "skipped" | "unknown";
  detail?: string;
  duration?: string;
}

export interface EngineeringWidgetField {
  label: string;
  value: string;
  mono?: boolean;
}

export interface EngineeringResultV1 {
  schema: "flamel.engineering_result.v1";
  tool: "file_read" | "code_nav";
  action: string;
  status: "ok" | "error";
  summary: string;
  counts: Record<string, number>;
  truncated: boolean;
  truncationReason?: string;
  warnings: string[];
  data: Record<string, unknown>;
  text: string;
}

export interface EngineeringToolView {
  kind: EngineeringWidgetKind;
  action: string;
  title: string;
  eyebrow: string;
  summary: string;
  detail?: string;
  status: EngineeringWidgetStatus;
  statusLabel?: string;
  path?: string;
  fields: EngineeringWidgetField[];
  entries: EngineeringWidgetEntry[];
  checks: EngineeringWidgetCheck[];
  code?: string;
  diffFiles?: EmbeddedDiffFile[];
  diffPatch?: string;
  logLines: string[];
  outputText: string;
  artifacts: ToolArtifact[];
  truncated: boolean;
  command?: string;
  structuredResult?: EngineeringResultV1;
}

type JsonRecord = Record<string, unknown>;

const MAX_OUTPUT_CHARS = 32 * 1024;
const MAX_ENTRIES = 60;
const MAX_LOG_LINES = 120;

const ACTION_ALIASES: Record<string, string> = {
  file_read: "read",
  flamel_read: "read",
  read_file: "read",
  batch_read: "batch",
  file_summary: "summary",
  flamel_glob: "glob",
  find_file: "find",
  list_dir: "list_dir",
  flamel_replace: "replace",
  replace_content: "replace_content",
  multi_replace: "multi_replace",
  replace_symbol_body: "replace_symbol",
  insert_before_symbol: "insert_before",
  insert_after_symbol: "insert_after",
  create_text_file: "create",
  diff_apply: "diff_apply",
  format_file: "format",
  preview_edit: "preview",
  flamel_search: "search",
  get_symbols_overview: "symbols",
  find_symbol: "find",
  find_referencing_symbols: "refs",
  get_imports: "imports",
  gather_context: "context",
  dependency_graph: "deps",
  flamel_shell: "run",
  shell_run: "run",
  shell_start: "start",
  shell_read: "read",
  shell_write: "write",
  shell_kill: "kill",
  shell_status: "status",
  multi_shell: "multi",
  bash: "run",
  exec_command: "exec",
  command_execution: "run",
  terminal: "run",
  runtime_logs: "logs",
  build_errors: "verification",
  run_tests: "verify",
  explain_error: "diagnostics",
  find_code_issues: "verification",
};

function asRecord(value: unknown): JsonRecord | undefined {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as JsonRecord)
    : undefined;
}

function stringValue(
  record: JsonRecord | undefined,
  keys: string[],
): string | undefined {
  if (!record) return undefined;
  for (const key of keys) {
    const value = record[key];
    if (typeof value === "string" && value.trim()) return value.trim();
    if (typeof value === "number" || typeof value === "boolean")
      return String(value);
  }
  return undefined;
}

function numberValue(
  record: JsonRecord | undefined,
  keys: string[],
): number | undefined {
  if (!record) return undefined;
  for (const key of keys) {
    const value = record[key];
    if (typeof value === "number" && Number.isFinite(value)) return value;
    if (
      typeof value === "string" &&
      value.trim() &&
      Number.isFinite(Number(value))
    ) {
      return Number(value);
    }
  }
  return undefined;
}

function shortToolName(name: string): string {
  return name.includes("__") ? name.split("__").pop() || name : name;
}

function partialInputRecord(partialInput: string | undefined): JsonRecord {
  if (!partialInput) return {};
  try {
    const parsed = JSON.parse(partialInput);
    return asRecord(parsed) ?? {};
  } catch {
    const result: JsonRecord = {};
    for (const key of [
      "action",
      "file",
      "path",
      "relative_path",
      "file_path",
      "project_path",
      "pattern",
      "name_path",
      "symbol_name",
      "command",
      "cmd",
      "cwd",
      "branch",
    ]) {
      const match = partialInput.match(
        new RegExp(`["']${key}["']\\s*:\\s*["']([^"']*)`),
      );
      if (match?.[1]) result[key] = match[1];
    }
    return result;
  }
}

export function engineeringInput(tool: ToolCall): JsonRecord {
  return { ...partialInputRecord(tool.partialInput), ...tool.input };
}

export function engineeringAction(tool: ToolCall): string {
  const input = engineeringInput(tool);
  const explicit = stringValue(input, ["action"]);
  if (explicit) return explicit.toLowerCase();
  const short = shortToolName(tool.name).toLowerCase();
  return ACTION_ALIASES[short] ?? short;
}

export function engineeringWidgetKind(
  tool: ToolCall,
): EngineeringWidgetKind | null {
  const capability = capabilityForTool(tool.name, engineeringInput(tool));
  const action = engineeringAction(tool);

  if (capability.name === "shell") return "shell";
  if (
    capability.name === "code_quality" ||
    capability.renderer === "diagnostics" ||
    (capability.name === "project_intelligence" &&
      ["verification", "verify"].includes(action))
  ) {
    return "diagnostics";
  }
  if (
    capability.family === "source-control" ||
    capability.name === "get_recent_changes"
  ) {
    return "source-control";
  }
  if (capability.renderer === "file") return "workspace";
  if (capability.renderer === "code") return "code";
  return null;
}

export function isEngineeringToolCall(tool: ToolCall): boolean {
  return engineeringWidgetKind(tool) !== null;
}

function engineeringResultV1(
  value: unknown,
  expectedTool: string,
  expectedAction: string,
): EngineeringResultV1 | undefined {
  const record = asRecord(value);
  if (
    record?.schema !== "flamel.engineering_result.v1" ||
    (record.tool !== "file_read" && record.tool !== "code_nav") ||
    record.tool !== expectedTool ||
    record.action !== expectedAction ||
    (record.status !== "ok" && record.status !== "error")
  ) {
    return undefined;
  }

  const counts = Object.fromEntries(
    Object.entries(asRecord(record.counts) ?? {}).filter(
      (entry): entry is [string, number] =>
        typeof entry[1] === "number" && Number.isFinite(entry[1]),
    ),
  );
  const warnings = Array.isArray(record.warnings)
    ? record.warnings.filter((warning): warning is string => typeof warning === "string").slice(0, 8)
    : [];

  return {
    schema: "flamel.engineering_result.v1",
    tool: record.tool,
    action: record.action,
    status: record.status,
    summary: typeof record.summary === "string" ? record.summary : "",
    counts,
    truncated: record.truncated === true,
    ...(typeof record.truncationReason === "string"
      ? { truncationReason: record.truncationReason }
      : {}),
    warnings,
    data: asRecord(record.data) ?? {},
    text: typeof record.text === "string" ? record.text : "",
  };
}

function structuredResultPath(result: EngineeringResultV1): string | undefined {
  const direct = stringValue(result.data, ["path"]);
  if (direct) return direct;
  const files = Array.isArray(result.data.files) ? result.data.files : [];
  return stringValue(asRecord(files[0]), ["path"]);
}

function structuredResultSource(
  result: EngineeringResultV1,
): { path: string; code: string } | undefined {
  if (result.tool === "code_nav" && result.action === "find") {
    const path = stringValue(result.data, ["path"]);
    const code = stringValue(result.data, ["source"]);
    return path && code ? { path, code } : undefined;
  }
  if (result.tool === "file_read" && ["read", "batch"].includes(result.action)) {
    const files = Array.isArray(result.data.files) ? result.data.files : [];
    const first = asRecord(files[0]);
    const path = stringValue(first, ["path"]);
    const code = stringValue(first, ["source"]);
    return path && code ? { path, code } : undefined;
  }
  return undefined;
}

function outputTextFrom(value: unknown, depth = 0): string {
  if (depth > 5 || value === null || value === undefined) return "";
  if (typeof value === "string") return value;
  if (Array.isArray(value)) {
    return value
      .map((item) => outputTextFrom(item, depth + 1))
      .filter(Boolean)
      .join("\n");
  }

  const record = asRecord(value);
  if (!record) return "";

  if (Array.isArray(record.content)) {
    const content = record.content
      .map((item) => {
        const block = asRecord(item);
        return stringValue(block, ["text"]) ?? outputTextFrom(item, depth + 1);
      })
      .filter(Boolean)
      .join("\n");
    if (content) return content;
  }

  for (const key of [
    "preview",
    "text",
    "output",
    "message",
    "summary",
    "result",
    "logs",
  ]) {
    if (record[key] !== undefined) {
      const text = outputTextFrom(record[key], depth + 1);
      if (text) return text;
    }
  }

  for (const key of ["data", "structuredContent", "structured_content"]) {
    if (record[key] !== undefined) {
      const text = outputTextFrom(record[key], depth + 1);
      if (text) return text;
    }
  }
  return "";
}

export function decodeEngineeringOutput(output: string | undefined): {
  parsed?: unknown;
  text: string;
  truncated: boolean;
} {
  const raw = output?.trim() ?? "";
  if (!raw) return { text: "", truncated: false };

  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    parsed = undefined;
  }

  let text = parsed === undefined ? raw : outputTextFrom(parsed) || raw;
  if ((text.startsWith("{") || text.startsWith("[")) && text !== raw) {
    try {
      const nested = JSON.parse(text);
      parsed = nested;
      text = outputTextFrom(nested) || text;
    } catch {
      // A textual source result may begin with braces; keep it intact.
    }
  }

  const truncated = text.length > MAX_OUTPUT_CHARS;
  if (truncated)
    text = `${text.slice(0, MAX_OUTPUT_CHARS)}\n… output truncated for this card`;
  return { parsed, text: stripAnsi(text), truncated };
}

function stripAnsi(value: string): string {
  return value.replace(/\u001b\[[0-9;]*m/g, "");
}

function cleanReadOutput(text: string): string {
  return text
    .split("\n")
    .filter((line) => !/^\[[^\]]+: lines? \d+/i.test(line.trim()))
    .map((line) => line.replace(/^\s*\d+\t/, ""))
    .join("\n")
    .trimEnd();
}

function legacyFileReadBatchResult(
  text: string,
  truncated: boolean,
): EngineeringResultV1 | undefined {
  const files: Array<{
    path: string;
    totalLines: number;
    resolvedRange: { startLine: number; endLine: number };
    source: string;
  }> = [];
  let current:
    | {
        path: string;
        totalLines: number;
        startLine: number;
        endLine: number;
        source: string[];
      }
    | undefined;

  const finish = () => {
    if (!current) return;
    files.push({
      path: current.path,
      totalLines: current.totalLines,
      resolvedRange: {
        startLine: current.startLine,
        endLine: current.endLine,
      },
      source: current.source.join("\n").trimEnd(),
    });
  };

  for (const line of text.split(/\r?\n/)) {
    const full = line.match(/^=== (.+?) \((\d+) lines?\) ===$/);
    const ranged = line.match(/^=== (.+?) \(lines (\d+)-(\d+) of (\d+)\) ===$/);
    if (full || ranged) {
      finish();
      current = full
        ? {
            path: full[1],
            totalLines: Number(full[2]),
            startLine: 1,
            endLine: Number(full[2]),
            source: [],
          }
        : {
            path: ranged![1],
            totalLines: Number(ranged![4]),
            startLine: Number(ranged![2]),
            endLine: Number(ranged![3]),
            source: [],
          };
      continue;
    }

    const numbered = current ? line.match(/^\s*\d+\t(.*)$/) : undefined;
    if (current && numbered) current.source.push(numbered[1]);
  }
  finish();

  if (files.length < 2) return undefined;
  return {
    schema: "flamel.engineering_result.v1",
    tool: "file_read",
    action: "batch",
    status: "ok",
    summary: files.length + " files read",
    counts: {
      files: files.length,
      lines: files.reduce(
        (total, file) =>
          total +
          Math.max(
            0,
            file.resolvedRange.endLine - file.resolvedRange.startLine + 1,
          ),
        0,
      ),
      totalLines: files.reduce((total, file) => total + file.totalLines, 0),
    },
    truncated,
    ...(truncated ? { truncationReason: "card_limit" } : {}),
    warnings: [],
    data: { files },
    text,
  };
}

function codeNavigationSource(
  action: string,
  input: JsonRecord,
  text: string,
): { path: string; code: string } | undefined {
  if (action !== "find" || !text) return undefined;
  const lines = text.split("\n");
  const headerIndex = lines.findIndex((line) =>
    /^Symbol\s+.+?\s+in\s+.+\s+\(L\d+(?:-\d+)?\):\s*$/.test(line.trim()),
  );
  if (headerIndex < 0) return undefined;
  const header = lines[headerIndex]
    .trim()
    .match(/^Symbol\s+.+?\s+in\s+(.+?)\s+\(L\d+(?:-\d+)?\):\s*$/);
  const code = lines
    .slice(headerIndex + 1)
    .flatMap((line) => {
      const numbered = line.match(/^\s*\d+\t(.*)$/);
      return numbered ? [numbered[1]] : [];
    })
    .join("\n")
    .trimEnd();
  const path = primaryPath(input) ?? header?.[1]?.trim();
  return path && code ? { path, code } : undefined;
}

function primaryPath(input: JsonRecord): string | undefined {
  return stringValue(input, ["file", "path", "relative_path", "file_path"]);
}

function displayAction(action: string): string {
  return action.replace(/[._-]+/g, " ").trim();
}

function findFirstArray(
  value: unknown,
  keys: Set<string>,
  depth = 0,
  seen = new Set<unknown>(),
): unknown[] | undefined {
  if (depth > 6 || value === null || value === undefined || seen.has(value))
    return undefined;
  if (typeof value === "object") seen.add(value);

  if (Array.isArray(value)) {
    for (const item of value) {
      const found = findFirstArray(item, keys, depth + 1, seen);
      if (found) return found;
    }
    return undefined;
  }

  const record = asRecord(value);
  if (!record) return undefined;
  for (const [key, candidate] of Object.entries(record)) {
    if (keys.has(key.toLowerCase()) && Array.isArray(candidate))
      return candidate;
  }
  for (const candidate of Object.values(record)) {
    const found = findFirstArray(candidate, keys, depth + 1, seen);
    if (found) return found;
  }
  return undefined;
}

function entryFromRecord(
  record: JsonRecord,
  index: number,
): EngineeringWidgetEntry | undefined {
  const primary = stringValue(record, [
    "path",
    "file",
    "name",
    "label",
    "title",
    "symbol",
    "check",
    "command",
  ]);
  if (!primary) return undefined;
  const secondary = stringValue(record, [
    "detail",
    "message",
    "summary",
    "description",
    "kind",
    "type",
  ]);
  const trailing = stringValue(record, [
    "status",
    "state",
    "line",
    "size",
    "duration",
  ]);
  return {
    id: stringValue(record, ["id"]) ?? `entry-${index}-${primary}`,
    primary,
    secondary,
    trailing,
  };
}

function entryFromLine(
  line: string,
  index: number,
): EngineeringWidgetEntry | undefined {
  const clean = line.trim();
  if (!clean || /^[{}\[\],]+$/.test(clean)) return undefined;

  const git = clean.match(/^([ MADRCU?!]{1,2})\s+(.+)$/);
  if (git?.[2] && git[1]?.trim()) {
    return {
      id: `line-${index}-${git[2]}`,
      primary: git[2],
      trailing: git[1].trim(),
    };
  }

  const location = clean.match(/^(.+?):(\d+)(?::(\d+))?[:\-]\s*(.*)$/);
  if (location) {
    return {
      id: `line-${index}-${location[1]}-${location[2]}`,
      primary: location[1],
      secondary: location[4] || undefined,
      trailing: location[3]
        ? `L${location[2]}:${location[3]}`
        : `L${location[2]}`,
    };
  }

  const tabbed = clean.split("\t").filter(Boolean);
  return {
    id: `line-${index}-${clean.slice(0, 40)}`,
    primary: tabbed[0] ?? clean,
    secondary: tabbed.length > 2 ? tabbed.slice(1, -1).join(" · ") : undefined,
    trailing: tabbed.length > 1 ? tabbed[tabbed.length - 1] : undefined,
  };
}

function sourceControlEntries(
  parsed: unknown,
  text: string,
): EngineeringWidgetEntry[] {
  const root = asRecord(parsed);
  const issue =
    asRecord(root?.pull_request) ??
    asRecord(root?.pullRequest) ??
    asRecord(root?.issue);
  if (issue) {
    const number = stringValue(issue, ["number"]);
    const title = stringValue(issue, ["title"]);
    const pull = asRecord(root?.pull);
    const head = stringValue(pull, ["head_branch", "headBranch"]);
    const base = stringValue(pull, ["base_branch", "baseBranch"]);
    const primary =
      [number ? `#${number}` : "Pull request", title]
        .filter(Boolean)
        .join(" ") || "Pull request";
    return [
      {
        id: stringValue(issue, ["id"]) ?? `pull-request-${number ?? "result"}`,
        primary,
        secondary: head && base ? `${head} → ${base}` : undefined,
        trailing: stringValue(issue, ["state", "status"]),
      },
    ];
  }
  return evidenceEntries(parsed, text);
}

function evidenceEntries(
  parsed: unknown,
  text: string,
): EngineeringWidgetEntry[] {
  const array = findFirstArray(
    parsed,
    new Set([
      "files",
      "matches",
      "results",
      "items",
      "symbols",
      "references",
      "commits",
    ]),
  );
  if (array) {
    const structured = array
      .map((item, index) =>
        typeof item === "string"
          ? entryFromLine(item, index)
          : entryFromRecord(asRecord(item) ?? {}, index),
      )
      .filter((item): item is EngineeringWidgetEntry => Boolean(item))
      .slice(0, MAX_ENTRIES);
    if (structured.length > 0) return structured;
  }

  return text
    .split("\n")
    .map(entryFromLine)
    .filter((item): item is EngineeringWidgetEntry => Boolean(item))
    .slice(0, MAX_ENTRIES);
}

function normalizeCheckStatus(
  value: string | undefined,
): EngineeringWidgetCheck["status"] {
  const status = value?.toLowerCase() ?? "";
  if (/pass|success|ok|done|complete/.test(status)) return "passed";
  if (/fail|error|broken/.test(status)) return "failed";
  if (/run|progress|active/.test(status)) return "running";
  if (/queue|pending|todo|waiting/.test(status)) return "queued";
  if (/skip|disabled|ignored/.test(status)) return "skipped";
  return "unknown";
}

function checksFrom(parsed: unknown, text: string): EngineeringWidgetCheck[] {
  const array = findFirstArray(
    parsed,
    new Set(["checks", "verifications", "diagnostics", "tests", "failures"]),
  );
  if (array) {
    const checks = array
      .map((item, index) => {
        const record = asRecord(item);
        if (!record) {
          if (typeof item !== "string") return undefined;
          return {
            id: `check-${index}`,
            name: item,
            status: normalizeCheckStatus(item),
          } satisfies EngineeringWidgetCheck;
        }
        const name = stringValue(record, [
          "name",
          "label",
          "title",
          "check",
          "id",
        ]);
        if (!name) return undefined;
        const rawStatus = stringValue(record, [
          "status",
          "state",
          "result",
          "outcome",
        ]);
        const duration = stringValue(record, [
          "duration",
          "elapsed",
          "time_ms",
        ]);
        return {
          id: stringValue(record, ["id"]) ?? `check-${index}-${name}`,
          name,
          status: normalizeCheckStatus(rawStatus),
          detail: stringValue(record, [
            "detail",
            "message",
            "summary",
            "error",
            "output",
          ]),
          duration: duration
            ? duration.match(/[a-z]/i)
              ? duration
              : `${duration} ms`
            : undefined,
        } satisfies EngineeringWidgetCheck;
      })
      .filter((item): item is EngineeringWidgetCheck => Boolean(item))
      .slice(0, 40);
    if (checks.length > 0) return checks;
  }

  return text
    .split("\n")
    .map((line, index) => {
      const match = line
        .trim()
        .match(
          /^(?:[✓✔☑]|\[?(pass|ok|fail|error|skip|running)\]?)[\s:—-]+(.+)$/i,
        );
      if (!match) return undefined;
      return {
        id: `check-line-${index}`,
        name: match[2],
        status: normalizeCheckStatus(match[1] || line),
      } satisfies EngineeringWidgetCheck;
    })
    .filter((item): item is EngineeringWidgetCheck => Boolean(item))
    .slice(0, 40);
}

function fieldsFromInput(
  input: JsonRecord,
  path: string | undefined,
): EngineeringWidgetField[] {
  const definitions: Array<[string, string[], boolean?]> = [
    ["Root", ["project_path", "root", "cwd"], true],
    ["Path", ["file", "path", "relative_path", "file_path"], true],
    ["Pattern", ["pattern", "query"], true],
    ["Symbol", ["name_path", "symbol_name"], true],
    ["Branch", ["branch", "ref"], true],
    ["Session", ["session_id"], true],
    ["Range", ["start_line", "end_line"], true],
  ];

  const fields: EngineeringWidgetField[] = [];
  for (const [label, keys, mono] of definitions) {
    const value = stringValue(input, keys);
    if (!value || (label === "Path" && value === path)) continue;
    fields.push({ label, value, mono });
  }

  const files = Array.isArray(input.files) ? input.files.length : undefined;
  const commands = Array.isArray(input.commands)
    ? input.commands.length
    : undefined;
  if (files !== undefined)
    fields.push({ label: "Files", value: String(files) });
  if (commands !== undefined)
    fields.push({ label: "Commands", value: String(commands) });
  return fields.slice(0, 8);
}

function looksLikeUnifiedDiff(value: string): boolean {
  return (
    /^diff --git /m.test(value) ||
    (/^--- (?:a\/|\/dev\/null|\S+)/m.test(value) &&
      /^\+\+\+ (?:b\/|\/dev\/null|\S+)/m.test(value))
  );
}

function countLines(value: string): number {
  return value ? value.split("\n").length : 0;
}

function diffForFileEdit(
  input: JsonRecord,
  outputText: string,
  action: string,
): { files?: EmbeddedDiffFile[]; patch?: string } {
  if (looksLikeUnifiedDiff(outputText)) return { patch: outputText };
  const inputPatch = stringValue(input, ["diff"]);
  if (inputPatch) return { patch: inputPatch };

  const edits = Array.isArray(input.edits) ? input.edits : undefined;
  if (edits) {
    const files = edits
      .map<EmbeddedDiffFile | undefined>((edit, index) => {
        const record = asRecord(edit);
        if (!record) return undefined;
        const path =
          stringValue(record, ["file", "path", "relative_path"]) ??
          `change-${index + 1}.txt`;
        const original =
          stringValue(record, ["old_string", "needle", "before"]) ?? "";
        const modified =
          stringValue(record, ["new_string", "repl", "after"]) ?? "";
        return {
          id: `${index}-${path}`,
          path,
          original,
          modified,
          additions: countLines(modified),
          deletions: countLines(original),
          status: original ? "modified" : "added",
        } satisfies EmbeddedDiffFile;
      })
      .filter((file): file is EmbeddedDiffFile => Boolean(file));
    if (files.length > 0) return { files };
  }

  const path = primaryPath(input) ?? "changes.txt";
  const original = stringValue(input, ["old_string", "needle", "before"]);
  const modified = stringValue(input, [
    "new_string",
    "repl",
    "after",
    "content",
  ]);
  if (original !== undefined || modified !== undefined) {
    return {
      files: [
        {
          path,
          original: original ?? "",
          modified: modified ?? "",
          additions: countLines(modified ?? ""),
          deletions: countLines(original ?? ""),
          status: action === "create" || !original ? "added" : "modified",
        },
      ],
    };
  }

  return {};
}

function commandFromInput(input: JsonRecord): string | undefined {
  const command = stringValue(input, ["command", "cmd"]);
  if (command) return command;
  if (Array.isArray(input.commands)) {
    const commands = input.commands
      .map((item) =>
        typeof item === "string"
          ? item
          : stringValue(asRecord(item), ["command", "cmd"]),
      )
      .filter((item): item is string => Boolean(item));
    if (commands.length) return commands.join(" · ");
  }
  const text = stringValue(input, ["text"]);
  return text ? text.trimEnd() : undefined;
}

function widgetStatus(
  tool: ToolCall,
  hasResult: boolean,
  mutationReceipt = false,
): EngineeringWidgetStatus {
  if (tool.status === "unknown") return "unknown";
  if (tool.status === "pending") return "pending";
  if (tool.status === "running") return "running";
  if (tool.status === "error") return "error";
  return hasResult || mutationReceipt ? "success" : "empty";
}

function summaryFor(
  kind: EngineeringWidgetKind,
  action: string,
  tool: ToolCall,
  entries: EngineeringWidgetEntry[],
  checks: EngineeringWidgetCheck[],
  command: string | undefined,
  result: unknown,
): string {
  if (tool.status === "unknown") return "Completion not confirmed";
  const running = tool.status === "pending" || tool.status === "running";
  const failed = tool.status === "error";

  if (kind === "shell") {
    return shellToolSummary(action, tool.status, command, result);
  }

  if (kind === "source-control") {
    if (failed) return "Source-control operation failed";
    if (looksLikeUnifiedDiff(tool.output ?? "") || action.includes("diff")) {
      return running ? "Preparing source diff" : "Source diff is ready";
    }
    if (checks.length) {
      const failedChecks = checks.filter(
        (check) => check.status === "failed",
      ).length;
      const activeChecks = checks.filter(
        (check) => check.status === "running" || check.status === "queued",
      ).length;
      if (failedChecks) {
        return `${failedChecks} of ${checks.length} checks failed`;
      }
      if (activeChecks) {
        return `${activeChecks} of ${checks.length} checks running`;
      }
      return `${checks.length} check${checks.length === 1 ? "" : "s"} passed`;
    }
    if (action === "repo.status") {
      return running
        ? "Checking the working tree"
        : entries.length
          ? `${entries.length} changed item${entries.length === 1 ? "" : "s"}`
          : "Working tree is clean";
    }
    return running
      ? `${displayAction(action)} in progress`
      : `${displayAction(action)} complete`;
  }

  if (kind === "diagnostics") {
    if (failed) return "Verification failed";
    if (running)
      return checks.length
        ? `${checks.length} checks in progress`
        : "Verification is running";
    if (checks.length) {
      const passed = checks.filter((check) => check.status === "passed").length;
      const failedChecks = checks.filter(
        (check) => check.status === "failed",
      ).length;
      return failedChecks
        ? `${failedChecks} of ${checks.length} checks failed`
        : `${passed || checks.length} checks passed`;
    }
    return "Diagnostics complete";
  }

  if (failed) return "Code inspection failed";
  if (running) return `${displayAction(action)} in progress`;
  return entries.length
    ? `${entries.length} evidence item${entries.length === 1 ? "" : "s"}`
    : "Code evidence is ready";
}

function titleFor(kind: EngineeringWidgetKind, canonicalName: string): string {
  if (kind === "workspace")
    return canonicalName === "file_read"
      ? "Workspace read"
      : "Workspace change";
  if (kind === "source-control") return "Source control";
  if (kind === "shell") return "Shell";
  if (kind === "diagnostics") return "Verification";
  return canonicalName === "project_intelligence"
    ? "Project intelligence"
    : "Code navigation";
}

function logLinesFor(
  text: string,
  entries: EngineeringWidgetEntry[],
): string[] {
  const entryLines = new Set(entries.map((entry) => entry.primary));
  return text
    .split("\n")
    .map((line) => line.trimEnd())
    .filter((line) => line.trim() && !entryLines.has(line.trim()))
    .slice(-MAX_LOG_LINES);
}

export function adaptEngineeringTool(
  tool: ToolCall,
): EngineeringToolView | null {
  const kind = engineeringWidgetKind(tool);
  if (!kind) return null;

  const input = engineeringInput(tool);
  const capability = capabilityForTool(tool.name, input);
  const action = engineeringAction(tool);
  const decoded = decodeEngineeringOutput(
    tool.presentation?.text ?? tool.output,
  );
  const protocolResult = engineeringResultV1(
    tool.presentation?.structuredContent,
    capability.name,
    action,
  );
  const structuredResult =
    protocolResult ??
    (capability.name === "file_read" && action === "batch"
      ? legacyFileReadBatchResult(decoded.text, decoded.truncated)
      : undefined);
  const resultSource = structuredResult
    ? structuredResultSource(structuredResult)
    : undefined;
  const truncated =
    structuredResult?.truncated === true ||
    decoded.truncated ||
    tool.presentation?.truncated === true;
  const codeSource =
    resultSource ??
    (kind === "code"
      ? codeNavigationSource(action, input, decoded.text)
      : undefined);
  const path =
    structuredResult && structuredResultPath(structuredResult)
      ? structuredResultPath(structuredResult)
      : primaryPath(input) ?? codeSource?.path;
  const command = commandFromInput(input);
  const checks =
    kind === "diagnostics" || kind === "source-control"
      ? checksFrom(decoded.parsed, decoded.text)
      : [];
  const entries = structuredResult
    ? []
    : kind === "shell" || kind === "diagnostics" || codeSource
      ? []
      : kind === "source-control"
        ? sourceControlEntries(decoded.parsed, decoded.text)
        : evidenceEntries(decoded.parsed, decoded.text);
  const editDiff =
    kind === "workspace" && capability.name !== "file_read"
      ? diffForFileEdit(input, decoded.text, action)
      : {};
  const gitPatch =
    kind === "source-control" && looksLikeUnifiedDiff(decoded.text)
      ? decoded.text
      : undefined;
  const code =
    codeSource?.code ??
    (kind === "workspace" &&
    capability.name === "file_read" &&
    ["read", "summary"].includes(action) &&
    decoded.text
      ? cleanReadOutput(decoded.text)
      : undefined);
  const diffPatch = editDiff.patch ?? gitPatch;
  const diffFiles = editDiff.files;
  const mutationReceipt =
    !capability.readOnly &&
    Boolean(path || command || Object.keys(input).length);
  const hasResult = Boolean(
    structuredResult ||
    decoded.text ||
    entries.length ||
    checks.length ||
    code ||
    diffPatch ||
    diffFiles?.length,
  );
  const status =
    structuredResult?.status === "error"
      ? "error"
      : widgetStatus(tool, hasResult, mutationReceipt);
  const generatedSummary = kind === "workspace"
    ? workspaceToolSummary({
        input,
        action,
        editing: capability.name !== "file_read",
        status: tool.status,
        resultData: structuredResult?.data,
        resultCounts: structuredResult?.counts,
        resultError: structuredResult?.status === "error",
        entryCount: entries.length,
      })
    : summaryFor(kind, action, tool, entries, checks, command, decoded.parsed);
  const settled = tool.status === "done" || tool.status === "error";

  return {
    kind,
    action,
    title: titleFor(kind, capability.name),
    eyebrow:
      kind === "source-control"
        ? "Git"
        : kind === "shell"
          ? "Runtime"
          : kind === "diagnostics"
            ? "Diagnostics"
            : kind === "code"
              ? "Code"
              : "Filesystem",
    summary:
      kind !== "workspace" && settled && structuredResult?.summary
        ? structuredResult.summary
        : generatedSummary,
    detail: displayAction(action),
    status,
    statusLabel:
      status === "success" && truncated ? "Done · clipped" : undefined,
    path,
    fields:
      structuredResult?.tool === "file_read" && structuredResult.action === "batch"
        ? []
        : fieldsFromInput(input, path),
    entries,
    checks,
    code,
    diffFiles,
    diffPatch,
    logLines: logLinesFor(structuredResult?.text || decoded.text, entries),
    outputText: structuredResult?.text || decoded.text,
    artifacts: tool.presentation?.artifacts ?? tool.artifacts ?? [],
    truncated,
    command,
    structuredResult,
  };
}
