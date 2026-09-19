import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { ToolCall } from "../../lib/messageProcessor";
import {
  adaptWorkflowTool,
  buildAgentRunTree,
  isWorkflowToolCall,
  WorkflowToolWidget,
  workflowWidgetKind,
} from "./WorkflowToolWidget";

function tool(
  name: string,
  input: Record<string, unknown> = {},
  output: unknown = {},
  status: ToolCall["status"] = "done",
): ToolCall {
  return {
    id: `fixture-${name}-${status}`,
    name,
    input,
    output: typeof output === "string" ? output : JSON.stringify(output),
    status,
  };
}

function assistantEnvelope(
  action: string,
  result: Record<string, unknown>,
  extra: Record<string, unknown> = {},
) {
  return {
    contractVersion: 1,
    surfaceContract: "flamel-assistant.v1",
    action,
    ok: true,
    state: "succeeded",
    approvalRequired: false,
    mutated: false,
    replayed: false,
    summary: {
      text: `${action} completed`,
      card: {
        title: "Authoritative receipt",
        action,
        status: "succeeded",
        approvalRequired: false,
      },
    },
    result,
    ...extra,
  };
}

describe("workflow tool routing", () => {
  it("routes Assistant families and direct workflow tools to authored widgets", () => {
    const cases: Array<[string, Record<string, unknown>, string]> = [
      ["mcp__flamel__assistant", { action: "assistant.status" }, "assistant"],
      [
        "mcp__flamel__assistant",
        { action: "automation.history" },
        "automation",
      ],
      ["mcp__flamel__assistant", { action: "job.events" }, "job"],
      ["mcp__flamel__assistant", { action: "brain.inspect" }, "memory"],
      ["mcp__flamel__assistant", { action: "skill.inspect" }, "skill"],
      ["mcp__flamel__assistant", { action: "pack.list" }, "skill"],
      ["mcp__flamel__assistant", { action: "connection.status" }, "connection"],
      ["mcp__flamel__plan", { action: "create" }, "plan"],
      ["mcp__flamel__work", { action: "get" }, "work"],
      ["mcp__flamel__Agent", {}, "agent"],
      ["mcp__flamel__AgentBatch", {}, "agent"],
      ["mcp__flamel__memory", { action: "search" }, "memory"],
      ["mcp__flamel__skill_catalog", { action: "inspect" }, "skill"],
    ];

    for (const [name, input, kind] of cases) {
      const fixture = tool(name, input);
      expect(isWorkflowToolCall(fixture), `${name} ${input.action ?? ""}`).toBe(
        true,
      );
      expect(workflowWidgetKind(fixture)).toBe(kind);
    }
    expect(isWorkflowToolCall(tool("mcp__flamel__file_read"))).toBe(false);
  });

  it("recovers action and identity from partial streaming input", () => {
    const fixture: ToolCall = {
      id: "partial",
      name: "mcp__flamel__assistant",
      input: {},
      partialInput: '{"action":"automation.pause","name":"Nightly index',
      status: "running",
    };
    const view = adaptWorkflowTool(fixture);
    expect(view).toMatchObject({
      kind: "automation",
      action: "automation.pause",
      title: "Nightly index",
      status: "running",
    });
  });
});

