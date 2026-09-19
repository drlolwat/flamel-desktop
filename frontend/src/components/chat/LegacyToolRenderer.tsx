import React, { useEffect, useState } from "react";
import type { ToolCall } from "../../lib/messageProcessor";
import { apiFetch } from "../../lib/api";
import { InteractiveConsole } from "../console/InteractiveConsole";
import {
  isShellLikeToolCall,
  toolCallToCommandEvents,
} from "../console/consoleAdapters";
import {
  isPlanToolMarkerOutput,
  planToolOutputLabel,
} from "../../lib/workflowEvents";
import { Icon } from "../ui/Icons";
import { parseToolName } from "./toolPresentation";

const EmbeddedDiffView = React.lazy(() =>
  import("../tool-widgets/EmbeddedDiffView").then((module) => ({
    default: module.EmbeddedDiffView,
  })),
);

const IMAGE_ARTIFACT_EXT_RE = /\.(?:png|jpe?g|webp|gif)$/i;
const QUOTED_IMAGE_ARTIFACT_RE =
  /["'`]((?:\/[^"'`]+?)\.(?:png|jpe?g|webp|gif))["'`]/gi;
const BARE_IMAGE_ARTIFACT_RE =
  /(?:^|[\s([{<])((?:\/[^\s"'`<>()\[\]{}]+?)\.(?:png|jpe?g|webp|gif))(?=$|[\s)\]}>.,;:])/gim;

function extractImageArtifactPaths(output: string): string[] {
  const paths: string[] = [];

  const add = (candidate: string) => {
    const clean = candidate.trim().replace(/[),.;:]+$/, "");
    if (
      clean.startsWith("/") &&
      IMAGE_ARTIFACT_EXT_RE.test(clean) &&
      !paths.includes(clean)
    ) {
      paths.push(clean);
    }
  };

  for (const match of output.matchAll(QUOTED_IMAGE_ARTIFACT_RE)) {
    add(match[1]);
  }
  for (const match of output.matchAll(BARE_IMAGE_ARTIFACT_RE)) {
    add(match[1]);
  }

  return paths.slice(0, 4);
}

function fileAssetPath(path: string): string {
  return `/api/file-asset?path=${encodeURIComponent(path)}`;
}

// Descriptive label for each flamel plan tool call
function getPlanToolLabel(tool: ToolCall, isRunning: boolean): string {
  // Resolve action from consolidated tool or legacy name
  const bare = tool.name.includes("__")
    ? tool.name.split("__").pop()!
    : tool.name;
  const isPlanConsolidated = bare === "plan";
  const action = isPlanConsolidated
    ? (tool.input?.action as string | undefined) || ""
    : bare.replace(/^flamel_plan_/, "");
  const name = isPlanConsolidated ? `flamel_plan_${action}` : bare;

  if (name === "flamel_plan_create")
    return isRunning ? "Creating plan…" : "Created plan";
  if (name === "flamel_plan_replace_items")
    return isRunning ? "Rewriting items…" : "Rewrote plan items";
  if (name === "flamel_plan_rename")
    return isRunning ? "Renaming plan…" : "Renamed plan";
  if (name === "flamel_plan_mark_finished")
    return isRunning ? "Finishing plan…" : "Marked plan complete";
  if (name === "flamel_plan_delete_item")
    return isRunning ? "Deleting item…" : "Deleted item";

  if (name === "flamel_plan_add_items") {
    const count = Array.isArray(tool.input?.items)
      ? (tool.input.items as any[]).length
      : 0;
    if (isRunning) return "Adding items…";
    return count > 1 ? `Added ${count} items` : "Added item";
  }

  if (name === "flamel_plan_update_item") {
    const status = tool.input?.status as string | undefined;
    // Parse item text from completed output: "ITEM_UPDATED:{...}"
    let itemText: string | undefined;
    if (!isRunning && tool.output) {
      try {
        const m = tool.output.match(/ITEM_UPDATED:(\{.+\})/);
        if (m) {
          const p = JSON.parse(m[1]);
          itemText = p.text as string;
        }
      } catch {}
    }
    const label = itemText
      ? `"${itemText.length > 32 ? itemText.slice(0, 29) + "…" : itemText}"`
      : "item";
    if (isRunning) {
      if (status === "done") return "Marking as done…";
      if (status === "in_progress") return "Starting item…";
      if (status === "blocked") return "Blocking item…";
      return "Updating item…";
    }
    if (status === "done") return `Marked ${label} as done`;
    if (status === "in_progress") return `Started ${label}`;
    if (status === "blocked") return `Blocked ${label}`;
    return `Updated ${label}`;
  }

  // Todo tools
  if (name === "flamel_todo_add") {
    if (isRunning) return "Adding todos…";
    let count = Array.isArray(tool.input?.items)
      ? (tool.input.items as any[]).length
      : 0;
    if (!count && tool.output) {
      try {
        const m = tool.output.match(/TODO_ADDED:(\[.+?\])/);
        if (m) count = JSON.parse(m[1]).length;
      } catch {}
    }
    return count > 1 ? `Added ${count} todos` : "Added todo";
  }
  if (name === "flamel_todo_check") {
    if (isRunning) return "Checking off todos…";
    let texts: string[] = [];
    if (tool.output) {
      try {
        const m = tool.output.match(/TODO_CHECKED:(\[.+?\])/);
        if (m) texts = JSON.parse(m[1]).map((t: any) => t.text);
      } catch {}
    }
    if (texts.length === 1)
      return `Checked off "${texts[0].length > 28 ? texts[0].slice(0, 25) + "…" : texts[0]}"`;
    return texts.length > 1
      ? `Checked off ${texts.length} todos`
      : "Checked off todo";
  }

  return isRunning ? "Updating plan…" : "Updated plan";
}

