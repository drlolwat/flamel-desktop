import React from "react";
import type { ToolCall } from "../../lib/messageProcessor";
import VizRenderer from "../viz/VizRenderer";
import { AdaptiveDataToolWidget } from "./AdaptiveDataToolWidget";
import { EngineeringToolWidget } from "./EngineeringToolWidget";
import { MediaToolWidget } from "./MediaToolWidget";
import { ResearchBrowserToolWidget } from "./ResearchBrowserToolWidget";
import { WorkflowToolWidget } from "./WorkflowToolWidget";
import { toolRenderKind } from "./toolWidgetRouting";
import { shortToolName } from "../../lib/toolCapabilities";
import { ToolWidgetShell } from "./ToolWidgetShell";
import { ToolWidgetInspector } from "./ToolWidgetPrimitives";

export {
  isAskUserQuestionTool,
  isToolHidden,
  toolRenderKind,
  type ToolRenderKind,
} from "./toolWidgetRouting";

type ToolRendererComponent = React.ComponentType<{ tool: ToolCall }>;
type ThinkingRendererComponent = React.ComponentType<{
  tool: ToolCall;
  blockStreaming?: boolean;
}>;

interface ToolWidgetRendererProps {
  tool: ToolCall;
  blockStreaming?: boolean;
  LegacyRenderer: ToolRendererComponent;
  ThinkingRenderer: ThinkingRendererComponent;
}

export const ToolWidgetRenderer = React.memo(function ToolWidgetRenderer({
  tool,
  blockStreaming,
  LegacyRenderer,
  ThinkingRenderer,
}: ToolWidgetRendererProps) {
  const kind = toolRenderKind(tool);
  if (tool.status === "unknown" && kind !== "hidden" && kind !== "question" && kind !== "engineering") {
    return (
      <ToolWidgetShell
        title={shortToolName(tool.name)}
        summary="Completion not confirmed"
        status="unknown"
        inspector={<ToolWidgetInspector input={tool.input} output={tool.output} raw={tool.presentation} />}
      />
    );
  }
  switch (kind) {
    case "hidden":
    case "question":
      return null;
    case "thinking":
      return <ThinkingRenderer tool={tool} blockStreaming={blockStreaming} />;
    case "viz":
      return <VizRenderer tool={tool} />;
    case "workflow":
      return <WorkflowToolWidget tool={tool} />;
    case "research":
      return <ResearchBrowserToolWidget kind="research" tool={tool} />;
    case "browser":
      return <ResearchBrowserToolWidget kind="browser" tool={tool} />;
    case "media":
      return <MediaToolWidget tool={tool} />;
    case "engineering":
      return <EngineeringToolWidget tool={tool} />;
    case "adaptive":
      return <AdaptiveDataToolWidget tool={tool} />;
    case "legacy":
      return <LegacyRenderer tool={tool} />;
  }
});