describe("Assistant workflow receipts", () => {
  it("uses the v1 summary card without exposing the contract as primary content", () => {
    const view = adaptWorkflowTool(
      tool(
        "mcp__flamel__assistant",
        { action: "assistant.status" },
        assistantEnvelope("assistant.status", {
          assistant: { id: "ember", model: "codex", status: "ready" },
        }),
      ),
    );
    expect(view).toMatchObject({
      title: "Authoritative receipt",
      summary: "assistant.status completed",
      status: "success",
      contractKnown: true,
      contractVersion: 1,
    });
    expect(view?.fields.map((entry) => entry.label)).toContain("Contract");
  });

  it("keeps approval-required operations as explicit read-only previews", () => {
    const fixture = tool(
      "mcp__flamel__assistant",
      { action: "automation.delete", profile_id: "nightly" },
      assistantEnvelope(
        "automation.delete",
        { automation: { id: "nightly", schedule: "0 2 * * *" } },
        {
          approvalRequired: true,
          state: "awaiting_user",
          summary: {
            text: "Delete Nightly index",
            card: {
              title: "Delete Nightly index",
              action: "automation.delete",
              status: "awaiting_user",
              approvalRequired: true,
            },
          },
        },
      ),
    );
    const view = adaptWorkflowTool(fixture);
    expect(view).toMatchObject({
      kind: "automation",
      approvalRequired: true,
      mutated: false,
      status: "pending",
      statusLabel: "Preview",
    });

    const html = renderToStaticMarkup(
      <WorkflowToolWidget tool={fixture} defaultExpanded />,
    );
    expect(html).toContain("Read-only preview");
    expect(html).toContain("No mutation has run");
    expect(html).not.toContain(">Approve<");
    expect(html).not.toContain(">Delete<");
  });

  it("distinguishes mutation, idempotent replay, authorization failure, and unknown contracts", () => {
    const mutated = adaptWorkflowTool(
      tool(
        "mcp__flamel__assistant",
        { action: "automation.pause" },
        assistantEnvelope(
          "automation.pause",
          { automation: { id: "nightly", status: "paused" } },
          { mutated: true },
        ),
      ),
    );
    const replayed = adaptWorkflowTool(
      tool(
        "mcp__flamel__assistant",
        { action: "job.rerun" },
        assistantEnvelope(
          "job.rerun",
          { job: { id: "job-12", status: "queued" } },
          { replayed: true },
        ),
      ),
    );
    const denied = adaptWorkflowTool(
      tool(
        "mcp__flamel__assistant",
        { action: "brain.export" },
        {
          ...assistantEnvelope("brain.export", {}),
          ok: false,
          state: "forbidden",
          error: "Administrator permission required",
        },
      ),
    );
    const unknown = adaptWorkflowTool(
      tool(
        "mcp__flamel__assistant",
        { action: "assistant.status" },
        {
          ...assistantEnvelope("assistant.status", {}),
          contractVersion: 7,
          surfaceContract: "flamel-assistant.v7",
        },
      ),
    );

    expect(mutated?.mutated).toBe(true);
    expect(replayed).toMatchObject({ replayed: true, statusLabel: "Replayed" });
    expect(denied).toMatchObject({ status: "error" });
    expect(denied?.summary).toContain("Administrator permission");
    expect(unknown).toMatchObject({ contractKnown: false, contractVersion: 7 });
  });
});

