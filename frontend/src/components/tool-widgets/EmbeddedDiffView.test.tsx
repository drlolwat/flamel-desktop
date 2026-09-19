import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { languageForPath } from "../../lib/monacoPresentation";
import { EmbeddedCodeView } from "./EmbeddedCodeView";
import { EmbeddedDiffView } from "./EmbeddedDiffView";
import {
  MAX_MONACO_LINES,
  canUseMonaco,
  editorHeightFor,
  lightweightPreview,
  monacoModelUri,
  parseUnifiedDiff,
} from "./codePresentation";

describe("Monaco presentation helpers", () => {
  it("detects languages from paths and special filenames", () => {
    expect(languageForPath("frontend/src/App.tsx")).toBe("typescript");
    expect(languageForPath("infra/Dockerfile.dev")).toBe("dockerfile");
    expect(languageForPath("Makefile")).toBe("makefile");
    expect(languageForPath(".env.production")).toBe("ini");
    expect(languageForPath("notes.unknown")).toBe("plaintext");
  });

  it("parses multi-file unified patches with preamble, status, and stats", () => {
    const parsed = parseUnifiedDiff(`Applied cleanly
diff --git a/src/app.ts b/src/app.ts
--- a/src/app.ts
+++ b/src/app.ts
@@ -1 +1 @@
-const mode = "old";
+const mode = "new";
diff --git a/src/new.ts b/src/new.ts
new file mode 100644
--- /dev/null
+++ b/src/new.ts
@@ -0,0 +1,2 @@
+export const ready = true;
+export const count = 2;`);

    expect(parsed.preamble).toBe("Applied cleanly");
    expect(parsed.files).toHaveLength(2);
    expect(parsed.files[0]).toMatchObject({
      path: "src/app.ts",
      additions: 1,
      deletions: 1,
      status: "modified",
      language: "typescript",
    });
    expect(parsed.files[1]).toMatchObject({
      path: "src/new.ts",
      additions: 2,
      deletions: 0,
      status: "added",
    });
  });

  it("keeps deleted-file identity and falls back for non-standard patch text", () => {
    const deleted = parseUnifiedDiff(`--- a/src/old.ts
+++ /dev/null
@@ -1 +0,0 @@
-export const old = true;`);

    expect(deleted.files[0]).toMatchObject({
      path: "src/old.ts",
      status: "deleted",
      deletions: 1,
    });

    const fallback = parseUnifiedDiff("@@ -1 +1 @@\n-old\n+new");
    expect(fallback.files[0].path).toBe("changes.diff");
    expect(fallback.files[0].patch).toContain("+new");
  });

  it("caps editor height and rejects oversized Monaco models", () => {
    expect(editorHeightFor(["one"], 170, 520)).toBe(170);
    expect(editorHeightFor(["line\n".repeat(100)], 170, 300)).toBe(300);
    expect(canUseMonaco(["small"])).toBe(true);
    expect(canUseMonaco(["x\n".repeat(MAX_MONACO_LINES + 1)])).toBe(false);
  });

  it("builds isolated model URIs and bounded lightweight previews", () => {
    expect(monacoModelUri("card-a", "original", "src/a file.ts")).toBe(
      "inmemory://flamel/tool-widget/card-a/original/src%2Fa%20file.ts",
    );
    expect(monacoModelUri("card-b", "original", "src/a file.ts")).not.toBe(
      monacoModelUri("card-a", "original", "src/a file.ts"),
    );

    const preview = lightweightPreview("line\n".repeat(400));
    expect(preview.truncated).toBe(true);
    expect(preview.text.split("\n").length).toBeLessThanOrEqual(261);
  });
});

describe("embedded code surfaces", () => {
  const oversizedOriginal = "old\n".repeat(MAX_MONACO_LINES + 1);

  it("renders purposeful multi-file controls without mounting Monaco for a large fallback", () => {
    const markup = renderToStaticMarkup(
      <EmbeddedDiffView
        files={[
          {
            path: "src/large.ts",
            original: oversizedOriginal,
            modified: "export const ready = true;\n",
            additions: 1,
            deletions: MAX_MONACO_LINES + 1,
          },
          {
            path: "src/new.ts",
            original: "",
            modified: "export const created = true;\n",
            additions: 1,
            deletions: 0,
            status: "added",
          },
        ]}
        onOpenInIde={() => undefined}
      />,
    );

    expect(markup).toContain('role="tablist"');
    expect(markup).toContain("large.ts");
    expect(markup).toContain("new.ts");
    expect(markup).toContain("Split");
    expect(markup).toContain("Inline");
    expect(markup).toContain("Open in IDE");
    expect(markup).toContain("Large diff for src/large.ts preview");
    expect(markup).not.toContain("monaco-editor");
  });

  it("uses a bounded readable fallback for oversized code", () => {
    const markup = renderToStaticMarkup(
      <EmbeddedCodeView path="src/large.ts" value={oversizedOriginal} />,
    );

    expect(markup).toContain("Large code view for src/large.ts preview");
    expect(markup).toContain("open in the IDE for the complete file");
    expect(markup.length).toBeLessThan(80_000);
  });
});
