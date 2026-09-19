import type { editor } from "monaco-editor";

const LANGUAGE_BY_EXTENSION: Record<string, string> = {
  bash: "shell",
  c: "c",
  cc: "cpp",
  conf: "ini",
  cpp: "cpp",
  cs: "csharp",
  css: "css",
  dart: "dart",
  dockerfile: "dockerfile",
  go: "go",
  graphql: "graphql",
  h: "c",
  hpp: "cpp",
  html: "html",
  ini: "ini",
  java: "java",
  js: "javascript",
  json: "json",
  jsx: "javascript",
  kt: "kotlin",
  kts: "kotlin",
  less: "less",
  lua: "lua",
  md: "markdown",
  mjs: "javascript",
  php: "php",
  prisma: "graphql",
  py: "python",
  rb: "ruby",
  rs: "rust",
  scss: "scss",
  sh: "shell",
  sql: "sql",
  swift: "swift",
  toml: "ini",
  ts: "typescript",
  tsx: "typescript",
  vue: "html",
  xml: "xml",
  yaml: "yaml",
  yml: "yaml",
  zsh: "shell",
};

export const FLAMEL_MONACO_THEME = "flamel-tool-widget";

export const READ_ONLY_MONACO_OPTIONS = {
  readOnly: true,
  domReadOnly: true,
  automaticLayout: true,
  scrollBeyondLastLine: false,
  fontFamily: "var(--font-mono)",
  fontSize: 12,
  lineHeight: 19,
  wordWrap: "off",
  padding: { top: 8, bottom: 12 },
} satisfies editor.IStandaloneEditorConstructionOptions;

export function languageForPath(path: string): string {
  const filename = path.split("/").pop()?.toLowerCase() || "";
  if (filename === "dockerfile" || filename.startsWith("dockerfile.")) return "dockerfile";
  if (filename === "makefile" || filename.startsWith("makefile.")) return "makefile";
  if (filename === ".env" || filename.startsWith(".env.")) return "ini";

  const ext = filename.includes(".") ? filename.split(".").pop() || "" : "";
  return LANGUAGE_BY_EXTENSION[ext] || "plaintext";
}

export function registerFlamelMonacoTheme(
  monacoApi: typeof import("monaco-editor"),
): void {
  if (!monacoApi.languages.getLanguages().some((entry) => entry.id === "diff")) {
    monacoApi.languages.register({
      id: "diff",
      extensions: [".diff", ".patch"],
      aliases: ["Diff", "Patch"],
    });
    monacoApi.languages.setMonarchTokensProvider("diff", {
      tokenizer: {
        root: [
          [/^diff --git.*$/, "diff.header"],
          [/^(?:---|\+\+\+).*$/, "diff.header"],
          [/^@@.*@@.*$/, "diff.hunk"],
          [/^\+.*/, "diff.inserted"],
          [/^-.*/, "diff.removed"],
          [/^\\ No newline at end of file$/, "comment"],
        ],
      },
    });
  }

  monacoApi.editor.defineTheme(FLAMEL_MONACO_THEME, {
    base: "vs-dark",
    inherit: true,
    rules: [
      { token: "comment", foreground: "706B67" },
      { token: "keyword", foreground: "E56A55" },
      { token: "string", foreground: "C9A77B" },
      { token: "number", foreground: "D4B47C" },
      { token: "type", foreground: "B9B2AA" },
      { token: "diff.header", foreground: "938A82" },
      { token: "diff.hunk", foreground: "A7A09A" },
      { token: "diff.inserted", foreground: "7FBE8D" },
      { token: "diff.removed", foreground: "E27A72" },
    ],
    colors: {
      "editor.background": "#121110",
      "editor.foreground": "#D7D2CC",
      "editorLineNumber.foreground": "#5E5955",
      "editorLineNumber.activeForeground": "#A49D96",
      "editor.selectionBackground": "#55302788",
      "editor.inactiveSelectionBackground": "#3C2A2670",
      "editor.lineHighlightBackground": "#1A181700",
      "editorCursor.foreground": "#D64232",
      "editorWhitespace.foreground": "#292624",
      "editorIndentGuide.background1": "#292624",
      "editorIndentGuide.activeBackground1": "#4B4540",
      "editorGutter.background": "#121110",
      "diffEditor.insertedTextBackground": "#224B354D",
      "diffEditor.removedTextBackground": "#5B2B2B4D",
      "diffEditor.insertedLineBackground": "#17372442",
      "diffEditor.removedLineBackground": "#47212142",
      "diffEditor.diagonalFill": "#1A1817",
      "scrollbarSlider.background": "#5A534D55",
      "scrollbarSlider.hoverBackground": "#6C635C77",
      "scrollbarSlider.activeBackground": "#82766D88",
    },
  });
}
