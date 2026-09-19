import React from "react";
import "./toolWidgetGallery.css";
import type { DisplayBlock, ToolCall } from "../../lib/messageProcessor";
import ClaudeBlock from "../chat/ClaudeBlock";
import { Icon } from "../ui/Icons";
import VizRenderer from "../viz/VizRenderer";
import {
  ToolArtifactActions,
  ToolWidgetFields,
  ToolWidgetInspector,
  ToolWidgetList,
  ToolWidgetSection,
  ToolWidgetState,
  ToolWidgetTable,
  ToolWidgetTabs,
  ToolWidgetTimeline,
} from "./ToolWidgetPrimitives";
import { EmbeddedDiffView } from "./EmbeddedDiffView";
import { EngineeringToolWidget } from "./EngineeringToolWidget";
import { MediaToolWidget } from "./MediaToolWidget";
import { ResearchBrowserToolWidget } from "./ResearchBrowserToolWidget";
import { ToolWidgetShell } from "./ToolWidgetShell";
import { WorkflowToolWidget } from "./WorkflowToolWidget";
import { AdaptiveDataToolWidget } from "./AdaptiveDataToolWidget";
import {
  legacySkillBatchText,
  skillBatchFiles,
} from "./skillBatch.fixture";

const fixtureArtifacts = [
  {
    id: "fixture-preview",
    kind: "image" as const,
    name: "workspace-map.webp",
    mediaType: "image/webp",
    byteSize: 18_432,
    sha256: "fixture",
    contentUrl: "/tool-sprites/workspace.webp",
    downloadUrl: "/tool-sprites/workspace.webp",
  },
];

const fixtureDiffFiles = [
  {
    path: "frontend/src/lib/toolCapabilities.ts",
    original: `export type ToolRendererKind = "generic" | "shell";\n\nexport function rendererFor(name: string) {\n  return name.includes("shell") ? "shell" : "generic";\n}\n`,
    modified: `export type ToolRendererKind = "adaptive" | "shell" | "workspace";\n\nexport function rendererFor(name: string) {\n  if (name.includes("shell")) return "shell";\n  if (name.includes("file")) return "workspace";\n  return "adaptive";\n}\n`,
    additions: 4,
    deletions: 2,
    status: "modified" as const,
  },
  {
    path: "frontend/src/components/tool-widgets/WidgetRegistry.ts",
    original: "",
    modified: `export const widgetFamilies = [\n  "workspace",\n  "source-control",\n  "terminal",\n] as const;\n`,
    additions: 5,
    deletions: 0,
    status: "added" as const,
  },
  {
    path: "frontend/src/components/chat/LegacyDiffTable.tsx",
    original: `export function LegacyDiffTable() {\n  return null;\n}\n`,
    modified: "",
    additions: 0,
    deletions: 3,
    status: "deleted" as const,
  },
  {
    path: "README.md",
    patch: `--- a/README.md\n+++ b/README.md\n@@ -4 +4 @@\n-Generic tool output\n+Purpose-built tool widgets\n`,
    additions: 1,
    deletions: 1,
    status: "modified" as const,
  },
];