// Shorten a path to last N segments for compact display
function shortPath(p: string, segments = 2): string {
  if (!p) return "…";
  const parts = p.split("/").filter(Boolean);
  return parts.length <= segments
    ? parts.join("/")
    : parts.slice(-segments).join("/");
}

// Unused — kept for potential future use
function _getSearchSummary(tool: ToolCall): string {
  const { server, label } = parseToolName(tool.name);
  const prefix = server || label;
  const inp = tool.input;

  if (tool.status === "running" || tool.status === "pending") {
    if (tool.name === "Read")
      return `Reading ${shortPath((inp.file_path as string) || (inp.path as string) || "")}`;
    if (tool.name === "Glob") return `Glob ${(inp.pattern as string) || "…"}`;
    if (tool.name === "Grep")
      return `Grep "${(inp.pattern as string) || "…"}" ${inp.path ? shortPath(inp.path as string) : ""}`.trim();
    // Serena tools — show what's being searched
    if (tool.name.includes("find_symbol"))
      return `${prefix}: finding ${(inp.symbol_name as string) || "…"}`;
    if (tool.name.includes("search_for_pattern"))
      return `${prefix}: searching "${(inp.pattern as string) || "…"}"`;
    if (tool.name.includes("search_for_file"))
      return `${prefix}: finding file ${(inp.file_name as string) || (inp.pattern as string) || "…"}`;
    if (tool.name.includes("get_document"))
      return `${prefix}: reading ${shortPath((inp.relative_path as string) || (inp.path as string) || "")}`;
    if (tool.name.includes("get_symbols_overview"))
      return `${prefix}: analyzing ${shortPath((inp.relative_path as string) || (inp.path as string) || "", 3)}`;
    if (tool.name.includes("list_dir"))
      return `${prefix}: listing ${shortPath((inp.relative_path as string) || (inp.path as string) || "", 3)}`;
    if (tool.name.includes("activate_project"))
      return `${prefix}: activating ${shortPath((inp.project_path as string) || "", 2)}`;
    const arg = getToolArg(tool);
    return arg ? `${prefix}: ${arg}` : `${prefix}: working…`;
  }

  const out = (tool.output || "").trim();

  if (tool.name === "Read") {
    const file = (inp.file_path || inp.path || "") as string;
    return `Read ${shortPath(file)}`;
  }
  if (tool.name === "Glob") {
    const pattern = (inp.pattern as string) || "";
    const lines = out.split("\n").filter(Boolean);
    if (lines.length === 0) return `Glob ${pattern} → no files`;
    if (lines.length === 1) return `Glob → ${shortPath(lines[0])}`;
    return `Glob ${pattern} → ${lines.length} files`;
  }
  if (tool.name === "Grep") {
    const pattern = (inp.pattern as string) || "";
    const lines = out.split("\n").filter(Boolean);
    if (lines.length === 0) return `Grep "${pattern}" → no matches`;
    return `Grep "${pattern}" → ${lines.length} match${lines.length !== 1 ? "es" : ""}`;
  }

  // Serena tools — show argument + result summary
  if (tool.name.includes("find_symbol")) {
    const sym = (inp.symbol_name as string) || "";
    const fileMatch = out.match(
      /[\w./\\-]+\.(ts|tsx|js|jsx|py|go|rs|java|cpp|cs|rb)/,
    );
    if (fileMatch)
      return `${prefix}: ${sym} → ${fileMatch[0].split("/").pop()}`;
    const lines = out.split("\n").filter(Boolean);
    return lines.length
      ? `${prefix}: ${sym} → ${lines.length} result${lines.length !== 1 ? "s" : ""}`
      : `${prefix}: ${sym} → not found`;
  }
  if (tool.name.includes("search_for_pattern")) {
    const pat = (inp.pattern as string) || "";
    const lines = out.split("\n").filter(Boolean);
    return lines.length
      ? `${prefix}: "${pat}" → ${lines.length} result${lines.length !== 1 ? "s" : ""}`
      : `${prefix}: "${pat}" → no results`;
  }
  if (tool.name.includes("search_for_file")) {
    const name = (inp.file_name as string) || (inp.pattern as string) || "";
    const lines = out.split("\n").filter(Boolean);
    return lines.length
      ? `${prefix}: ${name} → ${lines.length} file${lines.length !== 1 ? "s" : ""}`
      : `${prefix}: ${name} → not found`;
  }
  if (tool.name.includes("get_document")) {
    const file = (inp.relative_path || inp.path || "") as string;
    return `${prefix}: read ${shortPath(file)}`;
  }
  if (tool.name.includes("get_symbols_overview")) {
    const path = (inp.relative_path || inp.path || "") as string;
    return `${prefix}: symbols in ${shortPath(path, 3)}`;
  }
  if (tool.name.includes("list_dir")) {
    const path = (inp.relative_path || inp.path || "") as string;
    return `${prefix}: listed ${shortPath(path, 3)}`;
  }
  if (tool.name.includes("activate_project")) {
    return `${prefix}: activated ${shortPath((inp.project_path as string) || "", 2)}`;
  }

  const arg = getToolArg(tool);
  return arg ? `${prefix}: ${arg}` : prefix;
}

