import {
  appendThinkingSegment,
  stripHiddenControlMarkers,
} from "./messageProcessor";
import { compactBlockFromPayload } from "./compactionEvents";
import { toolFromAssistantSnapshot } from "./chatToolEvents";
import { applyToolResult, type ToolResultEventLike } from "./toolResultReducer";
import type {
  DisplayBlock,
  ToolCall,
  ContentSegment,
} from "./messageProcessor";

export function processHistoryMessages(
  messages: Array<{
    type: string;
    message: any;
    uuid?: string;
    timestamp?: string;
  }>,
  fallbackModel?: string,
): DisplayBlock[] {
  const blocks: DisplayBlock[] = [];
  // Map from tool_use_id to the DisplayBlock that owns it (for attaching tool results)
  const toolBlockMap = new Map<string, DisplayBlock>();
  const pendingResults = new Map<string, ToolResultEventLike[]>();

  function applyOrBufferResult(result: ToolResultEventLike) {
    if (typeof result.tool_use_id !== "string" || !result.tool_use_id) return;
    const parentBlock = toolBlockMap.get(result.tool_use_id);
    if (parentBlock?.tools) {
      const application = applyToolResult(parentBlock.tools, result);
      if (application) parentBlock.tools = application.tools;
    } else {
      const pending = pendingResults.get(result.tool_use_id) || [];
      pending.push(result);
      pendingResults.set(result.tool_use_id, pending);
    }
  }

  for (const entry of messages) {
    const { type, message } = entry;

    if (type === "compaction") {
      const block = compactBlockFromPayload(message);
      blocks.push({
        ...block,
        id: block.compaction?.runId
          ? `compaction:${block.compaction.runId}`
          : entry.uuid || block.id,
        isStreaming: false,
      });
      continue;
    }

    const content = Array.isArray(message?.content) ? message.content : [];
    // Persisted visible turns can embed receipts in assistant content, while
    // older histories use separate user/tool_result entries. Either may arrive
    // before its tool envelope; use the same terminal-state reducer as live chat.
    for (const item of content) {
      if (item?.type === "tool_result") applyOrBufferResult(item);
    }

    if (type === "user") {
      // Check if this is purely tool_results (injected, not a real user message)
      const isOnlyToolResults =
        content.length > 0 &&
        content.every((c: any) => c.type === "tool_result");

      if (isOnlyToolResults) {
        continue;
      }

      // Real user message
      const textContent = stripHiddenControlMarkers(
        content
          .filter((c: any) => c.type === "text")
          .map((c: any) => c.text)
          .join("\n"),
      );

      if (textContent.trim()) {
        blocks.push({
          id: entry.uuid || crypto.randomUUID(),
          role: "user",
          type: "user",
          content: textContent,
          ...(typeof message?.flamel?.requestId === "string"
            ? { requestId: message.flamel.requestId }
            : {}),
        });
      }
    }

    if (type === "assistant") {
      let thinking = "";
      let text = "";
      const tools: ToolCall[] = [];
      const segments: ContentSegment[] = [];

      for (const item of content) {
        if (item.type === "thinking") {
          const tText = item.thinking || "";
          const redacted = !tText && !!item.signature;
          if (tText) {
            thinking += thinking ? `\n\n${tText}` : tText;
          }
          if (tText || redacted) {
            appendThinkingSegment(segments, tText, redacted);
          }
        } else if (item.type === "text") {
          const cleanText = stripHiddenControlMarkers(item.text || "");
          text += cleanText;
          const lastSeg = segments[segments.length - 1];
          if (lastSeg?.type === "text") {
            lastSeg.text += cleanText;
          } else {
            segments.push({ type: "text", text: cleanText });
          }
        } else if (item.type === "tool_use") {
          const existingIndex = tools.findIndex((tool) => tool.id === item.id);
          const previous = existingIndex >= 0 ? tools[existingIndex]
            : toolBlockMap.get(item.id)?.tools?.find((tool) => tool.id === item.id);
          const tool = toolFromAssistantSnapshot({
            id: item.id,
            name: item.name,
            input: item.input || {},
          }, previous);
          if (existingIndex >= 0) {
            tools[existingIndex] = tool;
          } else {
            tools.push(tool);
            segments.push({ type: "tool_ref", toolId: item.id });
          }
        }
      }

      const block: DisplayBlock = {
        id: entry.uuid || crypto.randomUUID(),
        role: "assistant",
        type: "assistant",
        content: text,
        thinking: thinking || undefined,
        tools,
        segments,
        model: message?.model || fallbackModel,
        usage: message?.usage
          ? {
              inputTokens: message.usage.input_tokens || 0,
              outputTokens: message.usage.output_tokens || 0,
            }
          : undefined,
        isStreaming: false,
      };

      // Register tools in map so tool_results can be attached
      for (const tool of tools) {
        toolBlockMap.set(tool.id, block);
        const pending = pendingResults.get(tool.id);
        if (pending) {
          pendingResults.delete(tool.id);
          for (const result of pending) applyOrBufferResult(result);
        }
      }

      blocks.push(block);
    }
  }

  for (const block of blocks) {
    for (const tool of block.tools || []) {
      if (tool.status === "running") tool.status = "unknown";
    }
  }
  return blocks;
}