const engineeringFixtures: ToolCall[] = [
  {
    id: "fixture-engineering-read",
    name: "mcp__flamel__file_read",
    status: "done",
    input: {
      action: "read",
      file: "frontend/src/lib/toolCapabilities.ts",
    },
    output: JSON.stringify({
      content: [
        {
          type: "text",
          text: `     1\texport type ToolRendererKind =
     2\t  | "adaptive"
     3\t  | "workspace"
     4\t  | "source-control";
     5\t
     6\texport const authoredWidgets = true;
[frontend/src/lib/toolCapabilities.ts: lines 1-6 of 6]`,
        },
      ],
    }),
  },
  {
    id: "fixture-engineering-skill-batch",
    name: "mcp__flamel__file_read",
    status: "done",
    input: {
      action: "batch",
      files: skillBatchFiles.map((file) => file.path),
    },
    output: legacySkillBatchText,
  },
  {
    id: "fixture-engineering-bounded-read",
    name: "mcp__flamel__file_read",
    status: "done",
    input: {
      action: "read",
      file: "frontend/src/lib/toolPresentation.ts",
    },
    output:
      'export interface ToolPresentationData {\n  text: string;\n  truncated: boolean;\n}',
    presentation: {
      version: 1,
      text:
        'export interface ToolPresentationData {\n  text: string;\n  truncated: boolean;\n}',
      summary: "The tool completed; the full output is available as an artifact.",
      resources: [],
      media: [],
      artifacts: [
        {
          id: "mcp-file-read-gallery",
          kind: "file",
          name: "file_read-gallery.txt",
          mediaType: "text/plain",
          byteSize: 18_432,
          sha256: "",
          contentUrl: "/api/mcp-artifacts/file_read-gallery.txt",
          downloadUrl: "/api/mcp-artifacts/file_read-gallery.txt?download=1",
        },
      ],
      transport: {
        reason: "inline_budget_exceeded",
        inlineBudgetBytes: 12_288,
      },
      isError: false,
      truncated: true,
      legacy: false,
    },
  },
  {
    id: "fixture-engineering-edit",
    name: "mcp__flamel__file_edit",
    status: "done",
    input: {
      action: "replace",
      file: "frontend/src/components/chat/ClaudeBlock.tsx",
      old_string: 'const renderer = "generic";',
      new_string: 'const renderer = "workspace";',
    },
    output: "Replaced one occurrence in ClaudeBlock.tsx",
  },
  {
    id: "fixture-engineering-code",
    name: "mcp__flamel__code_nav",
    status: "done",
    input: {
      action: "search",
      path: "frontend/src",
      pattern: "EngineeringToolWidget",
    },
    output: JSON.stringify({
      structuredContent: {
        matches: [
          {
            path: "frontend/src/components/chat/ClaudeBlock.tsx",
            line: 58,
            message: "import { EngineeringToolWidget }",
          },
          {
            path: "frontend/src/components/chat/ClaudeBlock.tsx",
            line: 3110,
            message: "return <EngineeringToolWidget tool={tool} />",
          },
          {
            path: "frontend/src/components/tool-widgets/EngineeringToolWidget.tsx",
            line: 328,
            message: "export function EngineeringToolWidget",
          },
        ],
      },
    }),
  },
  {
    id: "fixture-engineering-git",
    name: "mcp__flamel__git",
    status: "done",
    input: {
      action: "repo.status",
      path: "/workspace/flamel",
    },
    output:
      " M frontend/src/components/chat/ClaudeBlock.tsx\n?? frontend/src/components/tool-widgets/EngineeringToolWidget.tsx",
  },
  {
    id: "fixture-engineering-shell",
    name: "mcp__flamel__shell",
    status: "running",
    input: {
      action: "run",
      command: "npm test -- EngineeringToolWidget",
      cwd: "frontend",
    },
    output:
      "RUN v3.2.4\n✓ engineering tool capability routing\n⠹ widget rendering",
  },
  {
    id: "fixture-engineering-diagnostics",
    name: "mcp__flamel__project_intelligence",
    status: "done",
    input: {
      action: "verification",
      project_path: "/workspace/flamel",
    },
    output: JSON.stringify({
      content: [{ type: "text", text: "Verification graph is ready" }],
      structuredContent: {
        checks: [
          {
            id: "types",
            name: "TypeScript",
            status: "passed",
            duration: 420,
          },
          {
            id: "tests",
            name: "Focused widgets",
            status: "passed",
            duration: 1210,
          },
          {
            id: "build",
            name: "Production build",
            status: "pending",
          },
        ],
      },
    }),
  },
];

const mediaFixture: ToolCall = {
  id: "fixture-rich-media",
  name: "image_gen",
  status: "done",
  input: {
    prompt: "A restrained pixel-art sprite sheet for Flamel tool families",
  },
  output: JSON.stringify({
    output_hint: "Two managed WebP artifacts are ready.",
  }),
  artifacts: [
    {
      id: "media-sheet",
      kind: "image",
      name: "tool-sprite-sheet.webp",
      mediaType: "image/webp",
      byteSize: 24_960,
      sha256: "fixture-media",
      contentUrl: "/tool-sprites/media.webp",
      downloadUrl: "/tool-sprites/media.webp",
    },
    {
      id: "workspace-sheet",
      kind: "image",
      name: "workspace-variant.webp",
      mediaType: "image/webp",
      byteSize: 18_432,
      sha256: "fixture-workspace",
      contentUrl: "/tool-sprites/workspace.webp",
      downloadUrl: "/tool-sprites/workspace.webp",
    },
  ],
};

