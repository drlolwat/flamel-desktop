import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { normalizeToolPresentation, type ToolCall } from "../../lib/messageProcessor";
import { adaptEngineeringTool } from "./engineeringToolAdapters";
import { EngineeringToolWidget } from "./EngineeringToolWidget";

function read(input: Record<string, unknown>, source: string, data: Record<string, unknown> = {}): ToolCall {
  const result = {
    schema: "flamel.engineering_result.v1", tool: "file_read",
    action: input.action ?? "read", status: "ok", summary: source.split("\n")[0],
    data, counts: {}, truncated: false, warnings: [], text: source,
  };
  return {
    id: "workspace-read", name: "mcp__flamel__file_read", status: "done", input, output: source,
    presentation: normalizeToolPresentation({ content: {
      content: [{ type: "text", text: source }], structuredContent: result,
    } }),
  };
}

describe("collapsed workspace filenames", () => {
  it.each([
    [{ file: "ci/verify-windows.ps1" }, "121 Assert-ByteRange -Bytes $Bytes", "Read verify-windows.ps1"],
    [{ relative_path: "ci/build.sh" }, "1 #!/usr/bin/env bash", "Read build.sh"],
    [{ file_path: "C:\\work\\src\\app.ts" }, "1 const password = 'never-in-the-card'", "Read app.ts"],
    [{ file: "deploy/.env" }, "1 DATABASE_PASSWORD=do-not-show", "Read .env"],
  ])("uses explicit request metadata instead of the first output line", (input, source, expected) => {
    const tool = read(input, source, { files: [{ path: "wrong-output-path.txt", source }] });
    expect(adaptEngineeringTool(tool)?.summary).toBe(expected);
    const markup = renderToStaticMarkup(<EngineeringToolWidget tool={tool} />);
    expect(markup).toContain(expected);
    expect(markup).not.toContain(source);
    expect(markup).not.toContain("wrong-output-path.txt");
  });

  it("uses structured path metadata when a restored call has no input path", () => {
    expect(adaptEngineeringTool(read({}, "1 #!/bin/sh", {
      files: [{ path: "install/restore.sh", source: "#!/bin/sh" }],
    }))?.summary).toBe("Read restore.sh");
    expect(adaptEngineeringTool(read({}, "1 SECRET=never-a-label"))?.summary).toBe("Read workspace");
  });

  it("summarizes string and range-object batches, with distinct short paths", () => {
    const files = ["/home/person/skills/ui/SKILL.md", { path: "/home/person/skills/git/SKILL.md", start_line: 1 }, "src/app.ts"];
    for (const value of [files, JSON.stringify(files)]) {
      const tool = read({ action: "batch", files: value }, "1 # hidden source");
      expect(adaptEngineeringTool(tool)?.summary).toBe("Read 3 files · ui/SKILL.md, git/SKILL.md +1");
      expect(adaptEngineeringTool({ ...tool, status: "running" })?.summary).toBe("Reading 3 files · ui/SKILL.md, git/SKILL.md +1");
    }
  });

  it("gets legacy batch filenames from structured metadata, not summary/source fields", () => {
    expect(adaptEngineeringTool(read({ action: "batch" }, "1 token=hidden", {
      files: [{ path: "a.ts", source: "token=hidden" }, { path: "b.ts", source: "other-secret" }],
    }))?.summary).toBe("Read 2 files · a.ts, b.ts");
  });

  it("retains the full batch count when result metadata was clipped", () => {
    const tool = read({ action: "batch" }, "hidden source", { files: [{ path: "a.ts" }, { path: "b.ts" }] });
    const result = tool.presentation!.structuredContent as Record<string, unknown>;
    result.counts = { files: 12 };
    result.truncated = true;
    expect(adaptEngineeringTool(tool)?.summary).toBe("Read 12 files · a.ts, b.ts +10");
  });

  it("counts distinct edited files, not replacements, and never previews replacement text", () => {
    const tool: ToolCall = {
      id: "edits", name: "file_edit", status: "done", input: {
        action: "multi_replace", edits: [
          { file: "src/app.ts", old_string: "secret-one", new_string: "secret-two" },
          { relative_path: "src/app.ts", old_string: "secret-three", new_string: "secret-four" },
          { path: "src/api.ts", old_string: "secret-five", new_string: "secret-six" },
        ],
      }, output: "secret-output",
    };
    expect(adaptEngineeringTool(tool)?.summary).toBe("Updated 2 files · app.ts, api.ts");
    expect(renderToStaticMarkup(<EngineeringToolWidget tool={tool} />)).not.toContain("secret-");
  });

  it("keeps create/delete/preview labels and uncertain/error state honest", () => {
    for (const [action, verb] of [["create", "Created"], ["delete", "Deleted"], ["preview", "Previewed"]]) {
      const tool: ToolCall = { id: action, name: "file_edit", status: "done", input: { action, relative_path: "src/app.ts", content: "private source" } };
      expect(adaptEngineeringTool(tool)?.summary).toBe(`${verb} app.ts`);
      expect(adaptEngineeringTool({ ...tool, status: "unknown" })?.summary).toBe("Completion not confirmed");
      expect(adaptEngineeringTool({ ...tool, status: "error" })?.summary).toBe("Workspace change failed");
    }
  });

  it("preserves failed structured reads and bounded discovery counts", () => {
    const tool = read({ action: "glob", pattern: "*.ts" }, "src/a.ts", { paths: [{ path: "src/a.ts" }, { path: "src/b.ts" }] });
    expect(adaptEngineeringTool(tool)?.summary).toBe("2 results found");
    const result = tool.presentation!.structuredContent as Record<string, unknown>;
    result.counts = { paths: 123 };
    expect(adaptEngineeringTool(tool)?.summary).toBe("123 results found");
    result.status = "error";
    expect(adaptEngineeringTool(tool)).toMatchObject({ summary: "Workspace read failed", status: "error" });
  });

  it("does not parse source/diff strings as paths and keeps long batches compact", () => {
    expect(adaptEngineeringTool(read({ content: '{"file":"secret-from-content"}' }, "secret-output"))?.summary).toBe("Read workspace");
    const tool = read({ action: "batch", files: Array.from({ length: 10 }, (_, i) => `${"long-folder/".repeat(50)}file-${i}.ts`) }, "hidden");
    const summary = adaptEngineeringTool(tool)!.summary;
    expect(summary).toBe("Read 10 files · file-0.ts, file-1.ts +8");
    expect(summary.length).toBeLessThan(170);
  });
});
