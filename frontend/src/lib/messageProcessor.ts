import type { SdkMessage, PlanItem } from "@flamel/shared";
import type { CommandEvent } from "./consoleTypes";
import type {
  ToolArtifact,
  ToolPresentationData,
} from "./toolPresentation";

export {
  normalizeToolPresentation,
  textFromToolResultContent,
} from "./toolPresentation";
export type {
  ToolArtifact,
  ToolMediaReference,
  ToolPresentationData,
  ToolResourceLink,
  ToolResultEnvelopeV1,
} from "./toolPresentation";

export type ContentSegment =
  | { type: "text"; text: string }
  | { type: "tool_ref"; toolId: string }
  | { type: "thinking"; text: string; redacted?: boolean };

export function appendThinkingSegment(
  segments: ContentSegment[],
  text: string,
  redacted: boolean,
) {
  if (!text && redacted) {
    const alreadyHasEmptyRedacted = segments.some(
      (segment) =>
        segment.type === "thinking" && segment.redacted && !segment.text,
    );
    if (alreadyHasEmptyRedacted) return;
  }
  const previous = segments[segments.length - 1];
  if (
    text &&
    !redacted &&
    previous?.type === "thinking" &&
    !previous.redacted
  ) {
    if (previous.text.trimEnd() === text.trimEnd()) return;
    previous.text += previous.text ? `\n\n${text}` : text;
    return;
  }
  segments.push({ type: "thinking", text, redacted });
}

export function mergeToolOutput(
  existing: string | undefined,
  incoming: string,
  isDelta: boolean,
): string {
  if (isDelta) return `${existing || ""}${incoming}`;
  return incoming || existing || "";
}

export interface CompactionDetails {
  runId?: string;
  providerNative?: boolean;
  phase?: string;
  label?: string;
  summary?: string;
  summaryChars?: number;
  preTokens?: number;
  model?: string;
  reasoningEffort?: string;
  modelRole?: string;
  fallbackUsed?: boolean;
  fallbackEnabled?: boolean;
  summarizedTurns?: number;
  preservedRecentMessages?: number;
  estimatedSummaryTokens?: number;
  estimatedRecoveredTokens?: number;
  recoveryStrategy?: string;
  queuedPrompt?: boolean;
}

export interface DisplayBlock {
  id: string;
  requestId?: string;
  role: "user" | "assistant" | "system";
  type: "user" | "assistant" | "system" | "compact";
  content?: string;
  images?: Array<{ data: string; mediaType: string; name?: string }>;
  thinking?: string;
  tools?: ToolCall[];
  segments?: ContentSegment[];
  usage?: {
    inputTokens: number;
    outputTokens: number;
    cacheReadTokens?: number;
    cacheCreationTokens?: number;
  };
  model?: string;
  duration?: number;
  isStreaming?: boolean;
  isStarted?: boolean; // true once MCP servers initialized and Claude is running
  mcpServerNames?: string[]; // names of MCP servers being initialized
  isWaiting?: boolean;
  statusText?: string; // latest activity breadcrumb (tool name being used)
  recoveryNote?: string; // visible marker when the run resumed or compacted context
  recoverySummary?: string; // expandable recovery digest shown with the note when available
  preTokens?: number; // for type='compact': context size before compaction
  compaction?: CompactionDetails; // live/final compaction summary and checkpoint metadata
  stopReason?: string; // 'user_interrupted' | 'end_turn' | 'max_tokens' | 'error' | 'process_died'
}

export interface ToolCall {
  id: string;
  name: string;
  input: Record<string, unknown>;
  partialInput?: string; // raw JSON string building up before input is finalized
  output?: string;
  status: "pending" | "running" | "done" | "error" | "unknown";
  diffStat?: { added: number; removed: number };
  // Sub-agent correlation: populated for mcp__flamel__Agent / Agent tool calls.
  // `runId` is extracted from the AGENT_RUN:{uuid} header on tool_result output;
  // `nestedEvents` is the stream-json event log from the child claude subprocess,
  // buffered live during execution and attached on completion.
  runId?: string;
  nestedEvents?: NestedAgentEvent[];
  commandEvents?: CommandEvent[];
  artifacts?: ToolArtifact[];
  presentation?: ToolPresentationData;
}

