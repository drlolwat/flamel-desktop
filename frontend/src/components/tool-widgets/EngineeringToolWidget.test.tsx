import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { CommandEvent } from "../../lib/consoleTypes";
import {
  normalizeToolPresentation,
  type ToolCall,
} from "../../lib/messageProcessor";
import { FLAMEL_TOOL_CAPABILITIES } from "../../lib/toolCapabilities";
import protocolFixture from "./engineeringResultV1.fixture.json";
import { applyStreamingToolResult } from "../../lib/chatToolEvents";
import { legacySkillBatchText, skillBatchFiles } from "./skillBatch.fixture";
import {
  EngineeringToolWidget,
  selectEngineeringConsoleEvents,
} from "./EngineeringToolWidget";
import {
  adaptEngineeringTool,
  engineeringAction,
  engineeringInput,
  engineeringWidgetKind,
} from "./engineeringToolAdapters";

function tool(
  name: string,
  input: Record<string, unknown> = {},
  output = "",
  status: ToolCall["status"] = "done",
): ToolCall {
  return {
    id: `tool-${name}-${status}`,
    name,
    input,
    output,
    status,
  };
}

function structuredEngineeringTool(
  name: "file_read" | "code_nav",
  action: string,
  data: Record<string, unknown>,
  options: {
    status?: ToolCall["status"];
    resultStatus?: "ok" | "error";
    truncated?: boolean;
    warnings?: string[];
    artifacts?: ToolCall["artifacts"];
    summary?: string;
    text?: string;
  } = {},
): ToolCall {
  const summary = options.summary ?? action + " result";
  const text = options.text ?? summary;
  const structuredContent = {
    schema: "flamel.engineering_result.v1",
    tool: name,
    action,
    status: options.resultStatus ?? "ok",
    summary,
    counts: {},
    truncated: options.truncated ?? false,
    truncationReason: options.truncated ? "producer_limit" : null,
    warnings: options.warnings ?? [],
    artifacts: [],
    data,
    text,
  };
  const content = {
    content: [{ type: "text", text }],
    structuredContent,
  };
  return {
    ...tool(name, { action }, text, options.status ?? "done"),
    presentation: normalizeToolPresentation({
      content,
      artifacts: options.artifacts,
      isError: options.resultStatus === "error",
    }),
  };
}

describe("engineering tool capability routing", () => {
  it("offers an interactive terminal only for a real supervised shell handle", () => {
    const result = { session_id: "pty:terminal-1", terminal_session_id: "terminal-1", command_session_id: "command-1", command: "node", cwd: "/project" };
    const render = (output: Record<string, unknown>) => renderToStaticMarkup(
      <EngineeringToolWidget tool={tool("shell", { action: "start", command: "node" }, JSON.stringify(output))} defaultExpanded />,
    );
    expect(render(result)).toContain("Open terminal");
    expect(render({ ...result, session_id: "legacy-process-1" })).not.toContain("Open terminal");
    expect(render({ session_id: "legacy-process-1" })).not.toContain("Open terminal");
  });

  const expected = new Map<string, ReturnType<typeof engineeringWidgetKind>>([
    ["file_read", "workspace"],
    ["file_edit", "workspace"],
    ["file_history", "workspace"],
    ["code_nav", "code"],
    ["project_intelligence", "code"],
    ["git", "source-control"],
    ["get_recent_changes", "source-control"],
    ["shell", "shell"],
    ["code_quality", "diagnostics"],
    ["diagnostics", "diagnostics"],
    ["flamel_context_stats", "diagnostics"],
  ]);

  it("routes every registered engineering alias through its authored family", () => {
    for (const [canonical, kind] of expected) {
      const capability = FLAMEL_TOOL_CAPABILITIES.find(
        (candidate) => candidate.name === canonical,
      );
      expect(capability, canonical).toBeTruthy();
      for (const name of [canonical, ...(capability?.aliases ?? [])]) {
        expect(engineeringWidgetKind(tool(`mcp__flamel__${name}`)), name).toBe(
          kind,
        );
      }
    }
  });

  it("covers consolidated actions and action-sensitive project verification", () => {
    for (const action of [
      "read",
      "batch",
      "summary",
      "glob",
      "find",
      "list_dir",
    ]) {
      expect(engineeringWidgetKind(tool("file_read", { action })), action).toBe(
        "workspace",
      );
    }
    for (const action of [
      "replace",
      "replace_content",
      "multi_replace",
      "replace_symbol",
      "insert_before",
      "insert_after",
      "create",
      "delete",
      "diff_apply",
      "format",
      "preview",
    ]) {
      expect(engineeringWidgetKind(tool("file_edit", { action })), action).toBe(
        "workspace",
      );
    }
    for (const action of [
      "search",
      "symbols",
      "find",
      "refs",
      "imports",
      "context",
      "deps",
    ]) {
      expect(engineeringWidgetKind(tool("code_nav", { action })), action).toBe(
        "code",
      );
    }
    for (const action of [
      "exec",
      "run",
      "start",
      "read",
      "write",
      "kill",
      "status",
      "multi",
    ]) {
      expect(engineeringWidgetKind(tool("shell", { action })), action).toBe(
        "shell",
      );
    }
    expect(
      engineeringWidgetKind(
        tool("project_intelligence", { action: "context" }),
      ),
    ).toBe("code");
    expect(
      engineeringWidgetKind(
        tool("project_intelligence", { action: "verification" }),
      ),
    ).toBe("diagnostics");
    expect(
      engineeringWidgetKind(tool("project_intelligence", { action: "verify" })),
    ).toBe("diagnostics");
  });
});