// Extract the most relevant argument from a tool call — show full detail like CLI
function extractFromPartialJson(partial: string): string {
  // Try to extract the value of common meaningful keys from a partial JSON string being streamed
  const keys = [
    "command",
    "pattern",
    "path",
    "file_path",
    "relative_path",
    "file",
    "name_path",
    "symbol_name",
    "query",
    "needle",
    "url",
    "message",
    "description",
  ];
  for (const key of keys) {
    const m = partial.match(new RegExp(`"${key}"\\s*:\\s*"([^"]{2,})`));
    if (m) {
      const val = m[1];
      return val.length > 80 ? val.slice(0, 77) + "…" : val;
    }
  }
  return "";
}

function getShellCommandInput(tool: ToolCall): string {
  const inputCommand =
    typeof tool.input.command === "string" ? tool.input.command : "";
  if (inputCommand) return inputCommand;
  const inputCmd = typeof tool.input.cmd === "string" ? tool.input.cmd : "";
  if (inputCmd) return inputCmd;
  if (Array.isArray(tool.input.commands)) {
    const commands = tool.input.commands
      .map((entry: unknown) => {
        if (typeof entry === "string") return entry;
        if (!entry || typeof entry !== "object") return "";
        const command = (entry as Record<string, unknown>).command;
        return typeof command === "string" ? command : "";
      })
      .filter(Boolean);
    if (commands.length > 0) return commands.join("\n");
  }
  if (tool.partialInput) return extractFromPartialJson(tool.partialInput);
  for (const event of [...(tool.commandEvents || [])].reverse()) {
    const eventCommand = event.block?.command || event.run?.command;
    if (typeof eventCommand === "string" && eventCommand.trim())
      return eventCommand;
  }
  const action =
    typeof tool.input.action === "string" ? tool.input.action : "shell";
  const sessionId =
    typeof tool.input.session_id === "string" ? tool.input.session_id : "";
  return sessionId ? `${action} session ${sessionId}` : action;
}

function getToolArg(tool: ToolCall): string {
  const inp = tool.input;
  const action = inp.action as string | undefined;

  // Agent: show subagent_type + description
  if (tool.name === "Agent" || tool.name === "mcp__flamel__Agent") {
    const subtype = (inp.subagent_type || "") as string;
    const desc = (inp.description || "") as string;
    if (subtype && desc)
      return `${subtype} · ${desc.length > 80 ? desc.slice(0, 77) + "…" : desc}`;
    return subtype || desc || "";
  }
  // AgentBatch: show batch description + agent count
  if (tool.name === "AgentBatch" || tool.name === "mcp__flamel__AgentBatch") {
    const desc = (inp.description || "") as string;
    const agents = Array.isArray(inp.agents) ? inp.agents.length : 0;
    return `batch${agents ? ` (${agents} agents)` : ""} · ${desc.length > 80 ? desc.slice(0, 77) + "…" : desc}`;
  }

  // Consolidated tool dispatch — use the action param to determine what to show
  if (action) {
    if (action === "exec" || action === "run" || action === "start") {
      const cmd = (inp.command || "") as string;
      return cmd.length > 120 ? cmd.slice(0, 117) + "…" : cmd;
    }
    if (action === "search") {
      const pat = (inp.pattern || "") as string;
      const path = (inp.path || inp.relative_path || "") as string;
      const parts = pat ? [`"${pat}"`] : [];
      if (path) parts.push(shortPath(path, 3));
      if (inp.include) parts.push(`(${inp.include})`);
      return parts.join(" ") || "";
    }
    if (action === "glob" || action === "find") {
      const pat = (inp.pattern || inp.file_name || "") as string;
      const path = (inp.path || inp.relative_path || "") as string;
      return path ? `${pat} in ${shortPath(path, 3)}` : pat;
    }
    if (action === "list_dir" || action === "summary") {
      const path = (inp.relative_path || inp.path || inp.file || "") as string;
      return path ? shortPath(path, 3) : "";
    }
    if (action === "read" || action === "batch") {
      const path = (inp.file || inp.relative_path || "") as string;
      return path ? shortPath(path, 3) : "";
    }
    if (
      action === "replace" ||
      action === "replace_content" ||
      action === "replace_symbol" ||
      action === "insert_before" ||
      action === "insert_after" ||
      action === "create" ||
      action === "diff_apply" ||
      action === "format" ||
      action === "preview"
    ) {
      const path = (inp.file || inp.relative_path || "") as string;
      return path ? shortPath(path, 3) : "";
    }
    if (
      action === "find" ||
      action === "refs" ||
      action === "symbols" ||
      action === "context" ||
      action === "deps"
    ) {
      const sym = (inp.name_path ||
        inp.symbol_name ||
        inp.relative_path ||
        inp.file ||
        "") as string;
      return sym ? sym : "";
    }
    if (action === "imports") {
      const path = (inp.file || inp.relative_path || "") as string;
      return path ? shortPath(path, 3) : "";
    }
  }

  // Grep/Glob: show pattern + scope
  if (tool.name === "Grep") {
    const pat = (inp.pattern || "") as string;
    const path = (inp.path || "") as string;
    const parts = [`"${pat}"`];
    if (path) parts.push(shortPath(path, 3));
    if (inp.glob) parts.push(`(${inp.glob})`);
    return parts.join(" ");
  }
  if (tool.name === "Glob") {
    const pat = (inp.pattern || "") as string;
    const path = (inp.path || "") as string;
    return path ? `${pat} in ${shortPath(path, 3)}` : pat;
  }

  // Bash: show command
  if (tool.name === "Bash") {
    const cmd = (inp.command || "") as string;
    return cmd.length > 120 ? cmd.slice(0, 117) + "…" : cmd;
  }

  // Shell/Bash: show command
  if (tool.name.includes("shell") || tool.name.includes("execute_shell")) {
    const cmd = (inp.command || "") as string;
    return cmd.length > 120 ? cmd.slice(0, 117) + "…" : cmd;
  }

  // Search/grep: show pattern + scope
  if (tool.name.includes("search") || tool.name.includes("grep")) {
    const pat = (inp.pattern || "") as string;
    const path = (inp.path || inp.relative_path || "") as string;
    const parts = pat ? [`"${pat}"`] : [];
    if (path) parts.push(shortPath(path as string, 3));
    if (inp.include) parts.push(`(${inp.include})`);
    return parts.join(" ") || "";
  }

  // Glob/find_file: show pattern
  if (tool.name.includes("glob") || tool.name.includes("find_file")) {
    const pat = (inp.pattern || inp.file_name || "") as string;
    const path = (inp.path || "") as string;
    return path ? `${pat} in ${shortPath(path as string, 3)}` : pat;
  }

  if (tool.name === "file_edit" && Array.isArray(inp.changes)) {
    const paths = (inp.changes as any[])
      .map((change) => change?.path)
      .filter(
        (path): path is string => typeof path === "string" && path.length > 0,
      );
    if (paths.length === 1) return shortPath(paths[0], 3);
    if (paths.length > 1) return `${paths.length} files`;
  }

  // Read/Edit/Write/replace: show file path
  const filePath = (inp.file_path ??
    inp.path ??
    inp.relative_path ??
    inp.file ??
    "") as string;
  if (filePath) return shortPath(filePath, 3);

  // Symbol tools: name_path or symbol_name
  const symName = (inp.name_path ?? inp.symbol_name ?? "") as string;
  if (symName) return symName;

  // git_add: show files list
  if (Array.isArray(inp.files)) {
    const files = inp.files as string[];
    return files.length === 1 ? files[0] : `${files.length} files`;
  }

  // git_commit: show message
  if (inp.message) {
    const msg = inp.message as string;
    return msg.length > 80 ? msg.slice(0, 77) + "…" : msg;
  }

  // git_push/pull: show remote + branch
  if (inp.remote || inp.branch) {
    return [inp.remote, inp.branch].filter(Boolean).join("/") as string;
  }

  // Memory graph: entity names/counts
  if (Array.isArray(inp.entities))
    return `${(inp.entities as any[]).length} entities`;
  if (Array.isArray(inp.entityNames))
    return (inp.entityNames as string[]).slice(0, 3).join(", ");
  if (Array.isArray(inp.names))
    return (inp.names as string[]).slice(0, 3).join(", ");

  // Fallback: common single-value keys
  const raw = (inp.query ??
    inp.command ??
    inp.pattern ??
    inp.needle ??
    inp.url ??
    inp.thought ??
    inp.project_path ??
    "") as string;
  return raw.length > 120 ? raw.slice(0, 117) + "…" : raw;
}