const researchFixture: ToolCall = {
  id: "fixture-rich-research",
  name: "web__run",
  status: "done",
  input: {
    search_query: [{ q: "purpose-built developer tool interfaces" }],
  },
  output: JSON.stringify({
    structuredContent: {
      results: [
        {
          title: "Designing dense developer tools",
          url: "https://example.com/developer-tools",
          snippet:
            "Hierarchy and interaction patterns for evidence-heavy interfaces.",
        },
        {
          title: "Accessible disclosure widgets",
          url: "https://a11y.example/disclosure",
          snippet:
            "Keyboard and screen-reader contracts for expandable content.",
        },
        {
          title: "Responsive data visualization",
          url: "https://charts.example/responsive",
          snippet: "Keeping complex data legible on narrow screens.",
        },
      ],
    },
  }),
};

const browserFixture: ToolCall = {
  id: "fixture-rich-browser",
  name: "mcp__flamel__browser_inspect",
  status: "done",
  input: {
    action: "inspect",
    url: "https://flamel.local/tool-widgets",
  },
  output: JSON.stringify({
    title: "Tool widget gallery",
    url: "https://flamel.local/tool-widgets",
    timeline: [
      { action: "navigate", status: "done", duration: "312 ms" },
      { action: "accessibility snapshot", status: "done", duration: "44 ms" },
      { action: "responsive capture", status: "done", duration: "180 ms" },
    ],
    console: [],
    network: [{ status: 200 }, { status: 200 }, { status: 304 }],
    audits: [{ name: "accessibility", score: 100 }],
  }),
  artifacts: [
    {
      id: "browser-capture",
      kind: "image",
      name: "browser-capture.webp",
      mediaType: "image/webp",
      byteSize: 21_504,
      sha256: "fixture-browser",
      contentUrl: "/tool-sprites/browser-research.webp",
      downloadUrl: "/tool-sprites/browser-research.webp",
    },
  ],
};

const workflowFixtures: ToolCall[] = [
  {
    id: "fixture-agent-status-missing",
    name: "mcp__flamel__AgentBatchStatus",
    status: "done",
    input: { batch_id: "missing" },
    output: JSON.stringify({status:"not_found",agents:[],total:0,completed:0,message:"No recorded agent batch was found in this session. Completion is unconfirmed."}),
  },
  {
    id: "fixture-agent-status-recovered",
    name: "mcp__flamel__AgentBatchStatus",
    status: "done",
    input: { batch_id: "recovered" },
    output: JSON.stringify({status:"completed_with_errors",agents:[
      {run_id:"recovered-ui",description:"Check the compact layout",status:"success",output:"The saved result remains available after reconnect.\n".repeat(12)},
      {run_id:"recovered-check",description:"Run provider checks",status:"failed",error:"The provider stopped before its final result."},
    ],total:2,completed:2}),
  },
  {
    id: "fixture-workflow-plan",
    name: "mcp__flamel__plan",
    status: "done",
    input: { action: "get", plan_id: "plan-authored-widgets" },
    output:
      'PLAN_GET:{"plan":{"id":"plan-authored-widgets","title":"Authored tool widgets","items":[{"id":"foundation","text":"Shared card foundation","status":"done"},{"id":"workflow","text":"Workflow families","status":"done"},{"id":"adaptive","text":"Adaptive data surfaces","status":"in_progress"},{"id":"release","text":"Visual release gate","status":"todo"}]}}',
  },
  {
    id: "fixture-workflow-agent",
    name: "mcp__flamel__AgentBatch",
    status: "running",
    runId: "batch-craft",
    input: {
      agents: [
        { task_name: "Review visual rhythm", subagent_type: "ui-designer" },
        {
          task_name: "Audit responsive states",
          subagent_type: "frontend-developer",
        },
      ],
    },
    nestedEvents: [
      {
        type: "batch_start",
        run_id: "batch-craft",
        title: "Craft review",
        agents: [
          {
            run_id: "agent-visual",
            task_name: "Review visual rhythm",
            subagent_type: "ui-designer",
            status: "running",
          },
          {
            run_id: "agent-responsive",
            task_name: "Audit responsive states",
            subagent_type: "frontend-developer",
            status: "done",
          },
        ],
      },
      {
        type: "nested_done",
        run_id: "agent-responsive",
        parent_run_id: "batch-craft",
        summary: "Desktop and narrow layouts checked",
      },
    ],
  },
  {
    id: "fixture-workflow-skill",
    name: "mcp__flamel__skill_catalog",
    status: "done",
    input: { action: "inspect", skill_id: "ui-craft" },
    output: JSON.stringify({
      skill: {
        id: "ui-craft",
        title: "UI craft",
        version: "1.4.0",
        signature_status: "verified",
        sha256: "34e7d3a2",
        scope: "project",
        active: true,
        persona: "ui-designer",
        triggers: [
          { name: "interface implementation" },
          { pattern: "visual quality review" },
        ],
      },
    }),
  },
  {
    id: "fixture-workflow-approval",
    name: "mcp__flamel__assistant",
    status: "done",
    input: { action: "automation.delete", profile_id: "nightly-index" },
    output: JSON.stringify({
      contractVersion: 1,
      surfaceContract: "flamel-assistant.v1",
      action: "automation.delete",
      ok: true,
      state: "awaiting_user",
      approvalRequired: true,
      mutated: false,
      replayed: false,
      summary: {
        text: "Delete Nightly index",
        card: {
          title: "Delete Nightly index",
          action: "automation.delete",
          status: "awaiting_user",
          approvalRequired: true,
        },
      },
      result: {
        automation: {
          id: "nightly-index",
          name: "Nightly index",
          cron: "0 2 * * *",
          timezone: "UTC",
        },
      },
    }),
  },
];

