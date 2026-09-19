import React from "react";
import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { routineActivityGroup } from "../tool-widgets/toolWidgetRouting";
import type { ContentSegment, ToolCall } from "../../lib/messageProcessor";
import { InputDeliveryList } from "./InputDeliveryList";

describe("compact chat activity", () => {
  it("groups only consecutive completed reads, never commands or failures", () => {
    const tools = new Map<string, ToolCall>(["a", "b", "c", "d"].map(id => [id, {
      id, name: "file_read", input: { action: "read", file: `${id}.ts` }, status: "done",
    }]));
    const segments: ContentSegment[] = [...tools.keys()].map(toolId => ({ type: "tool_ref", toolId }));
    expect(routineActivityGroup(segments, 0, tools).length).toBe(4);
    tools.get("c")!.status = "unknown";
    expect(routineActivityGroup(segments, 0, tools)).toEqual([]);
    tools.get("c")!.status = "error";
    expect(routineActivityGroup(segments, 0, tools)).toEqual([]);
    tools.get("c")!.status = "done";
    tools.get("c")!.name = "shell";
    expect(routineActivityGroup(segments, 0, tools)).toEqual([]);
    segments[1] = { type: "text", text: "Keep this explanation in place" };
    expect(routineActivityGroup(segments, 0, tools)).toEqual([]);
  });

  it("keeps uncertain guidance text and the delivery explanation accessible", () => {
    const html = renderToStaticMarkup(<InputDeliveryList receipts={[{
      requestId: "input-1", sessionId: "session-1", message: "Use the existing reducer",
      delivery: "uncertain", note: "Check delivery before sending again.",
    }]} />);
    expect(html).toContain("Delivery uncertain");
    expect(html).toContain("Use the existing reducer");
    expect(html).toContain("Check delivery before sending again.");
    expect(html).not.toContain("Sent to current run");
  });
});