// LCS-based line diff — generates edit script between two arrays of lines
function lcsLineDiff(
  oldLines: string[],
  newLines: string[],
): Array<{ op: "eq" | "add" | "del"; line: string }> {
  const m = oldLines.length,
    n = newLines.length;
  const dp: number[][] = Array.from({ length: m + 1 }, () =>
    new Array(n + 1).fill(0),
  );
  for (let i = m - 1; i >= 0; i--)
    for (let j = n - 1; j >= 0; j--)
      dp[i][j] =
        oldLines[i] === newLines[j]
          ? dp[i + 1][j + 1] + 1
          : Math.max(dp[i + 1][j], dp[i][j + 1]);
  const result: Array<{ op: "eq" | "add" | "del"; line: string }> = [];
  let i = 0,
    j = 0;
  while (i < m || j < n) {
    if (i < m && j < n && oldLines[i] === newLines[j]) {
      result.push({ op: "eq", line: oldLines[i++] });
      j++;
    } else if (j < n && (i >= m || dp[i + 1][j] >= dp[i][j + 1])) {
      result.push({ op: "add", line: newLines[j++] });
    } else {
      result.push({ op: "del", line: oldLines[i++] });
    }
  }
  return result;
}

// Build a unified-diff string from two text blobs (used for Edit/Write tool inputs)
function buildInlineDiff(
  file: string,
  oldStr: string,
  newStr: string,
  ctx = 3,
): string {
  const oldLines = oldStr ? oldStr.split("\n") : [];
  const newLines = newStr ? newStr.split("\n") : [];
  if (oldLines.length + newLines.length > 800) return ""; // bail on huge inputs
  const ops = lcsLineDiff(oldLines, newLines);
  if (!ops.some((o) => o.op !== "eq")) return "";
  type R = {
    op: "eq" | "add" | "del";
    line: string;
    oldN: number;
    newN: number;
  };
  const raw: R[] = [];
  let on = 1,
    nn = 1;
  for (const o of ops) {
    raw.push({ ...o, oldN: on, newN: nn });
    if (o.op !== "add") on++;
    if (o.op !== "del") nn++;
  }
  const changed = raw
    .map((r, i) => (r.op !== "eq" ? i : -1))
    .filter((i) => i >= 0);
  const hunks: Array<{ s: number; e: number }> = [];
  for (const ci of changed) {
    const s = Math.max(0, ci - ctx),
      e = Math.min(raw.length - 1, ci + ctx);
    if (hunks.length && s <= hunks[hunks.length - 1].e + 1)
      hunks[hunks.length - 1].e = e;
    else hunks.push({ s, e });
  }
  const out: string[] = [`--- a/${file}`, `+++ b/${file}`];
  for (const { s, e } of hunks) {
    const slice = raw.slice(s, e + 1);
    const os = slice.find((r) => r.op !== "add")?.oldN ?? 1;
    const ns = slice.find((r) => r.op !== "del")?.newN ?? 1;
    const oc = slice.filter((r) => r.op !== "add").length;
    const nc = slice.filter((r) => r.op !== "del").length;
    out.push(`@@ -${os},${oc} +${ns},${nc} @@`);
    for (const r of slice)
      out.push(`${r.op === "add" ? "+" : r.op === "del" ? "-" : " "}${r.line}`);
  }
  return out.join("\n");
}