const adaptiveFixtures: ToolCall[] = [
  {
    id: "fixture-adaptive-database",
    name: "mcp__flamel__db",
    status: "done",
    input: { action: "query", query: "active customer value" },
    output: JSON.stringify({
      rows: [
        {
          customer: "Northstar",
          segment: "Enterprise",
          value: 4200,
          active: true,
        },
        { customer: "Juniper", segment: "Growth", value: 1850, active: true },
        {
          customer: "Morrow",
          segment: "Enterprise",
          value: 3175,
          active: false,
        },
      ],
    }),
  },
  {
    id: "fixture-adaptive-knowledge",
    name: "mcp__flamel__search_flamel_docs",
    status: "done",
    input: { query: "tool presentation contracts" },
    output: JSON.stringify({
      results: [
        {
          id: "presentation",
          title: "Tool presentation",
          snippet: "Authored surfaces, bounded disclosure, and safe fallbacks.",
          url: "https://docs.flamel.dev/tool-presentation",
          status: "current",
        },
        {
          id: "capabilities",
          title: "Capability metadata",
          snippet: "Deterministic family, adapter, and safe-action routing.",
          url: "https://docs.flamel.dev/capabilities",
          status: "current",
        },
      ],
    }),
  },
  {
    id: "fixture-adaptive-external",
    name: "mcp__metrics__release_health",
    status: "done",
    input: { environment: "production" },
    output: JSON.stringify({
      content: [
        {
          type: "resource_link",
          name: "Release evidence",
          uri: "https://status.flamel.dev/releases/widget-cards",
          mimeType: "text/html",
        },
      ],
      structuredContent: {
        environment: "production",
        metrics: [
          {
            label: "Success rate",
            value: "99.98%",
            delta: "+0.12%",
            trend: "up",
          },
          { label: "p95", value: "182 ms", delta: "-14 ms", trend: "down" },
        ],
        history: [
          {
            id: "deploy",
            title: "Canary complete",
            status: "success",
            timestamp: "02:18 UTC",
          },
          {
            id: "verify",
            title: "Visual gate",
            status: "success",
            timestamp: "02:12 UTC",
          },
        ],
      },
    }),
  },
];

