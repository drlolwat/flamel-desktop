import React from "react";
import { shortToolName } from "../../lib/toolCapabilities";
import type { ToolCall } from "../../lib/messageProcessor";
export { isVizTool } from "../tool-widgets/toolWidgetRouting";
import { Icon } from "../ui/Icons";
import {
  ToolWidgetInspector,
  ToolWidgetState,
} from "../tool-widgets/ToolWidgetPrimitives";
import { ToolWidgetShell } from "../tool-widgets/ToolWidgetShell";

// Lazy-load viz components to avoid pulling recharts into the main bundle
const ChartViz = React.lazy(() => import("./ChartViz"));
const DiagramViz = React.lazy(() => import("./DiagramViz"));
const MetricViz = React.lazy(() => import("./MetricViz"));
const ProgressViz = React.lazy(() => import("./ProgressViz"));
const SvgViz = React.lazy(() => import("./SvgViz"));

class VizErrorBoundary extends React.Component<
  { children: React.ReactNode },
  { error: string | null }
> {
  constructor(props: { children: React.ReactNode }) {
    super(props);
    this.state = { error: null };
  }

  static getDerivedStateFromError(err: unknown) {
    return { error: err instanceof Error ? err.message : String(err) };
  }

  render() {
    if (this.state.error) {
      return (
        <ToolWidgetState
          status="error"
          title="Visualization error"
          detail={this.state.error}
        />
      );
    }
    return this.props.children;
  }
}

type VisualizationStatus = "pending" | "running" | "success" | "error";

export interface VisualizationDetails {
  kind: "chart" | "diagram" | "metric" | "progress" | "svg" | "unknown";
  title: string;
  summary: string;
  detail?: string;
  status: VisualizationStatus;
  validationError?: string;
}

export function visualizationDetails(tool: ToolCall): VisualizationDetails {
  const shortName = shortToolName(tool.name);
  const input = tool.input || {};
  const kind =
    shortName === "render_chart"
      ? "chart"
      : shortName === "render_diagram"
        ? "diagram"
        : shortName === "render_metric"
          ? "metric"
          : shortName === "render_progress"
            ? "progress"
            : shortName === "render_svg"
              ? "svg"
              : "unknown";
  const title =
    typeof input.title === "string" && input.title.trim()
      ? input.title.trim()
      : kind === "unknown"
        ? "Visualization"
        : kind.charAt(0).toUpperCase() + kind.slice(1) + " visualization";
  const running = tool.status === "running" || tool.status === "pending";
  let validationError: string | undefined;
  if (!running) {
    if (Object.keys(input).length === 0) {
      validationError = "No visualization input was received.";
    } else if (
      kind === "chart" &&
      (!Array.isArray(input.data) || !input.xKey || !input.yKeys)
    ) {
      validationError = "Chart data, xKey, and yKeys are required.";
    } else if (kind === "diagram" && typeof input.diagram !== "string") {
      validationError = "Diagram source is required.";
    } else if (kind === "metric" && !Array.isArray(input.metrics)) {
      validationError = "Metric values are required.";
    } else if (kind === "progress" && !Array.isArray(input.bars)) {
      validationError = "Progress bars are required.";
    } else if (kind === "svg" && typeof input.svg !== "string") {
      validationError = "SVG markup is required.";
    } else if (kind === "unknown") {
      validationError = "Unsupported visualization tool: " + shortName;
    }
  }

  const count =
    kind === "chart" && Array.isArray(input.data)
      ? input.data.length
      : kind === "metric" && Array.isArray(input.metrics)
        ? input.metrics.length
        : kind === "progress" && Array.isArray(input.bars)
          ? input.bars.length
          : undefined;
  const detail =
    kind === "chart" && typeof input.type === "string"
      ? input.type
      : kind === "diagram" && typeof input.diagram === "string"
        ? input.diagram.split(/\r?\n/)[0]?.trim()
        : kind;
  const status: VisualizationStatus =
    tool.status === "error" || validationError
      ? "error"
      : tool.status === "running"
        ? "running"
        : tool.status === "pending"
          ? "pending"
          : "success";
  const summary =
    status === "running"
      ? "Rendering visualization"
      : status === "pending"
        ? "Waiting for visualization data"
        : status === "error"
          ? "Visualization could not render"
          : count !== undefined
            ? count + " " + (count === 1 ? "item" : "items") + " rendered"
            : "Visualization ready";

  return { kind, title, summary, detail, status, validationError };
}

function VisualizationCanvas({
  tool,
  details,
}: {
  tool: ToolCall;
  details: VisualizationDetails;
}) {
  const shortName = shortToolName(tool.name);
  const input = tool.input || {};

  if (
    (tool.status === "running" || tool.status === "pending") &&
    Object.keys(input).length === 0
  ) {
    return (
      <ToolWidgetState
        status={tool.status}
        title="Preparing visualization"
        detail="The renderer will mount as structured input arrives."
      />
    );
  }

  if (details.validationError) {
    return (
      <ToolWidgetState
        status="error"
        title="Invalid visualization input"
        detail={details.validationError}
      />
    );
  }

  const fallback = (
    <ToolWidgetState
      status="running"
      title="Loading renderer"
      detail="The visualization engine is loaded only while this card is open."
    />
  );

  return (
    <VizErrorBoundary>
      <React.Suspense fallback={fallback}>
        <div className="visualization-widget__canvas" data-kind={details.kind}>
          {shortName === "render_chart" ? (
            <ChartViz
              type={input.type as string}
              data={input.data as Record<string, unknown>[]}
              xKey={input.xKey as string}
              yKeys={input.yKeys as string[]}
              colors={input.colors as string[] | undefined}
              stacked={input.stacked as boolean | undefined}
            />
          ) : null}
          {shortName === "render_diagram" ? (
            <DiagramViz diagram={input.diagram as string} />
          ) : null}
          {shortName === "render_metric" ? (
            <MetricViz metrics={input.metrics as never[]} />
          ) : null}
          {shortName === "render_progress" ? (
            <ProgressViz bars={input.bars as never[]} />
          ) : null}
          {shortName === "render_svg" ? (
            <SvgViz
              svg={input.svg as string}
              width={input.width as number | undefined}
              height={input.height as number | undefined}
            />
          ) : null}
        </div>
      </React.Suspense>
    </VizErrorBoundary>
  );
}

/** Render a viz tool call through the authored tool-widget shell. */
export default React.memo(function VizRenderer({
  tool,
  defaultExpanded,
}: {
  tool: ToolCall;
  defaultExpanded?: boolean;
}) {
  const details = visualizationDetails(tool);

  return (
    <ToolWidgetShell
      title={details.title}
      eyebrow="Visualization"
      summary={details.summary}
      detail={details.detail}
      status={details.status}
      statusLabel={details.status === "running" ? "Rendering" : undefined}
      icon={<Icon.Image size={14} />}
      sprite="data"
      defaultExpanded={
        defaultExpanded ??
        (details.status === "success" ||
          details.status === "running" ||
          details.status === "error")
      }
      inspector={
        <ToolWidgetInspector
          input={tool.input}
          output={tool.output}
          raw={{ name: tool.name, partialInput: tool.partialInput }}
        />
      }
    >
      <VisualizationCanvas tool={tool} details={details} />
    </ToolWidgetShell>
  );
});
