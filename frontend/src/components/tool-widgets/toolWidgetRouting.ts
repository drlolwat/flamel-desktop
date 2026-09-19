import { capabilityForTool, isToolRenderer, shortToolName } from "../../lib/toolCapabilities";
import type { ContentSegment, ToolCall } from "../../lib/messageProcessor";

export function routineActivityGroup(segments: ContentSegment[], start: number, tools: ReadonlyMap<string, ToolCall>): ToolCall[] {
  const group: ToolCall[] = [];
  for (let index = start; index < segments.length; index++) {
    const segment = segments[index];
    if (segment.type !== "tool_ref") break;
    const tool = tools.get(segment.toolId);
    if (!tool || tool.status !== "done") break;
    const capability = capabilityForTool(tool.name, tool.input);
    if (!capability.readOnly || !["file_read", "code_nav"].includes(capability.name)) break;
    group.push(tool);
  }
  return group.length >= 3 ? group : [];
}
import { isAdaptiveDataToolCall } from "./adaptiveToolAdapters";
import { isEngineeringToolCall } from "./engineeringToolAdapters";

const INVISIBLE_TOOLS = new Set(["ToolSearch", "TodoRead"]);
const INVISIBLE_PLAN_ACTIONS = new Set([
  "get",
  "list",
  "update_item",
  "add_items",
  "delete_item",
]);
const INVISIBLE_LEGACY_TOOLS = new Set([
  "flamel_plan_get",
  "flamel_plan_list",
  "flamel_todo_list",
  "flamel_plan_update_item",
  "flamel_plan_add_items",
  "flamel_plan_delete_item",
]);

export type WorkflowWidgetKind =
  | "assistant"
  | "automation"
  | "job"
  | "plan"
  | "work"
  | "agent"
  | "memory"
  | "skill"
  | "connection";

export type ToolRenderKind =
  | "hidden"
  | "thinking"
  | "question"
  | "viz"
  | "workflow"
  | "research"
  | "browser"
  | "media"
  | "engineering"
  | "adaptive"
  | "legacy";

function partialInput(tool: ToolCall): Record<string, unknown> {
  if (!tool.partialInput) return tool.input;
  try {
    const parsed = JSON.parse(tool.partialInput);
    return parsed && typeof parsed === "object" && !Array.isArray(parsed)
      ? { ...(parsed as Record<string, unknown>), ...tool.input }
      : tool.input;
  } catch {
    const action = tool.partialInput.match(/"action"\s*:\s*"([^"]+)/)?.[1];
    return action ? { action, ...tool.input } : tool.input;
  }
}

export function workflowAction(tool: ToolCall): string {
  const unprefixed = tool.name.includes("__")
    ? tool.name.split("__").pop()!
    : tool.name;
  if (unprefixed.startsWith("assistant.")) return unprefixed;
  const action = partialInput(tool).action;
  return typeof action === "string" && action
    ? action
    : shortToolName(tool.name);
}

export function workflowWidgetKind(
  tool: ToolCall,
): WorkflowWidgetKind | undefined {
  const action = workflowAction(tool).toLowerCase();
  const short = shortToolName(tool.name).toLowerCase();
  const lowerName = tool.name.toLowerCase();

  if (
    short === "agent" ||
    short === "agentbatch" ||
    short === "agentbatchstatus" ||
    lowerName.endsWith("__agent") ||
    lowerName.endsWith("__agentbatch")
  ) {
    return "agent";
  }
  if (short === "plan" || short.startsWith("flamel_plan_")) return "plan";
  if (short === "work") return "work";
  if (short === "job" || action.startsWith("job.")) return "job";
  if (short === "memory" || action.startsWith("brain.")) return "memory";
  if (
    short === "skill_catalog" ||
    short === "skill_search" ||
    action.startsWith("skill.") ||
    action.startsWith("pack.")
  ) {
    return "skill";
  }
  if (action.startsWith("connection.")) return "connection";
  if (action.startsWith("automation.")) return "automation";
  if (short === "assistant" || action.startsWith("assistant.")) {
    return "assistant";
  }
  return undefined;
}

export function isWorkflowToolCall(tool: ToolCall): boolean {
  return workflowWidgetKind(tool) !== undefined;
}

export function isResearchToolCall(tool: ToolCall): boolean {
  const lower = tool.name.toLowerCase();
  return (
    isToolRenderer(tool.name, "research") ||
    lower === "web__run" ||
    lower.includes("__web__run") ||
    "search_query" in tool.input ||
    "image_query" in tool.input
  );
}

export function isBrowserToolCall(tool: ToolCall): boolean {
  return isToolRenderer(tool.name, "browser");
}

export function isMediaToolCall(tool: ToolCall): boolean {
  return isToolRenderer(tool.name, "assets");
}

export function isVizTool(toolName: string): boolean {
  const shortName = shortToolName(toolName);
  return (
    toolName.startsWith("mcp__viz__") ||
    [
      "render_chart",
      "render_diagram",
      "render_metric",
      "render_progress",
      "render_svg",
    ].includes(shortName)
  );
}

export function isAskUserQuestionTool(name: string): boolean {
  return name === "AskUserQuestion" || name.endsWith("__AskUserQuestion");
}

export function isToolHidden(tool: ToolCall): boolean {
  const { name, input } = tool;
  if (INVISIBLE_TOOLS.has(name)) return true;
  if (name === "plan" || name.endsWith("__plan")) {
    const action = input?.action;
    if (typeof action === "string" && INVISIBLE_PLAN_ACTIONS.has(action)) {
      return true;
    }
  }

  const bare = name.includes("__") ? name.split("__").pop()! : name;
  return INVISIBLE_LEGACY_TOOLS.has(bare);
}

export function toolRenderKind(tool: ToolCall): ToolRenderKind {
  if (isToolHidden(tool)) return "hidden";
  if (tool.name === "mcp__flamel__thinking") return "thinking";
  if (isAskUserQuestionTool(tool.name)) return "question";
  if (isVizTool(tool.name)) return "viz";
  if (isWorkflowToolCall(tool)) return "workflow";
  if (isResearchToolCall(tool)) return "research";
  if (isBrowserToolCall(tool)) return "browser";
  if (isMediaToolCall(tool)) return "media";
  if (isEngineeringToolCall(tool)) return "engineering";
  if (isAdaptiveDataToolCall(tool)) return "adaptive";
  return "legacy";
}