describe("engineering tool adapters", () => {
  it("renders shell output reads as reads, not completed processes", () => {
    const output = JSON.stringify({ content: [{ type: "text", text: JSON.stringify({
      output: "Server listening", exited: false, exit_code: null,
    }) }] });
    for (const name of ["shell", "mcp__flamel__shell_read"]) {
      const read = tool(name, { action: "read", session_id: "pty:server" }, output);
      expect(adaptEngineeringTool(read)?.summary).toBe("Read output · command still running");
      const html = renderToStaticMarkup(<EngineeringToolWidget tool={read} />);
      expect(html).toContain("Read output · command still running");
      expect(html).not.toContain("Command completed");
    }
  });

  it("keeps a running command independent of unrelated reads and settles on its own receipt", () => {
    const command = { ...tool("shell", { action: "run", command: "npm test" }, "", "running"), id: "command" };
    const tools = [command, ...["first", "second"].map(id => ({
      ...tool("shell", { action: "read", session_id: `pty:${id}` }, "", "running"), id,
    }))];
    const pending = new Map();
    const apply = (id: string, output: string) => applyStreamingToolResult(tools,
      { tool_use_id: id, content: output, is_error: false, is_delta: false },
      () => {}, () => {}, (_id, events) => events, () => undefined, pending);
    for (const id of ["first", "second"]) apply(id, JSON.stringify({ exited: false, exit_code: null }));
    expect(adaptEngineeringTool(tools[0])).toMatchObject({ status: "running", summary: "Running npm test" });
    for (const read of tools.slice(1)) expect(adaptEngineeringTool(read)?.summary).toBe("Read output · command still running");
    apply("command", "8 tests passed");
    expect(adaptEngineeringTool(tools[0])).toMatchObject({ status: "success", summary: "Ran npm test" });
    const html = renderToStaticMarkup(<EngineeringToolWidget tool={tools[0]} />);
    expect(html).toContain("Done");
    expect(html).not.toContain("Live");
  });

  it("unwraps MCP file output and presents source instead of wrapper JSON", () => {
    const view = adaptEngineeringTool(
      tool(
        "mcp__flamel__file_read",
        { action: "read", file: "frontend/src/App.tsx" },
        JSON.stringify({
          content: [
            {
              type: "text",
              text: '     1\timport React from "react";\n     2\texport function App() {}\n[frontend/src/App.tsx: lines 1-2 of 2]',
            },
          ],
        }),
      ),
    );

    expect(view).toMatchObject({
      kind: "workspace",
      action: "read",
      path: "frontend/src/App.tsx",
      status: "success",
      summary: "Read App.tsx",
    });
    expect(view?.code).toBe(
      'import React from "react";\nexport function App() {}',
    );
    expect(view?.code).not.toContain('"content"');
  });

  it("turns persisted string-only file batches into one compact source preview", () => {
    const batchTool = tool(
      "mcp__flamel__file_read",
      { action: "batch", files: skillBatchFiles.map((file) => file.path) },
      legacySkillBatchText,
    );
    const view = adaptEngineeringTool(batchTool);
    const files = view?.structuredResult?.data.files as
      Array<{ path: string; source: string }> | undefined;
    const markup = renderToStaticMarkup(
      <EngineeringToolWidget tool={batchTool} defaultExpanded />,
    );

    expect(view).toMatchObject({
      summary: "Read 6 files · flamel-ui-implementation/SKILL.md, flamel-frontend-engineering/SKILL.md +4",
      fields: [],
      entries: [],
      status: "success",
      structuredResult: {
        schema: "flamel.engineering_result.v1",
        action: "batch",
        counts: { files: 6, lines: 19, totalLines: 19 },
      },
    });
    expect(files).toHaveLength(6);
    expect(files?.[0].source).toContain("  - Preserve nested indentation.");
    expect(markup.match(/role="tab"/g)).toHaveLength(6);
    expect(markup).toContain("flamel-ui-implementation");
    expect(markup).toContain("SKILL.md");
    expect(markup).toContain("L1–4 of 4");
    expect(markup).toContain("markdown");
    expect(markup).toContain("Copy");
    expect(markup).toContain("Open in IDE");
    expect(markup).toContain("Inspect raw data");
    expect(markup).toContain("Keep the hierarchy compact");
    expect(markup).not.toContain("Frontend engineering");
    expect(markup).not.toContain("=== ");
    expect(markup).not.toContain("engineering-widget__evidence");
  });

  it("builds complete and patch diff presentations for every edit shape", () => {
    const replace = adaptEngineeringTool(
      tool("file_edit", {
        action: "replace",
        file: "src/a.ts",
        old_string: "const oldValue = 1;",
        new_string: "const newValue = 2;",
      }),
    );
    expect(replace?.diffFiles?.[0]).toMatchObject({
      path: "src/a.ts",
      original: "const oldValue = 1;",
      modified: "const newValue = 2;",
      status: "modified",
    });

    const created = adaptEngineeringTool(
      tool("create_text_file", {
        relative_path: "src/new.ts",
        content: "export const ready = true;",
      }),
    );
    expect(created?.diffFiles?.[0]).toMatchObject({
      path: "src/new.ts",
      original: "",
      status: "added",
    });

    const multi = adaptEngineeringTool(
      tool("multi_replace", {
        edits: [
          { file: "src/a.ts", old_string: "a", new_string: "b" },
          { file: "src/b.ts", old_string: "", new_string: "created" },
        ],
      }),
    );
    expect(multi?.diffFiles).toHaveLength(2);

    const patch = adaptEngineeringTool(
      tool("diff_apply", {
        diff: "--- a/src/a.ts\n+++ b/src/a.ts\n@@ -1 +1 @@\n-old\n+new",
      }),
    );
    expect(patch?.diffPatch).toContain("+++ b/src/a.ts");
  });

  it("normalizes structured code evidence and caps noisy result lists", () => {
    const matches = Array.from({ length: 80 }, (_, index) => ({
      path: `src/file-${index}.ts`,
      line: index + 1,
      message: "matched rendererForTool",
    }));
    const view = adaptEngineeringTool(
      tool(
        "code_nav",
        { action: "search", pattern: "rendererForTool", path: "frontend/src" },
        JSON.stringify({ structuredContent: { matches } }),
      ),
    );

    expect(view?.kind).toBe("code");
    expect(view?.entries).toHaveLength(60);
    expect(view?.entries[0]).toMatchObject({
      primary: "src/file-0.ts",
      secondary: "matched rendererForTool",
      trailing: "1",
    });
    expect(view?.fields.map((field) => field.label)).toEqual(["Pattern"]);
  });

  it("renders symbol bodies in the embedded code surface", () => {
    const output = [
      "Symbol 'send_turn' in /workspace/src/personal_assistant.rs (L1299-1305):",
      "1299\tpub(crate) async fn send_turn(",
      "1300\t    state: &Arc<AppState>,",
      "1301\t) -> Result<(), String> {",
      "1302\t    Ok(())",
      "1303\t}",
    ].join("\n");
    const findTool = tool(
      "code_nav",
      {
        action: "find",
        relative_path: "src/personal_assistant.rs",
        name_path: "send_turn",
        include_body: true,
      },
      output,
    );
    const view = adaptEngineeringTool(findTool);
    const markup = renderToStaticMarkup(
      <EngineeringToolWidget tool={findTool} defaultExpanded />,
    );

    expect(view).toMatchObject({
      path: "src/personal_assistant.rs",
      code: [
        "pub(crate) async fn send_turn(",
        "    state: &Arc<AppState>,",
        ") -> Result<(), String> {",
        "    Ok(())",
        "}",
      ].join("\n"),
      entries: [],
    });
    expect(markup).toContain("Source for src/personal_assistant.rs");
    expect(markup).toContain("Open in IDE");
    expect(markup).not.toContain(
      "Bounded matches from the requested project scope",
    );
  });

  it("renders git status as receipts and git patches as Monaco-ready diffs", () => {
    const status = adaptEngineeringTool(
      tool(
        "git",
        { action: "repo.status", path: "/workspace/flamel" },
        " M src/a.ts\n?? src/new.ts",
      ),
    );
    expect(status).toMatchObject({
      kind: "source-control",
      summary: "2 changed items",
      status: "success",
    });
    expect(status?.entries.map((entry) => entry.trailing)).toEqual(["M", "??"]);

    const diff = adaptEngineeringTool(
      tool(
        "git",
        { action: "repo.diff" },
        "diff --git a/src/a.ts b/src/a.ts\n--- a/src/a.ts\n+++ b/src/a.ts\n@@ -1 +1 @@\n-old\n+new",
      ),
    );
    expect(diff?.diffPatch).toContain("diff --git");
    expect(diff?.summary).toBe("Source diff is ready");
  });

  it("renders pull requests and checks without dumping provider JSON", () => {
    const output = JSON.stringify({
      checks: [
        {
          id: "quality",
          name: "chattle quality",
          status: "in_progress",
        },
      ],
      issue: {
        id: "pr-2",
        number: 2,
        state: "open",
        title: "Add bot typing presence API",
      },
      pull: {
        head_branch: "fix/bot-typing-presence",
        base_branch: "main",
      },
    });
    const statusTool = tool(
      "git",
      {
        action: "flamel.pr.status",
        owner: "lolwat",
        repo: "chattle",
        number: 2,
      },
      output,
    );
    const view = adaptEngineeringTool(statusTool);
    const markup = renderToStaticMarkup(
      <EngineeringToolWidget tool={statusTool} defaultExpanded />,
    );

    expect(view).toMatchObject({
      summary: "1 of 1 checks running",
      entries: [
        {
          primary: "#2 Add bot typing presence API",
          secondary: "fix/bot-typing-presence → main",
          trailing: "open",
        },
      ],
      checks: [
        {
          name: "chattle quality",
          status: "running",
        },
      ],
    });
    expect(markup).toContain("Pull request checks");
    expect(markup).toContain("#2 Add bot typing presence API");
    expect(markup).not.toContain("&quot;head_branch&quot;");
    expect(markup).not.toContain("&quot;author_id&quot;");
  });

  it("builds diagnostic check matrices with status and duration", () => {
    const view = adaptEngineeringTool(
      tool(
        "project_intelligence",
        { action: "verification", project_path: "/workspace/flamel" },
        JSON.stringify({
          content: [{ type: "text", text: "Verification graph ready" }],
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
                name: "Focused tests",
                status: "failed",
                message: "1 failed",
              },
            ],
          },
        }),
      ),
    );

    expect(view).toMatchObject({
      kind: "diagnostics",
      status: "success",
      summary: "1 of 2 checks failed",
    });
    expect(view?.checks).toEqual([
      expect.objectContaining({
        name: "TypeScript",
        status: "passed",
        duration: "420 ms",
      }),
      expect.objectContaining({
        name: "Focused tests",
        status: "failed",
        detail: "1 failed",
      }),
    ]);
  });

  it("preserves partial inputs and distinguishes running, error, empty, and mutation receipts", () => {
    const partial = {
      ...tool("shell", {}, "", "running"),
      partialInput: '{"action":"run","command":"npm test"',
    };
    expect(engineeringInput(partial)).toMatchObject({
      action: "run",
      command: "npm test",
    });
    expect(engineeringAction(partial)).toBe("run");
    expect(adaptEngineeringTool(partial)).toMatchObject({
      kind: "shell",
      status: "running",
      command: "npm test",
      summary: "Running npm test",
    });

    expect(
      adaptEngineeringTool(tool("file_read", { action: "find" })),
    ).toMatchObject({
      status: "empty",
      summary: "No matching files",
    });
    expect(
      adaptEngineeringTool(
        tool("file_edit", { action: "format", file: "src/a.ts" }),
      ),
    ).toMatchObject({
      status: "success",
      summary: "Formatted a.ts",
    });
    expect(
      adaptEngineeringTool(
        tool("code_nav", { action: "search" }, "provider failed", "error"),
      ),
    ).toMatchObject({
      status: "error",
      summary: "Code inspection failed",
    });
  });

  it("renders bounded source and artifact actions instead of the legacy budget notice", () => {
    const legacyEnvelope = JSON.stringify({
      status: "ok",
      summary: "Output exceeded the 12288 byte inline budget",
      data: { preview: "     1\texport const mobile = true;" },
      artifacts: [
        {
          path: "/tmp/flamel-mcp-artifacts/file_read-mobile.txt",
          media_type: "text/plain",
          bytes: 18_432,
        },
      ],
      warnings: [],
      next_actions: ["Read the artifact only if the preview is insufficient"],
    });
    const boundedTool = {
      ...tool(
        "file_read",
        { action: "read", file: "src/mobile.ts" },
        legacyEnvelope,
      ),
      presentation: normalizeToolPresentation({ content: legacyEnvelope }),
    };
    const view = adaptEngineeringTool(boundedTool);
    const markup = renderToStaticMarkup(
      <EngineeringToolWidget tool={boundedTool} defaultExpanded />,
    );

    expect(view).toMatchObject({
      code: "export const mobile = true;",
      statusLabel: "Done · clipped",
    });
    expect(markup).toContain("Preview clipped");
    expect(markup).toContain("Open");
    expect(markup).toContain("Download");
    expect(markup).not.toContain("Output exceeded");
    expect(markup).not.toContain("12288");
  });

  it("keeps clipped error and missing-artifact states honest", () => {
    const envelope = JSON.stringify({
      schema: "flamel.oversized_result.v1",
      status: "error",
      summary:
        "The tool returned an error; the full output is available as an artifact.",
      data: {
        preview: "     1\tpermission denied",
        truncated: true,
        original: { media_type: "text/plain", bytes: 12_600 },
      },
      artifacts: [],
      diagnostics: {
        reason: "inline_budget_exceeded",
        inline_budget_bytes: 12_288,
      },
    });
    const errorTool = {
      ...tool(
        "file_read",
        { action: "read", file: "src/private.ts" },
        envelope,
        "error",
      ),
      presentation: normalizeToolPresentation({
        content: envelope,
        isError: true,
      }),
    };
    const view = adaptEngineeringTool(errorTool);
    const markup = renderToStaticMarkup(
      <EngineeringToolWidget tool={errorTool} defaultExpanded />,
    );

    expect(view).toMatchObject({
      code: "permission denied",
      status: "error",
      truncated: true,
      artifacts: [],
    });
    expect(markup).toContain("Preview clipped");
    expect(markup).toContain("Full result unavailable");
    expect(markup).toContain("permission denied");
    expect(markup).not.toContain("inline_budget_exceeded");
  });

  it("consumes the shared v1 contract with an action-native renderer for all actions", () => {
    const samples: Record<string, Record<string, unknown>> = {
      "file_read:read": {
        files: [
          {
            path: "src/read.ts",
            requestedRange: { startLine: 1, endLine: 2 },
            resolvedRange: { startLine: 1, endLine: 2 },
            totalLines: 2,
            language: "typescript",
            source: "export const read = true;",
          },
        ],
      },
      "file_read:batch": {
        files: [
          {
            path: "src/first.ts",
            resolvedRange: { startLine: 1, endLine: 1 },
            totalLines: 1,
            language: "typescript",
            source: "export const first = true;",
          },
          {
            path: "src/second.ts",
            resolvedRange: { startLine: 1, endLine: 1 },
            totalLines: 1,
            language: "typescript",
            source: "export const second = true;",
          },
        ],
      },
      "file_read:summary": {
        path: "src/summary.ts",
        purpose: "Assistant orchestration",
        imports: [{ line: 1, text: "import React from \"react\";", kind: "module" }],
        symbols: [{ name: "Summary", kind: "function", startLine: 3, endLine: 8 }],
      },
      "file_read:glob": {
        root: "src",
        pattern: "*.ts",
        paths: [{ path: "src/a.ts", kind: "file" }],
      },
      "file_read:find": {
        root: "src",
        query: "assistant",
        mode: "name",
        paths: [{ path: "src/assistant.ts", kind: "file", score: 0.9 }],
      },
      "file_read:list_dir": {
        root: "src",
        paths: [{ name: "components", path: "src/components", kind: "directory" }],
      },
      "code_nav:search": {
        query: "dispatch",
        root: "src",
        contextLines: 1,
        matches: [
          { path: "src/a.ts", line: 10, column: 2, text: "dispatch()", role: "match" },
          { path: "src/a.ts", line: 11, column: 1, text: "return;", role: "context" },
          { path: "src/b.ts", line: 4, column: 3, text: "dispatch()", role: "match" },
        ],
      },
      "code_nav:symbols": {
        path: "src/a.ts",
        language: "typescript",
        symbols: [
          {
            name: "Assistant",
            kind: "class",
            startLine: 1,
            endLine: 20,
            methods: [{ name: "run", kind: "method", startLine: 5, endLine: 10 }],
          },
        ],
      },
      "code_nav:find": {
        path: "src/a.ts",
        namePath: "Assistant/run",
        language: "typescript",
        range: { startLine: 5, endLine: 10 },
        source: "run() { return true; }",
        children: [{ name: "nested", kind: "function", startLine: 7, endLine: 8 }],
      },
      "code_nav:refs": {
        symbol: "run",
        root: "src",
        contextLines: 1,
        matches: [{ path: "src/b.ts", line: 7, column: 4, text: "assistant.run()", role: "match" }],
      },
      "code_nav:imports": {
        path: "src/a.ts",
        language: "typescript",
        totalLines: 20,
        imports: [{ line: 1, text: "import { api } from \"./api\";", kind: "named" }],
      },
      "code_nav:context": {
        task: "render assistant results",
        keywords: ["assistant", "result"],
        files: [
          {
            path: "src/a.ts",
            score: 0.94,
            language: "typescript",
            symbols: ["Assistant", "render"],
          },
        ],
      },
      "code_nav:deps": {
        path: "src/a.ts",
        outbound: [{ from: "src/a.ts", to: "src/api.ts", kind: "import", line: 1 }],
        inbound: [{ from: "src/app.ts", to: "src/a.ts", kind: "import" }],
      },
    };

    expect(protocolFixture.schema).toBe("flamel.engineering_result.v1");
    expect(protocolFixture.actions).toHaveLength(13);
    for (const descriptor of protocolFixture.actions) {
      const key = descriptor.tool + ":" + descriptor.action;
      const name = descriptor.tool as "file_read" | "code_nav";
      const renderedTool = structuredEngineeringTool(name, descriptor.action, samples[key]);
      const view = adaptEngineeringTool(renderedTool);
      const markup = renderToStaticMarkup(
        <EngineeringToolWidget tool={renderedTool} defaultExpanded />,
      );

      expect(view?.structuredResult, key).toMatchObject({
        schema: protocolFixture.schema,
        tool: descriptor.tool,
        action: descriptor.action,
      });
      expect(view?.entries, key).toEqual([]);
      expect(markup, key).not.toContain("Unsupported engineering result");
      expect(markup, key).not.toContain("flamel.engineering_result.v1");
    }
  });

  it("keeps batch source lazy and groups search context by file", () => {
    const batch = structuredEngineeringTool("file_read", "batch", {
      files: [
        {
          path: "src/first.ts",
          resolvedRange: { startLine: 2, endLine: 4 },
          totalLines: 20,
          language: "typescript",
          source: "export const firstOnly = true;",
        },
        {
          path: "src/second.ts",
          resolvedRange: { startLine: 8, endLine: 9 },
          totalLines: 30,
          language: "typescript",
          source: "export const secondOnly = true;",
        },
      ],
    });
    const batchMarkup = renderToStaticMarkup(
      <EngineeringToolWidget tool={batch} defaultExpanded />,
    );

    expect(batchMarkup.match(/role="tab"/g)).toHaveLength(2);
    expect(batchMarkup).toContain("first.ts");
    expect(batchMarkup).toContain("second.ts");
    expect(batchMarkup).toContain("L2–4 of 20");
    expect(batchMarkup).toContain("firstOnly");
    expect(batchMarkup).not.toContain("secondOnly");

    const search = structuredEngineeringTool("code_nav", "search", {
      matches: [
        { path: "src/a.ts", line: 1, column: 2, text: "before", role: "context" },
        { path: "src/a.ts", line: 2, column: 4, text: "needle", role: "match" },
        { path: "src/b.ts", line: 9, column: 1, text: "needle", role: "match" },
      ],
    });
    const searchMarkup = renderToStaticMarkup(
      <EngineeringToolWidget tool={search} defaultExpanded />,
    );
    expect(searchMarkup).toContain("Code matches by file");
    expect(searchMarkup).toContain("src/a.ts");
    expect(searchMarkup).toContain("src/b.ts");
    expect(searchMarkup).toContain('data-role="context"');
    expect(searchMarkup.match(/<summary><span title="src\//g)).toHaveLength(2);
  });

  it("bounds huge and long structured results and renders zero, error, and artifact states", () => {
    const hugeBatch = structuredEngineeringTool("file_read", "batch", {
      files: Array.from({ length: 80 }, (_, index) => ({
        path: "src/file-" + index + ".ts",
        resolvedRange: { startLine: 1, endLine: 1 },
        totalLines: 1,
        language: "typescript",
        source: "export const file" + index + " = true;",
      })),
    });
    const hugeMarkup = renderToStaticMarkup(
      <EngineeringToolWidget tool={hugeBatch} defaultExpanded />,
    );
    expect(hugeMarkup.match(/role="tab"/g)).toHaveLength(32);

    const longSearch = structuredEngineeringTool("code_nav", "search", {
      matches: [
        {
          path: "src/long.ts",
          line: 1,
          column: 1,
          text: "x".repeat(20_000),
          role: "match",
        },
      ],
    });
    const longMarkup = renderToStaticMarkup(
      <EngineeringToolWidget tool={longSearch} defaultExpanded />,
    );
    expect(longMarkup.length).toBeLessThan(30_000);
    expect(longMarkup).toContain("xxxxxxxx");

    const empty = structuredEngineeringTool("file_read", "glob", { paths: [] });
    expect(
      renderToStaticMarkup(<EngineeringToolWidget tool={empty} defaultExpanded />),
    ).toContain("No workspace paths");

    const failed = structuredEngineeringTool(
      "code_nav",
      "search",
      {},
      {
        resultStatus: "error",
        summary: "Search failed",
        text: "permission denied",
      },
    );
    const failedMarkup = renderToStaticMarkup(
      <EngineeringToolWidget tool={failed} defaultExpanded />,
    );
    expect(adaptEngineeringTool(failed)?.status).toBe("error");
    expect(failedMarkup).toContain("Search failed");
    expect(failedMarkup).toContain("permission denied");

    const clipped = structuredEngineeringTool(
      "file_read",
      "read",
      {
        files: [
          {
            path: "src/clipped.ts",
            resolvedRange: { startLine: 1, endLine: 1 },
            totalLines: 200,
            language: "typescript",
            source: "export const clipped = true;",
          },
        ],
      },
      {
        truncated: true,
        warnings: ["Additional lines were preserved in the artifact."],
        artifacts: [
          {
            id: "artifact-clipped",
            kind: "file",
            name: "clipped.txt",
            mediaType: "text/plain",
            byteSize: 1000,
            sha256: "abc",
            contentUrl: "/api/artifacts/artifact-clipped/content",
            downloadUrl: "/api/artifacts/artifact-clipped/download",
          },
        ],
      },
    );
    const clippedMarkup = renderToStaticMarkup(
      <EngineeringToolWidget tool={clipped} defaultExpanded />,
    );
    expect(clippedMarkup).toContain("Preview clipped");
    expect(clippedMarkup).toContain("Additional lines");
    expect(clippedMarkup).toContain("Download");
  });

  it("bounds very large results while keeping enough data for the viewer fallback", () => {
    const output = "export const row = true;\n".repeat(24_000);
    const view = adaptEngineeringTool(
      tool("file_read", { action: "read", file: "src/large.ts" }, output),
    );
    expect(view?.statusLabel).toBe("Done · clipped");
    expect(view?.code?.length).toBeLessThanOrEqual(33_000);
    expect(view?.code).toContain("output truncated for this card");
    expect(view?.logLines.length).toBeLessThanOrEqual(120);
  });
});

