import type { ToolCall } from "./messageProcessor";
import {
  mergeToolOutput,
  normalizeToolPresentation,
  textFromToolResultContent,
} from "./messageProcessor";
import { stripAgentMarkerHeaders } from "./workflowEvents";

export interface ToolResultEventLike {
  tool_use_id: string;
  content?: unknown;
  presentation?: unknown;
  artifacts?: unknown;
  is_error?: boolean;
  is_delta?: boolean;
}

export interface ToolResultReduction {
  tool: ToolCall;
  output: string;
  isDelta: boolean;
  runId?: string;
}

export interface AppliedToolResult extends ToolResultReduction {
  tools: ToolCall[];
  previousTool: ToolCall;
  index: number;
  changed: boolean;
}

export interface ApplyToolResultOptions {
  decorate?: (tool: ToolCall, reduction: ToolResultReduction) => ToolCall;
}

function sameSerializable(left: unknown, right: unknown): boolean {
  if (left === right) return true;
  try {
    return JSON.stringify(left) === JSON.stringify(right);
  } catch {
    return false;
  }
}

function sameResultState(left: ToolCall, right: ToolCall): boolean {
  return (
    left.status === right.status &&
    left.output === right.output &&
    left.runId === right.runId &&
    sameSerializable(left.diffStat, right.diffStat) &&
    sameSerializable(left.presentation, right.presentation) &&
    sameSerializable(left.artifacts, right.artifacts) &&
    sameSerializable(left.nestedEvents, right.nestedEvents)
  );
}

export function parseToolDiffStat(
  output: string,
): { added: number; removed: number } | undefined {
  if (!output.includes("--- a/") || !output.includes("+++ b/"))
    return undefined;

  let added = 0;
  let removed = 0;
  for (const line of output.split("\n")) {
    if (line.startsWith("+") && !line.startsWith("+++")) added += 1;
    else if (line.startsWith("-") && !line.startsWith("---")) removed += 1;
  }
  return added > 0 || removed > 0 ? { added, removed } : undefined;
}

export function reduceToolResult(
  tool: ToolCall,
  result: ToolResultEventLike,
): ToolResultReduction {
  const isDelta = result.is_delta === true;
  if (tool.status === "done" || tool.status === "error") {
    return {
      tool,
      output: tool.output ?? "",
      isDelta,
      runId: tool.runId,
    };
  }

  const rawOutput = textFromToolResultContent(result.content);
  const agentMarker = isDelta
    ? { output: rawOutput, runId: undefined }
    : stripAgentMarkerHeaders(rawOutput, tool.name);
  const mergedOutput = mergeToolOutput(tool.output, agentMarker.output, isDelta);
  const retainedArtifacts =
    Array.isArray(result.artifacts) && result.artifacts.length > 0
      ? result.artifacts
      : tool.artifacts;
  const normalized = normalizeToolPresentation({
    content: result.content,
    envelope: result.presentation,
    fallbackText: mergedOutput,
    artifacts: retainedArtifacts,
    isError: result.is_error === true,
  });
  const output = normalized.text;
  const previousPresentation = tool.presentation;
  const presentation = {
    ...normalized,
    structuredContent:
      normalized.structuredContent ?? previousPresentation?.structuredContent,
    resources:
      normalized.resources.length > 0
        ? normalized.resources
        : (previousPresentation?.resources ?? []),
    media:
      normalized.media.length > 0
        ? normalized.media
        : (previousPresentation?.media ?? []),
    artifacts:
      normalized.artifacts.length > 0
        ? normalized.artifacts
        : (previousPresentation?.artifacts ?? tool.artifacts ?? []),
    transport: normalized.transport ?? previousPresentation?.transport,
    truncated: normalized.truncated || previousPresentation?.truncated === true,
    legacy:
      normalized.legacy &&
      (previousPresentation ? previousPresentation.legacy : true),
  };
  const diffStat = parseToolDiffStat(output) ?? tool.diffStat;
  const runId = agentMarker.runId ?? tool.runId;
  const nextTool: ToolCall = {
    ...tool,
    status: isDelta ? "running" : result.is_error ? "error" : "done",
    output,
    presentation,
    ...(presentation.artifacts.length > 0
      ? { artifacts: presentation.artifacts }
      : {}),
    ...(diffStat ? { diffStat } : {}),
    ...(runId ? { runId } : {}),
  };

  return {
    tool: sameResultState(tool, nextTool) ? tool : nextTool,
    output,
    isDelta,
    runId,
  };
}

export function applyToolResult(
  tools: ToolCall[],
  result: ToolResultEventLike,
  options: ApplyToolResultOptions = {},
): AppliedToolResult | undefined {
  const index = tools.findIndex((tool) => tool.id === result.tool_use_id);
  if (index < 0) return undefined;

  const previousTool = tools[index];
  const reduction = reduceToolResult(previousTool, result);
  const decorated = options.decorate
    ? options.decorate(reduction.tool, reduction)
    : reduction.tool;
  const tool = sameResultState(previousTool, decorated)
    ? previousTool
    : decorated;
  const changed = tool !== previousTool;
  const nextTools = changed
    ? tools.map((candidate, candidateIndex) =>
        candidateIndex === index ? tool : candidate,
      )
    : tools;

  return {
    ...reduction,
    tool,
    tools: nextTools,
    previousTool,
    index,
    changed,
  };
}