const visualizationFixtures: ToolCall[] = [
  {
    id: "fixture-viz-chart",
    name: "mcp__viz__render_chart",
    status: "done",
    input: {
      type: "bar",
      title: "Widget latency",
      data: [
        { family: "Files", latency: 44 },
        { family: "Browser", latency: 71 },
        { family: "Media", latency: 58 },
        { family: "Data", latency: 39 },
      ],
      xKey: "family",
      yKeys: ["latency"],
    },
  },
  {
    id: "fixture-viz-diagram",
    name: "mcp__viz__render_diagram",
    status: "done",
    input: {
      title: "Presentation flow",
      diagram:
        "flowchart\nTool result -> Adapter\nAdapter -> Authored card\nAuthored card -> Inspector",
    },
  },
  {
    id: "fixture-viz-metric",
    name: "mcp__viz__render_metric",
    status: "done",
    input: {
      metrics: [
        {
          label: "Authored coverage",
          value: "100%",
          delta: "+24%",
          trend: "up",
        },
        { label: "Raw-first cards", value: "0", delta: "−18", trend: "down" },
      ],
    },
  },
  {
    id: "fixture-viz-progress",
    name: "mcp__viz__render_progress",
    status: "done",
    input: {
      title: "Family rollout",
      bars: [
        { label: "Engineering", value: 100 },
        { label: "Media", value: 88 },
        { label: "Workflow", value: 62 },
      ],
    },
  },
  {
    id: "fixture-viz-svg",
    name: "mcp__viz__render_svg",
    status: "done",
    input: {
      title: "Sanitized vector",
      width: 320,
      height: 120,
      svg: '<svg viewBox="0 0 320 120" aria-label="Flamel widget architecture"><rect x="8" y="20" width="86" height="48" rx="5" fill="#191919" stroke="#d96552" /><rect x="117" y="20" width="86" height="48" rx="5" fill="#191919" stroke="#7f8a8f" /><rect x="226" y="20" width="86" height="48" rx="5" fill="#191919" stroke="#2ecf73" /><path d="M94 44h23M203 44h23" stroke="#7f8a8f" /><text x="51" y="49" fill="#eee" text-anchor="middle" font-size="12">result</text><text x="160" y="49" fill="#eee" text-anchor="middle" font-size="12">adapter</text><text x="269" y="49" fill="#eee" text-anchor="middle" font-size="12">widget</text></svg>',
    },
  },
];

const messageFixtures: Array<{ label: string; block: DisplayBlock }> = [
  {
    label: "Completed · ordered segments",
    block: {
      id: "gallery-message-complete",
      role: "assistant",
      type: "assistant",
      model: "gpt-5.6-codex",
      content: "The authored renderer is now isolated from tool fallbacks.",
      segments: [
        {
          type: "thinking",
          text: "Keep the message order stable while moving renderer ownership.",
        },
        {
          type: "text",
          text: "The authored renderer is now isolated from tool fallbacks.",
        },
      ],
    },
  },
  {
    label: "Streaming · inline activity",
    block: {
      id: "gallery-message-streaming",
      role: "assistant",
      type: "assistant",
      model: "gpt-5.6-codex",
      content: "Running the focused renderer checks.",
      segments: [
        { type: "text", text: "Running the focused renderer checks." },
      ],
      isStreaming: true,
      isStarted: true,
      statusText: "__thinking__",
    },
  },
  {
    label: "Completed · context checkpoint",
    block: {
      id: "gallery-message-checkpoint",
      role: "assistant",
      type: "compact",
      compaction: {
        providerNative: true,
        preTokens: 128_000,
        preservedRecentMessages: 12,
      },
    },
  },
];