describe("authored workflow evidence", () => {
  it("renders automation schedule and run history", () => {
    const view = adaptWorkflowTool(
      tool(
        "mcp__flamel__assistant",
        { action: "automation.history" },
        assistantEnvelope("automation.history", {
          automation: {
            id: "nightly-index",
            name: "Nightly index",
            cron: "0 2 * * *",
            timezone: "UTC",
            next_run_at: "2026-08-21T02:00:00Z",
          },
          history: [
            { id: "run-2", status: "succeeded", duration: "42s" },
            { id: "run-1", status: "failed", error: "Provider timeout" },
          ],
        }),
      ),
    );
    expect(view?.fields.map((entry) => entry.label)).toEqual(
      expect.arrayContaining(["Schedule", "Timezone", "Next run"]),
    );
    expect(view?.timeline).toHaveLength(2);
  });

  it("shows Job objective, progress, evidence, and failure state", () => {
    const view = adaptWorkflowTool(
      tool(
        "mcp__flamel__assistant",
        { action: "job.events" },
        assistantEnvelope("job.events", {
          job: {
            id: "job-42",
            objective: "Verify release",
            status: "running",
            completed: 3,
            total: 5,
          },
          evidence: [
            { id: "check-1", title: "Typecheck", status: "succeeded" },
            {
              id: "check-2",
              title: "Browser QA",
              status: "failed",
              error: "Overflow",
            },
          ],
        }),
      ),
    );
    expect(view).toMatchObject({
      kind: "job",
      title: "Verify release",
      progress: { done: 3, total: 5 },
    });
    expect(view?.timeline[1]).toMatchObject({
      title: "Browser QA",
      status: "error",
    });
  });

  it("shows plan progress from persisted marker payloads", () => {
    const view = adaptWorkflowTool(
      tool(
        "mcp__flamel__plan",
        { action: "get", plan_id: "plan-1" },
        'PLAN_GET:{"plan":{"id":"plan-1","title":"Ship widgets","items":[{"id":"a","text":"Adapter","status":"done"},{"id":"b","text":"Visual QA","status":"in_progress"}]}}',
      ),
    );
    expect(view).toMatchObject({
      kind: "plan",
      title: "Ship widgets",
      progress: { done: 1, total: 2 },
    });
    expect(view?.items.map((item) => item.primary)).toEqual([
      "Adapter",
      "Visual QA",
    ]);
  });

  it("shows work ownership, dependencies, and artifacts together", () => {
    const view = adaptWorkflowTool(
      tool(
        "mcp__flamel__work",
        { action: "get" },
        {
          work: {
            id: "work-7",
            title: "Workflow cards",
            owner: "frontend",
            dependencies: [
              { id: "dep-1", title: "Shell foundation", status: "done" },
            ],
            artifacts: [
              { id: "artifact-1", name: "visual-qa.png", status: "ready" },
            ],
          },
        },
      ),
    );
    expect(view?.fields).toEqual(
      expect.arrayContaining([expect.objectContaining({ label: "Owner" })]),
    );
    expect(view?.items.map((item) => item.primary)).toEqual([
      "Shell foundation",
      "visual-qa.png",
    ]);
  });

  it("builds a nested agent execution tree including batch-declared children", () => {
    const fixture: ToolCall = {
      ...tool("mcp__flamel__AgentBatch", {}, {}, "running"),
      runId: "batch-1",
      nestedEvents: [
        {
          type: "batch_start",
          run_id: "batch-1",
          title: "Renderer audit",
          agents: [
            {
              run_id: "agent-ui",
              task_name: "Inspect UI",
              subagent_type: "ui-designer",
              status: "running",
            },
            {
              run_id: "agent-tests",
              task_name: "Inspect tests",
              subagent_type: "test-writer-fixer",
              status: "done",
            },
          ],
        },
        {
          type: "nested_done",
          run_id: "agent-tests",
          parent_run_id: "batch-1",
          summary: "Coverage complete",
        },
      ],
    };
    const tree = buildAgentRunTree(fixture.nestedEvents ?? [], fixture);
    expect(tree).toHaveLength(1);
    expect(tree[0]).toMatchObject({ id: "batch-1", title: "Renderer audit" });
    expect(tree[0].children).toHaveLength(2);
    expect(tree[0].children[1]).toMatchObject({
      id: "agent-tests",
      status: "success",
      detail: "Coverage complete",
    });
    expect(adaptWorkflowTool(fixture)?.progress).toEqual({ done: 1, total: 2 });
  });

  it("never invents a successful child for an empty or missing batch status", () => {
    for (const output of [
      "No AgentBatch runs are tracked in this MCP process.",
      {status:"not_found",agents:[],total:0,completed:0},
      {status:"unknown",agents:[{run_id:"child",description:"Inspect logs",status:"unknown"}],total:1,completed:0},
    ]) {
      const fixture = tool("mcp__flamel__AgentBatchStatus",{},output);
      const view = adaptWorkflowTool(fixture);
      expect(view?.statusLabel).toBe("Unconfirmed");
      expect(view?.status).not.toBe("success");
      expect(view?.summary).not.toContain("1 of 1 complete");
      expect(view?.agentTree.every((node) => node.status !== "success")).toBe(true);
      const html = renderToStaticMarkup(<WorkflowToolWidget tool={fixture} defaultExpanded />);
      expect(html).not.toContain("Delegated task");
    }
  });

  it("restores actual child outcomes and output from a saved status receipt", () => {
    const fixture = tool("mcp__flamel__AgentBatchStatus",{}, {
      batch_id:"batch",status:"completed_with_errors",agents:[
        {run_id:"one",description:"Inspect UI",status:"success",output:"UI checked"},
        {run_id:"two",description:"Run tests",status:"failed",error:"Provider stopped"},
      ],total:2,completed:2,
    });
    const view = adaptWorkflowTool(fixture);
    expect(view?.status).toBe("error");
    expect(view?.progress).toEqual({done:1,total:2});
    expect(view?.agentTree.map((node) => node.status)).toEqual(["success","error"]);
    expect(view?.agentTree[1].detail).toBe("Provider stopped");
  });

  it("keeps the actual batch tree after reloading a saved tool result without live events", () => {
    const fixture = tool("mcp__flamel__AgentBatch",{}, 'AGENT_BATCH:batch\nAGENT_RUNS:one,two\n'+JSON.stringify({
      batch_id:"batch",status:"success",agents:[
        {run_id:"one",description:"Inspect UI",status:"success",output:"UI checked"},
        {run_id:"two",description:"Run tests",status:"success",output:"Tests passed"},
      ],total:2,completed:2,
    }));
    expect(adaptWorkflowTool(fixture)?.agentTree.map((node) => node.id)).toEqual(["one","two"]);
    expect(adaptWorkflowTool(fixture)?.progress).toEqual({done:2,total:2});
  });

  it("does not mark a batch successful when a child completion reports failure", () => {
    const fixture = tool("mcp__flamel__AgentBatch",{}, {}, "running");
    fixture.nestedEvents = [
      {type:"batch_start",run_id:"batch",agents:[{run_id:"child",index:0,status:"running"}]},
      {type:"batch_agent_done",run_id:"batch",child_run_id:"child",status:"failed",error:"Launch failed"},
      {type:"batch_done",run_id:"batch",failed:1},
    ];
    const tree = buildAgentRunTree(fixture.nestedEvents,fixture);
    expect(tree[0].status).toBe("error");
    expect(tree[0].children[0].status).toBe("error");
    expect(adaptWorkflowTool(fixture)?.progress).toEqual({done:0,total:1});
  });

  it("renders memory receipts without turning record payloads into raw JSON", () => {
    const fixture = tool(
      "mcp__flamel__assistant",
      { action: "brain.inspect" },
      assistantEnvelope("brain.inspect", {
        records: [
          {
            id: "memory-1",
            content: "Prefer authored compact interfaces",
            scope: "project",
            status: "active",
          },
        ],
      }),
    );
    const view = adaptWorkflowTool(fixture);
    expect(view?.items[0]).toMatchObject({
      primary: "Prefer authored compact interfaces",
      secondary: "project",
    });
    const html = renderToStaticMarkup(
      <WorkflowToolWidget tool={fixture} defaultExpanded />,
    );
    expect(html).toContain("Prefer authored compact interfaces");
    expect(html).toContain("Inspect raw data");
  });

  it("surfaces governed skill identity, review evidence, triggers, projection, and provenance", () => {
    const fixture = tool(
      "mcp__flamel__skill_catalog",
      { action: "inspect", skill_id: "ui-craft" },
      {
        skill: {
          id: "ui-craft",
          title: "UI craft",
          version: "1.4.0",
          sha256: "34e7d3",
          signature_status: "verified",
          scope: "project",
          active: true,
          persona: "ui-designer",
          triggers: [
            { name: "interface implementation" },
            { pattern: "visual quality review" },
          ],
        },
      },
    );
    const view = adaptWorkflowTool(fixture);
    expect(view).toMatchObject({ kind: "skill", title: "UI craft" });
    expect(view?.fields.map((entry) => entry.label)).toEqual(
      expect.arrayContaining([
        "Version",
        "Signature",
        "Hash",
        "Scope",
        "Triggers",
        "Provenance",
        "Projection",
      ]),
    );
    const html = renderToStaticMarkup(
      <WorkflowToolWidget tool={fixture} defaultExpanded />,
    );
    expect(html).toContain("ui-designer");
    expect(html).toContain("interface implementation");
    expect(html).not.toContain('{"skill"');
  });

  it("shows connection permission scope and canonical navigation only", () => {
    const fixture = tool(
      "mcp__flamel__assistant",
      { action: "connection.status" },
      assistantEnvelope("connection.status", {
        connection: {
          id: "github",
          name: "GitHub",
          status: "connected",
          permissions: [
            { scope: "repository:read" },
            { permission: "pull_request:write" },
          ],
        },
      }),
    );
    const view = adaptWorkflowTool(fixture);
    expect(view?.fields).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          label: "Permissions",
          value: "repository:read, pull_request:write",
        }),
      ]),
    );
    const html = renderToStaticMarkup(
      <WorkflowToolWidget tool={fixture} defaultExpanded />,
    );
    expect(html).toContain("Open canonical surface");
    expect(html).not.toContain(">Disconnect<");
  });
});