// Extract old→new strings from file-editing tool inputs and produce a synthetic diff
const EDIT_TOOL_NAMES = new Set([
  "Edit",
  "Write",
  "file_edit",
  "flamel_replace",
  "multi_replace",
  "diff_apply",
  "mcp__flamel__file_edit",
  "mcp__flamel__replace_content",
  "mcp__flamel__multi_replace",
  "mcp__flamel__diff_apply",
  "mcp__flamel__create_text_file",
]);

function getEditDiff(tool: ToolCall): string | null {
  if (!EDIT_TOOL_NAMES.has(tool.name)) return null;
  const inp = tool.input;
  const action = (inp.action ?? "") as string;
  if (
    tool.name === "diff_apply" ||
    tool.name === "mcp__flamel__diff_apply" ||
    ((tool.name === "file_edit" || tool.name === "mcp__flamel__file_edit") &&
      action === "diff_apply")
  ) {
    const diff = (inp.diff ?? "") as string;
    return diff.trim() ? diff : null;
  }
  if (
    tool.name === "multi_replace" ||
    tool.name === "mcp__flamel__multi_replace" ||
    ((tool.name === "file_edit" || tool.name === "mcp__flamel__file_edit") &&
      action === "multi_replace")
  ) {
    const edits = Array.isArray(inp.edits) ? inp.edits : [];
    const diffs = edits
      .map((edit: any) => {
        const file = (edit?.file ??
          edit?.relative_path ??
          edit?.path ??
          "") as string;
        const old = (edit?.old_string ?? "") as string;
        const neu = (edit?.new_string ?? "") as string;
        if (!file || (!old && !neu)) return null;
        return buildInlineDiff(file, old, neu) || null;
      })
      .filter((diff: string | null): diff is string => !!diff);
    return diffs.length ? diffs.join("\n") : null;
  }
  const file = (inp.file_path ??
    inp.relative_path ??
    inp.path ??
    inp.file ??
    "") as string;
  if (!file) return null;
  if (
    tool.name === "Edit" ||
    tool.name === "flamel_replace" ||
    ((tool.name === "file_edit" || tool.name === "mcp__flamel__file_edit") &&
      action === "replace")
  ) {
    const old = (inp.old_string ?? "") as string;
    const neu = (inp.new_string ?? "") as string;
    if (!old && !neu) return null;
    return buildInlineDiff(file, old, neu) || null;
  }
  if (
    tool.name === "mcp__flamel__replace_content" ||
    ((tool.name === "file_edit" || tool.name === "mcp__flamel__file_edit") &&
      action === "replace_content")
  ) {
    const old = (inp.needle ?? inp.old_string ?? "") as string;
    const neu = (inp.repl ?? "") as string;
    if (!old && !neu) return null;
    return buildInlineDiff(file, old, neu) || null;
  }
  if (
    tool.name === "Write" ||
    tool.name === "mcp__flamel__create_text_file" ||
    ((tool.name === "file_edit" || tool.name === "mcp__flamel__file_edit") &&
      action === "create")
  ) {
    const content = (inp.content ?? "") as string;
    if (!content || content.split("\n").length > 400) return null;
    return buildInlineDiff(file, "", content) || null;
  }
  return null;
}

function isUnifiedDiff(text: string): boolean {
  return (
    text.includes("--- a/") && text.includes("+++ b/") && text.includes("@@ ")
  );
}

function DiffViewer({ text }: { text: string }) {
  return (
    <React.Suspense fallback={null}>
      <EmbeddedDiffView patch={text} maximumHeight={380} />
    </React.Suspense>
  );
}

