export interface NestedAgentRunNode {
  runId: string;
  parentRunId?: string;
  events: Array<Record<string, unknown>>;
  children: NestedAgentRunNode[];
}

export function buildNestedAgentRunTree(
  events: Array<Record<string, unknown>>,
): NestedAgentRunNode[] {
  const nodes = new Map<string, NestedAgentRunNode>();
  for (const event of events) {
    const runId = typeof event.run_id === "string" ? event.run_id : "";
    if (!runId) continue;
    const node = nodes.get(runId) || { runId, events: [], children: [] };
    node.events.push(event);
    if (typeof event.parent_run_id === "string" && event.parent_run_id) {
      node.parentRunId = event.parent_run_id;
    }
    nodes.set(runId, node);
  }

  const roots: NestedAgentRunNode[] = [];
  for (const node of nodes.values()) {
    const parent = node.parentRunId ? nodes.get(node.parentRunId) : undefined;
    if (parent && parent !== node) parent.children.push(node);
    else roots.push(node);
  }
  return roots;
}
