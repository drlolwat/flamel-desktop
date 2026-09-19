export const skillBatchFiles = [
  {
    path: "/var/lib/flamel/home/.flamel/skill-projections/codex/8562040fbd41ae50/skills/flamel-ui-implementation/SKILL.md",
    language: "markdown",
    source:
      "# UI implementation\n\n- Keep the hierarchy compact.\n  - Preserve nested indentation.",
    totalLines: 4,
    resolvedRange: {
      startLine: 1,
      endLine: 4,
    },
  },
  {
    path: "/var/lib/flamel/home/.flamel/skill-projections/codex/8562040fbd41ae50/skills/flamel-frontend-engineering/SKILL.md",
    language: "markdown",
    source:
      "# Frontend engineering\n\nTrace component state, data flow, accessibility, and performance.",
    totalLines: 3,
    resolvedRange: {
      startLine: 1,
      endLine: 3,
    },
  },
  {
    path: "/var/lib/flamel/home/.flamel/skill-projections/codex/8562040fbd41ae50/skills/flamel-test-engineering/SKILL.md",
    language: "markdown",
    source:
      "# Test engineering\n\nLeave focused evidence that fails when the changed behavior regresses.",
    totalLines: 3,
    resolvedRange: {
      startLine: 1,
      endLine: 3,
    },
  },
  {
    path: "/var/lib/flamel/home/.flamel/skill-projections/codex/8562040fbd41ae50/skills/flamel-debugging/SKILL.md",
    language: "markdown",
    source:
      "# Debugging\n\nReproduce the failure, identify its root cause, and verify the repair.",
    totalLines: 3,
    resolvedRange: {
      startLine: 1,
      endLine: 3,
    },
  },
  {
    path: "/var/lib/flamel/home/.flamel/skill-projections/codex/8562040fbd41ae50/skills/flamel-release-verification/SKILL.md",
    language: "markdown",
    source:
      "# Release verification\n\nProve the release works across supported surfaces.",
    totalLines: 3,
    resolvedRange: {
      startLine: 1,
      endLine: 3,
    },
  },
  {
    path: "/var/lib/flamel/home/.flamel/skill-projections/codex/8562040fbd41ae50/skills/flamel-git-release-workflow/SKILL.md",
    language: "markdown",
    source:
      "# Git release workflow\n\nInspect status before branch, commit, pull request, or release mutations.",
    totalLines: 3,
    resolvedRange: {
      startLine: 1,
      endLine: 3,
    },
  },
];

export const skillBatchResult = {
  schema: "flamel.engineering_result.v1",
  tool: "file_read",
  action: "batch",
  status: "ok",
  summary: "6 files read",
  counts: {
    files: skillBatchFiles.length,
    lines: skillBatchFiles.reduce((total, file) => total + file.totalLines, 0),
  },
  truncated: false,
  warnings: [],
  data: { files: skillBatchFiles },
  text: "",
} as const;

export const legacySkillBatchText = skillBatchFiles
  .map(
    (file) =>
      `=== ${file.path} (${file.totalLines} lines) ===\n` +
      file.source
        .split("\n")
        .map((line, index) => `${String(index + 1).padStart(6)}\t${line}`)
        .join("\n"),
  )
  .join("\n\n");