describe("EngineeringToolWidget", () => {
  it("renders a compact authored receipt without generic INPUT or OUTPUT dumps", () => {
    const markup = renderToStaticMarkup(
      <EngineeringToolWidget
        tool={tool(
          "file_read",
          { action: "read", file: "src/app.ts" },
          "1\texport const app = true;",
        )}
      />,
    );

    expect(markup).toContain("Workspace read");
    expect(markup).toContain("Read app.ts");
    expect(markup).not.toContain(">INPUT<");
    expect(markup).not.toContain(">OUTPUT<");
    expect(markup).not.toContain("&quot;action&quot;");
  });

  it("uses live command events while running and synthetic history output after completion", () => {
    const timestamp = "2026-08-20T00:00:00.000Z";
    const liveEvent: CommandEvent = {
      event_id: "live-output",
      kind: "output_chunk",
      session_id: "session-1",
      run_id: "run-1",
      block_id: "block-1",
      sequence: 1,
      timestamp,
      chunk: {
        id: "chunk-1",
        session_id: "session-1",
        run_id: "run-1",
        block_id: "block-1",
        sequence: 1,
        stream: "stdout",
        text: "live-only evidence\n",
        received_at: timestamp,
      },
    };
    const syntheticEvent: CommandEvent = {
      ...liveEvent,
      event_id: "history-output",
      chunk: {
        ...liveEvent.chunk!,
        id: "history-chunk",
        text: "history-only evidence\n",
      },
    };
    const liveTool = {
      ...tool("shell", { action: "run", command: "npm test" }, "", "running"),
      commandEvents: [liveEvent],
    };

    expect(selectEngineeringConsoleEvents(liveTool, [syntheticEvent])).toEqual([
      liveEvent,
    ]);
    expect(
      selectEngineeringConsoleEvents(
        tool(
          "shell",
          { action: "run", command: "npm test" },
          "history-only evidence",
        ),
        [syntheticEvent],
      ),
    ).toEqual([syntheticEvent]);

    const liveMarkup = renderToStaticMarkup(
      <EngineeringToolWidget tool={liveTool} defaultExpanded />,
    );
    expect(liveMarkup).toContain("Running npm test");
    expect(liveMarkup).toContain("1 line");

    const historyMarkup = renderToStaticMarkup(
      <EngineeringToolWidget
        tool={tool(
          "shell",
          { action: "run", command: "npm test" },
          "history-only evidence",
        )}
      />,
    );
    expect(historyMarkup).toContain("Ran npm test");
    expect(historyMarkup).not.toContain("history-only evidence");
  });
});