/// Events forwarded by flamel_mcp from a sub-agent's stream-json output.
/// Includes synthetic `nested_start` / `nested_done` / `nested_error` / `nested_raw`
/// events plus passthrough events from the child claude (`assistant`, `user`,
/// `result`, `system`, `stream_event`, etc.).
export type NestedAgentEvent = Record<string, unknown> & { type?: string };

export type HiddenControlDirective = "continue" | "complete";

const AUTO_CONTINUE_SENTINEL = "<flamel:auto-continue/>";
const HIDDEN_CONTROL_PREFIX = "<flamel:";

export function stripHiddenControlMarkers(content: string): string {
  let cleaned = content;
  if (cleaned.includes(AUTO_CONTINUE_SENTINEL)) {
    cleaned = cleaned.replaceAll(AUTO_CONTINUE_SENTINEL, "");
  }

  let cursor = 0;
  let output = "";
  while (true) {
    const start = cleaned.indexOf(HIDDEN_CONTROL_PREFIX, cursor);
    if (start < 0) {
      output += cleaned.slice(cursor);
      break;
    }
    output += cleaned.slice(cursor, start);
    const end = cleaned.indexOf(">", start);
    if (end < 0) {
      break;
    }
    cursor = end + 1;
  }

  return output.replace(/[ \t]+\n/g, "\n");
}

export function extractHiddenControlSignal(content: string): {
  content: string;
  shouldAutoContinue: boolean;
  continuationDirective: HiddenControlDirective | null;
} {
  let directive: HiddenControlDirective | null = null;
  const normalized = content.toLowerCase();
  if (normalized.includes("<flamel:continue/>")) directive = "continue";
  if (normalized.includes("<flamel:complete/>")) directive = "complete";
  const cleaned = stripHiddenControlMarkers(content).trimEnd();
  return {
    content: cleaned,
    shouldAutoContinue:
      directive === "continue" || content.includes(AUTO_CONTINUE_SENTINEL),
    continuationDirective: directive,
  };
}

export function extractPlanItems(toolCalls: ToolCall[]): PlanItem[] | null {
  const todoWrite = toolCalls.find(
    (t) => t.name.toLowerCase() === "todowrite" || t.name === "todo_write",
  );
  if (!todoWrite) return null;

  try {
    const todos = todoWrite.input.todos as any[];
    if (!Array.isArray(todos)) return null;
    return todos.map((t, i) => ({
      id: `${i}`,
      text: t.content || t.text || String(t),
      status: (t.status === "completed"
        ? "done"
        : t.status === "in_progress"
          ? "in_progress"
          : "todo") as PlanItem["status"],
      position: i,
    }));
  } catch {
    return null;
  }
}

// Parse markdown checkboxes from a Write tool call to a .claude/plans/*.md file
export function extractPlanFromWrite(tool: ToolCall): PlanItem[] | null {
  if (tool.name !== "Write" && tool.name !== "write_file") return null;
  const path = (tool.input.file_path || tool.input.path || "") as string;
  if (!path.includes("/.claude/plans/") && !path.includes("\\.claude\\plans\\"))
    return null;

  const content = (tool.input.content ||
    tool.input.new_content ||
    "") as string;
  if (!content) return null;

  const items: PlanItem[] = [];
  let idx = 0;
  for (const line of content.split("\n")) {
    const checked = line.match(/^\s*-\s*\[x\]\s*(.+)/i);
    const inprog = line.match(/^\s*-\s*\[>\]\s*(.+)/i);
    const unchecked = line.match(/^\s*-\s*\[ \]\s*(.+)/i);
    if (checked) {
      items.push({
        id: String(idx),
        text: checked[1].trim(),
        status: "done",
        position: idx++,
      });
    } else if (inprog) {
      items.push({
        id: String(idx),
        text: inprog[1].trim(),
        status: "in_progress",
        position: idx++,
      });
    } else if (unchecked) {
      items.push({
        id: String(idx),
        text: unchecked[1].trim(),
        status: "todo",
        position: idx++,
      });
    }
  }
  return items.length > 0 ? items : null;
}

export function parseNdjson(text: string): SdkMessage[] {
  return text
    .split("\n")
    .filter((l) => l.trim())
    .map((line) => {
      try {
        return JSON.parse(line);
      } catch {
        return null;
      }
    })
    .filter(Boolean);
}