export default function ToolWidgetGallery() {
  return (
    <main className="tool-widget-gallery">
      <div className="tool-widget-gallery__frame">
        <header className="tool-widget-gallery__intro">
          <h1>Tool widget foundation</h1>
          <p>
            Interaction, density, responsive behavior, raw-data escape hatches,
            and decorative asset fallbacks in one deterministic fixture.
          </p>
        </header>

        <div className="tool-widget-gallery__stack">
          <p className="tool-widget-gallery__section-label">
            Assistant message composition
          </p>
          {messageFixtures.map(({ label, block }) => (
            <section
              key={block.id}
              style={{
                minWidth: 0,
                overflow: "hidden",
                padding: "10px 0 2px",
                border: "1px solid var(--border-soft)",
                borderRadius: "var(--radius-md)",
                background: "var(--surface)",
              }}
            >
              <span
                className="tool-widget-gallery__section-label"
                style={{ display: "block", margin: 0, padding: "0 12px 5px" }}
              >
                {label}
              </span>
              <ClaudeBlock block={block} />
            </section>
          ))}

          <p className="tool-widget-gallery__section-label">
            Expanded · successful
          </p>
          <ToolWidgetShell
            title="Workspace read"
            eyebrow="Filesystem"
            summary="3 files matched the request"
            detail="frontend/src"
            status="success"
            icon={<Icon.FolderOpen size={14} />}
            sprite="workspace"
            defaultExpanded
            trailing="14.2 KB"
            inspector={
              <ToolWidgetInspector
                input={{ action: "glob", pattern: "**/*.tsx" }}
                output={{ matches: 3, truncated: false }}
                raw={{ version: 1, resources: [] }}
              />
            }
          >
            <ToolWidgetSection
              title="Result"
              description="Curated evidence stays ahead of provider payloads."
            >
              <ToolWidgetFields
                fields={[
                  { label: "Root", value: "frontend/src", mono: true },
                  { label: "Pattern", value: "**/*.tsx", mono: true },
                  { label: "Matches", value: "3" },
                  { label: "Duration", value: "42 ms", mono: true },
                ]}
              />
              <ToolWidgetList
                items={[
                  {
                    id: "app",
                    primary: "App.tsx",
                    secondary: "frontend/src/App.tsx",
                    trailing: "510 lines",
                  },
                  {
                    id: "shell",
                    primary: "Shell.tsx",
                    secondary: "frontend/src/components/layout/Shell.tsx",
                    trailing: "1,104 lines",
                  },
                  {
                    id: "block",
                    primary: "ClaudeBlock.tsx",
                    secondary: "frontend/src/components/chat/ClaudeBlock.tsx",
                    trailing: "820 lines",
                  },
                ]}
              />
            </ToolWidgetSection>
          </ToolWidgetShell>

          <p className="tool-widget-gallery__section-label">
            Expanded · streaming
          </p>
          <ToolWidgetShell
            title="Verification"
            eyebrow="Runtime"
            summary="Frontend checks are still running"
            status="running"
            icon={<Icon.Server size={14} />}
            sprite="terminal"
            defaultExpanded
            trailing="00:18"
          >
            <ToolWidgetTimeline
              items={[
                {
                  id: "typecheck",
                  title: "TypeScript",
                  detail: "No type errors",
                  meta: "done",
                  status: "success",
                },
                {
                  id: "tests",
                  title: "Focused widget tests",
                  detail: "Executing fixture and interaction cases",
                  meta: "18/27",
                  status: "running",
                },
                {
                  id: "build",
                  title: "Production build",
                  detail: "Waiting for tests",
                  meta: "queued",
                  status: "pending",
                },
              ]}
            />
            <ToolWidgetState
              status="running"
              title="Collecting evidence"
              detail="Partial output updates in place without changing the card geometry."
            />
          </ToolWidgetShell>

          <p className="tool-widget-gallery__section-label">
            Collapsed · compact receipt
          </p>
          <ToolWidgetShell
            title="Source control"
            eyebrow="Git"
            summary="2 files changed"
            detail="fix/chat-terminal-continuity"
            status="success"
            icon={<Icon.Branch size={14} />}
            sprite="source-control"
            trailing="+84 −17"
          >
            <ToolWidgetState status="empty" title="Open to inspect the diff" />
          </ToolWidgetShell>

          <p className="tool-widget-gallery__section-label">
            Expanded · media and data
          </p>
          <ToolWidgetShell
            title="Generated media"
            eyebrow="Asset"
            summary="Workspace illustration is ready"
            status="success"
            icon={<Icon.Image size={14} />}
            sprite="media"
            defaultExpanded
          >
            <ToolWidgetTabs
              tabs={[
                {
                  id: "artifact",
                  label: "Artifact",
                  content: <ToolArtifactActions artifacts={fixtureArtifacts} />,
                },
                {
                  id: "metadata",
                  label: "Metadata",
                  content: (
                    <ToolWidgetTable
                      caption="Managed artifact metadata"
                      columns={[
                        { key: "property", label: "Property" },
                        { key: "value", label: "Value" },
                      ]}
                      rows={[
                        {
                          id: "type",
                          property: "Media type",
                          value: "image/webp",
                        },
                        { id: "size", property: "Size", value: "18 KB" },
                        {
                          id: "alpha",
                          property: "Transparency",
                          value: "Preserved",
                        },
                      ]}
                    />
                  ),
                },
              ]}
            />
          </ToolWidgetShell>

          <p className="tool-widget-gallery__section-label">
            Error and empty states
          </p>
          <ToolWidgetShell
            title="Browser inspection"
            eyebrow="Browser"
            summary="The requested page did not respond"
            status="error"
            icon={<Icon.ExternalLink size={14} />}
            sprite="browser-research"
            defaultExpanded
          >
            <ToolWidgetState
              status="error"
              title="Navigation timed out"
              detail="The last stable page and collected console evidence remain available."
            />
          </ToolWidgetShell>

          <ToolWidgetShell
            title="Query result"
            eyebrow="Data"
            summary="No rows matched the current filter"
            status="empty"
            icon={<Icon.Drive size={14} />}
            sprite="data"
            defaultExpanded
          >
            <ToolWidgetState
              status="empty"
              title="No matching rows"
              detail="The query completed successfully; adjust the filter to broaden the result."
            />
          </ToolWidgetShell>

          <p className="tool-widget-gallery__section-label">
            Collapsed · Monaco loads on demand
          </p>
          <ToolWidgetShell
            title="Code changes"
            eyebrow="Diff"
            summary="4 files are ready to inspect"
            detail="Monaco mounts only after disclosure"
            status="success"
            icon={<Icon.Diff size={14} />}
            sprite="source-control"
            trailing="+10 −6"
          >
            <EmbeddedDiffView
              files={fixtureDiffFiles}
              maximumHeight={360}
              onOpenInIde={(file) =>
                window.dispatchEvent(
                  new CustomEvent("flamel:fixture-open-ide", {
                    detail: { path: file.path },
                  }),
                )
              }
            />
          </ToolWidgetShell>

          <p className="tool-widget-gallery__section-label">
            Engineering adapters · live renderer
          </p>
          {engineeringFixtures.map((tool) => (
            <EngineeringToolWidget key={tool.id} tool={tool} defaultExpanded />
          ))}

          <p className="tool-widget-gallery__section-label">
            Media, browser, and research · live renderer
          </p>
          <MediaToolWidget tool={mediaFixture} defaultExpanded />
          <ResearchBrowserToolWidget
            tool={researchFixture}
            kind="research"
            defaultExpanded
          />
          <ResearchBrowserToolWidget
            tool={browserFixture}
            kind="browser"
            defaultExpanded
          />

          <p className="tool-widget-gallery__section-label">
            Workflow, agents, and governed skills · live renderer
          </p>
          {workflowFixtures.map((tool) => (
            <WorkflowToolWidget key={tool.id} tool={tool} defaultExpanded />
          ))}

          <p className="tool-widget-gallery__section-label">
            Adaptive data, knowledge, and external receipts · live renderer
          </p>
          {adaptiveFixtures.map((tool) => (
            <AdaptiveDataToolWidget key={tool.id} tool={tool} defaultExpanded />
          ))}

          <p className="tool-widget-gallery__section-label">
            Visualization engine · all renderers
          </p>
          {visualizationFixtures.map((tool) => (
            <VizRenderer key={tool.id} tool={tool} defaultExpanded />
          ))}

          <p className="tool-widget-gallery__section-label">Pending workflow</p>
          <ToolWidgetShell
            title="Assistant automation"
            eyebrow="Workflow"
            summary="Waiting for approval"
            detail="schedule.create"
            status="pending"
            icon={<Icon.Clock size={14} />}
            sprite="assistant-workflow"
          >
            <ToolWidgetState
              status="pending"
              title="Approval required"
              detail="This preview cannot mutate the schedule until it is confirmed."
            />
          </ToolWidgetShell>
        </div>
      </div>
    </main>
  );
}