function ToolImageArtifacts({
  paths,
  artifacts = [],
}: {
  paths: string[];
  artifacts?: NonNullable<ToolCall["artifacts"]>;
}) {
  const [previews, setPreviews] = useState<
    Array<{ path: string; url?: string; error?: string }>
  >([]);
  const managedImages = artifacts.filter(
    (artifact) =>
      artifact.kind === "image" && artifact.mediaType.startsWith("image/"),
  );
  const pathKey = paths.join("\n");

  useEffect(() => {
    let cancelled = false;
    const objectUrls: string[] = [];
    const currentPaths =
      managedImages.length === 0 && pathKey ? pathKey.split("\n") : [];

    if (currentPaths.length === 0) {
      setPreviews([]);
      return;
    }

    setPreviews(currentPaths.map((path) => ({ path })));

    Promise.all(
      currentPaths.map(async (path) => {
        try {
          const response = await apiFetch(fileAssetPath(path));
          if (!response.ok) {
            throw new Error(`${response.status} ${response.statusText}`.trim());
          }
          const blob = await response.blob();
          const url = URL.createObjectURL(blob);
          objectUrls.push(url);
          return { path, url };
        } catch (error) {
          return {
            path,
            error: error instanceof Error ? error.message : String(error),
          };
        }
      }),
    ).then((loaded) => {
      if (cancelled) {
        loaded.forEach((preview) => {
          if (preview.url) URL.revokeObjectURL(preview.url);
        });
        return;
      }
      setPreviews(loaded);
    });

    return () => {
      cancelled = true;
      objectUrls.forEach((url) => URL.revokeObjectURL(url));
    };
  }, [pathKey, managedImages.length]);

  if (managedImages.length > 0) {
    return (
      <div
        style={{
          display: "grid",
          gridTemplateColumns: "repeat(auto-fit, minmax(180px, 320px))",
          gap: 8,
          padding: "8px 10px 10px",
        }}
      >
        {managedImages.map((artifact) => (
          <a
            key={artifact.id}
            href={artifact.contentUrl}
            target="_blank"
            rel="noreferrer"
            title={`Open ${artifact.name}`}
            style={{
              display: "block",
              border: "1px solid var(--border)",
              borderRadius: 6,
              overflow: "hidden",
              background: "var(--bg)",
            }}
          >
            <img
              src={artifact.contentUrl}
              alt={artifact.name}
              loading="lazy"
              style={{
                width: "100%",
                maxHeight: 360,
                display: "block",
                objectFit: "contain",
              }}
            />
          </a>
        ))}
      </div>
    );
  }

  if (paths.length === 0) return null;

  return (
    <div
      style={{
        background: "var(--surface)",
        borderTop: "1px solid var(--border-soft)",
        padding: "8px 12px 10px",
        display: "grid",
        gap: 8,
      }}
    >
      {previews.map((preview, index) => {
        const label =
          preview.path.split("/").pop() || `image artifact ${index + 1}`;
        if (preview.error) {
          return (
            <div
              key={preview.path}
              style={{
                color: "var(--error)",
                fontFamily: "var(--font-mono)",
                fontSize: 10,
              }}
              title={preview.path}
            >
              image artifact unavailable: {preview.error}
            </div>
          );
        }
        if (!preview.url) {
          return (
            <div
              key={preview.path}
              style={{
                color: "var(--text-dim)",
                fontFamily: "var(--font-mono)",
                fontSize: 10,
              }}
              title={preview.path}
            >
              loading image artifact…
            </div>
          );
        }
        return (
          <a
            key={preview.path}
            href={preview.url}
            target="_blank"
            rel="noreferrer"
            style={{
              color: "inherit",
              display: "block",
              textDecoration: "none",
            }}
          >
            <img
              src={preview.url}
              alt={label}
              loading="lazy"
              style={{
                background: "var(--surface-2)",
                border: "1px solid var(--border-soft)",
                borderRadius: "var(--radius-md)",
                display: "block",
                maxHeight: 420,
                maxWidth: "100%",
                objectFit: "contain",
              }}
            />
            <div
              style={{
                color: "var(--text-dim)",
                fontFamily: "var(--font-mono)",
                fontSize: 10,
                marginTop: 4,
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
              }}
              title={preview.path}
            >
              {preview.path}
            </div>
          </a>
        );
      })}
    </div>
  );
}

// Compatibility-only renderer for tools that predate the authored widget registry.
export const LegacyToolRow = React.memo(function LegacyToolRow({
  tool,
}: {
  tool: ToolCall;
}) {
  const [expanded, setExpanded] = useState(false);

  const isRunning = tool.status === "running" || tool.status === "pending";
  const isError = tool.status === "error";
  const isConsoleTool = isShellLikeToolCall(tool);

  const dotColor = isError
    ? "var(--error)"
    : isRunning
      ? "var(--accent)"
      : "var(--success)";
  const borderColor = isRunning
    ? "var(--accent-border)"
    : isError
      ? "rgba(255,92,117,0.34)"
      : "var(--border-soft)";
  const statusLabel = isRunning
    ? isConsoleTool
      ? "live"
      : "running"
    : isError
      ? "error"
      : "done";
  const statusTitle = isRunning
    ? isConsoleTool
      ? "Command output is streaming in chat"
      : "Tool call is running"
    : isError
      ? "Tool call failed"
      : "Tool call finished";
  const statusBackground = isRunning
    ? "var(--accent-glow)"
    : isError
      ? "rgba(255,92,117,0.10)"
      : "rgba(46,207,115,0.10)";
  const statusBorder = isRunning
    ? "var(--accent-border)"
    : isError
      ? "rgba(255,92,117,0.26)"
      : "rgba(46,207,115,0.24)";

  const { server, label, isReclaude } = parseToolName(
    tool.name,
    tool.input?.action as string | undefined,
    tool.input as Record<string, unknown>,
  );
  const arg = getToolArg(tool);

  // Synthetic edit diffs are previews of the requested input, so only show them after success.
  const editDiff = !isRunning && !isError ? getEditDiff(tool) : null;
  const outputIsUnifiedDiff = !!tool.output && isUnifiedDiff(tool.output);
  const outputIsPlanMarker = !isError && isPlanToolMarkerOutput(tool.output);
  const imageArtifactPaths =
    tool.output && !outputIsUnifiedDiff && !outputIsPlanMarker
      ? extractImageArtifactPaths(tool.output)
      : [];

  // Prefer synthetic diff stat over the one stored on the tool (which comes from output)
  const displayStat: { added: number; removed: number } | null = (() => {
    if (isError) return null;
    if (tool.diffStat) return tool.diffStat;
    if (!editDiff) return null;
    let added = 0,
      removed = 0;
    for (const line of editDiff.split("\n")) {
      if (line.startsWith("+") && !line.startsWith("+++")) added++;
      else if (line.startsWith("-") && !line.startsWith("---")) removed++;
    }
    return added + removed > 0 ? { added, removed } : null;
  })();

  const hasInput = Object.keys(tool.input).length > 0 || !!tool.partialInput;
  const shellCommandInput = isConsoleTool ? getShellCommandInput(tool) : "";
  const rendersInputBlock = hasInput && !isConsoleTool;
  const showConsoleOutput = isConsoleTool;
  const syntheticConsoleEvents = React.useMemo(
    () =>
      showConsoleOutput
        ? toolCallToCommandEvents(tool, {
            title: server ? `${server} ${label}` : label,
            source: server || "chat",
          })
        : [],
    [showConsoleOutput, tool, server, label],
  );
  const consoleEvents = React.useMemo(() => {
    if (!showConsoleOutput) return [];
    const liveEvents = tool.commandEvents || [];
    const liveHasOutput = liveEvents.some(
      (event) =>
        event.kind === "output_chunk" ||
        event.kind === "input_sent" ||
        event.kind === "error",
    );
    return liveEvents.length > 0 && (isRunning || liveHasOutput)
      ? liveEvents
      : syntheticConsoleEvents;
  }, [
    isRunning,
    showConsoleOutput,
    syntheticConsoleEvents,
    tool.commandEvents,
  ]);

  if (showConsoleOutput) {
    return (
      <div style={{ margin: "6px 0", minWidth: 0 }}>
        <InteractiveConsole
          events={consoleEvents}
          connected={!isError}
          title={isError ? "Shell error" : "Shell output"}
          height={220}
          compact
          flattenSingleBlock
          commandFirst
          promptCommand={shellCommandInput || undefined}
        />
        <ToolImageArtifacts
          paths={imageArtifactPaths}
          artifacts={tool.artifacts}
        />
      </div>
    );
  }

  return (
    <div
      style={{
        margin: "5px 0",
        border: `1px solid ${borderColor}`,
        borderRadius: 7,
        background: "var(--surface)",
        boxShadow: isRunning ? "0 0 0 1px var(--accent-glow)" : "none",
        transition:
          "border-color var(--transition-base), box-shadow var(--transition-base)",
        overflow: "hidden",
        minWidth: 0,
      }}
    >
      <div
        className="flex items-center cursor-pointer"
        style={{ minHeight: 30, gap: 7, padding: "0 8px", minWidth: 0 }}
        onClick={() => setExpanded((e) => !e)}
        onMouseEnter={(e) =>
          (e.currentTarget.style.background = "var(--surface-2)")
        }
        onMouseLeave={(e) =>
          (e.currentTarget.style.background = expanded
            ? "var(--surface-2)"
            : "transparent")
        }
      >
        <span
          title={statusTitle}
          style={{
            width: 7,
            height: 7,
            borderRadius: "50%",
            background: dotColor,
            boxShadow: isRunning
              ? "0 0 8px var(--accent-glow-strong)"
              : isError
                ? "0 0 7px rgba(255,92,117,0.35)"
                : "0 0 7px rgba(46,207,115,0.28)",
            flexShrink: 0,
          }}
        />

        <span
          style={{
            color: "var(--text-dim)",
            display: "flex",
            flexShrink: 0,
            opacity: 0.72,
          }}
        >
          {expanded ? (
            <Icon.ChevronDown size={11} />
          ) : (
            <Icon.ChevronRight size={11} />
          )}
        </span>

        <span
          style={{
            display: "inline-flex",
            alignItems: "center",
            gap: 5,
            minWidth: 0,
            flex: "0 1 auto",
          }}
        >
          {isReclaude ? (
            <span
              style={{
                color: "var(--accent)",
                fontSize: 11,
                fontWeight: 650,
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
              }}
            >
              {label}
            </span>
          ) : server ? (
            <>
              <span
                style={{
                  color: "var(--accent)",
                  fontSize: 10,
                  fontWeight: 700,
                  border: "1px solid var(--accent-border)",
                  background: "var(--accent-glow)",
                  borderRadius: 5,
                  padding: "1px 5px",
                  lineHeight: 1.3,
                  flexShrink: 0,
                }}
              >
                {server}
              </span>
              <span
                style={{
                  color: "var(--text)",
                  fontSize: 11,
                  fontWeight: 600,
                  overflow: "hidden",
                  textOverflow: "ellipsis",
                  whiteSpace: "nowrap",
                }}
              >
                {label}
              </span>
            </>
          ) : (
            <span
              style={{
                color: "var(--text)",
                fontSize: 11,
                fontWeight: 600,
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
              }}
            >
              {label}
            </span>
          )}
        </span>

        {(() => {
          const partialArg = tool.partialInput
            ? extractFromPartialJson(tool.partialInput)
            : "";
          const display = arg || partialArg;
          if (!display) return null;
          return (
            <span
              style={{
                color: "var(--text-dim)",
                fontSize: 10,
                fontFamily: "var(--font-mono)",
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
                flex: 1,
                minWidth: 24,
                opacity: arg ? 1 : 0.58,
              }}
            >
              {display}
            </span>
          );
        })()}

        <span
          style={{
            marginLeft: "auto",
            display: "grid",
            gridTemplateColumns: "58px 62px",
            gap: 6,
            alignItems: "center",
            justifyContent: "end",
            flex: "0 0 126px",
          }}
        >
          <span
            aria-hidden={!displayStat}
            style={{
              visibility: displayStat ? "visible" : "hidden",
              display: "inline-flex",
              alignItems: "center",
              justifyContent: "center",
              gap: 4,
              minHeight: 18,
              padding: "1px 6px",
              border: "1px solid var(--border-soft)",
              background: "var(--surface-2)",
              borderRadius: 5,
              fontSize: 10,
              fontFamily: "var(--font-mono)",
            }}
          >
            <span style={{ color: "var(--success)" }}>
              +{displayStat?.added ?? 0}
            </span>
            <span style={{ color: "var(--error)" }}>
              −{displayStat?.removed ?? 0}
            </span>
          </span>
          <span
            title={statusTitle}
            style={{
              minHeight: 18,
              display: "inline-flex",
              alignItems: "center",
              justifyContent: "center",
              padding: "1px 6px",
              borderRadius: 5,
              border: `1px solid ${statusBorder}`,
              background: statusBackground,
              color: dotColor,
              fontSize: 10,
              fontWeight: 700,
              lineHeight: 1,
              textTransform: "uppercase",
            }}
          >
            {statusLabel}
          </span>
        </span>
      </div>
      {expanded && (
        <div
          style={{
            padding: "0 8px 8px 28px",
            borderTop: "1px solid var(--border-soft)",
          }}
        >
          {editDiff ? (
            /* File-editing tool: show inline diff instead of raw input/output */
            <DiffViewer text={editDiff} />
          ) : (
            <>
              {rendersInputBlock && (
                <pre
                  style={{
                    background: "var(--surface)",
                    color: "var(--text-muted)",
                    fontFamily: "var(--font-mono)",
                    fontSize: 11,
                    padding: "6px 12px",
                    maxHeight: 200,
                    overflowY: "auto",
                    overflowX: "auto",
                    borderBottom: tool.output
                      ? "1px solid var(--border-soft)"
                      : undefined,
                    margin: "8px 0 0",
                    borderRadius: "var(--radius-sm) var(--radius-sm) 0 0",
                  }}
                >
                  <span
                    style={{
                      color: "var(--accent)",
                      opacity: 0.7,
                      fontSize: 10,
                      display: "block",
                      marginBottom: 4,
                    }}
                  >
                    INPUT{tool.partialInput ? " (building...)" : ""}
                  </span>
                  {tool.partialInput
                    ? tool.partialInput
                    : JSON.stringify(tool.input, null, 2)}
                </pre>
              )}
              {(tool.output ||
                (showConsoleOutput && (isRunning || shellCommandInput))) &&
                (outputIsPlanMarker ? (
                  <div
                    style={{
                      background: "var(--surface)",
                      color: "var(--text-muted)",
                      fontSize: 11,
                      padding: "7px 12px",
                      margin: rendersInputBlock ? 0 : "8px 0 0",
                      borderRadius: rendersInputBlock
                        ? "0 0 var(--radius-sm) var(--radius-sm)"
                        : "var(--radius-sm)",
                    }}
                  >
                    <span style={{ color: "var(--accent)", fontWeight: 600 }}>
                      {planToolOutputLabel(tool.output)}
                    </span>
                  </div>
                ) : outputIsUnifiedDiff ? (
                  <div
                    style={{
                      margin: rendersInputBlock ? 0 : "8px 0 0",
                      borderRadius: rendersInputBlock
                        ? "0 0 var(--radius-sm) var(--radius-sm)"
                        : "var(--radius-sm)",
                      overflow: "hidden",
                      borderTop: rendersInputBlock
                        ? "1px solid var(--border-soft)"
                        : undefined,
                    }}
                  >
                    <DiffViewer text={tool.output || ""} />
                  </div>
                ) : showConsoleOutput ? (
                  <>
                    <InteractiveConsole
                      events={consoleEvents}
                      connected={!isError}
                      title={isError ? "Shell error" : "Shell output"}
                      height={220}
                      compact
                      flattenSingleBlock
                      promptCommand={shellCommandInput || undefined}
                    />
                    <ToolImageArtifacts
                      paths={imageArtifactPaths}
                      artifacts={tool.artifacts}
                    />
                  </>
                ) : (
                  <>
                    <pre
                      style={{
                        background: "var(--surface)",
                        color: "var(--text-muted)",
                        fontFamily: "var(--font-mono)",
                        fontSize: 11,
                        padding: "6px 12px",
                        maxHeight: 300,
                        overflowY: "auto",
                        overflowX: "auto",
                        margin: rendersInputBlock ? 0 : "8px 0 0",
                        borderBottom:
                          imageArtifactPaths.length > 0
                            ? "1px solid var(--border-soft)"
                            : undefined,
                        borderRadius:
                          imageArtifactPaths.length > 0
                            ? rendersInputBlock
                              ? "0"
                              : "var(--radius-sm) var(--radius-sm) 0 0"
                            : rendersInputBlock
                              ? "0 0 var(--radius-sm) var(--radius-sm)"
                              : "var(--radius-sm)",
                      }}
                    >
                      <span
                        style={{
                          color: isError ? "var(--error)" : "var(--success)",
                          opacity: 0.7,
                          fontSize: 10,
                          display: "block",
                          marginBottom: 4,
                        }}
                      >
                        {isError ? "ERROR" : "OUTPUT"}
                      </span>
                      {tool.output?.slice(0, 3000)}
                    </pre>
                    <ToolImageArtifacts
                      paths={imageArtifactPaths}
                      artifacts={tool.artifacts}
                    />
                  </>
                ))}
              {!tool.output && isRunning && !showConsoleOutput && (
                <div
                  style={{
                    padding: "8px 12px 0",
                    color: "var(--text-dim)",
                    fontSize: 11,
                  }}
                >
                  running…
                </div>
              )}
            </>
          )}
        </div>
      )}
    </div>
  );
});
