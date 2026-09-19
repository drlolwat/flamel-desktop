import type { ToolCall } from "../../lib/messageProcessor";

type Metadata = Record<string, unknown>;
const PATH_KEYS = ["file", "path", "relative_path", "file_path"];

function record(value: unknown): Metadata | undefined {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as Metadata)
    : undefined;
}

function pathValue(value: unknown): string | undefined {
  // Only explicit path fields are labels. Never inspect source, patches, or output text.
  if (typeof value !== "string" || /[\u0000-\u001f\u007f]/.test(value))
    return undefined;
  return value.trim().replace(/\\/g, "/") || undefined;
}

function pathFrom(value: unknown): string | undefined {
  const metadata = record(value);
  for (const key of PATH_KEYS) {
    const path = pathValue(metadata?.[key]);
    if (path) return path;
  }
  return undefined;
}

function array(value: unknown): unknown[] {
  // Some persisted MCP calls keep a JSON-encoded array.
  if (typeof value === "string") {
    try {
      value = JSON.parse(value);
    } catch {
      return [];
    }
  }
  return Array.isArray(value) ? value : [];
}

function pathsFrom(metadata: Metadata): string[] {
  return [...new Set([
    pathFrom(metadata),
    ...array(metadata.files).map((file) => pathValue(file) ?? pathFrom(file)),
    ...array(metadata.edits).map(pathFrom),
  ].filter((path): path is string => Boolean(path)))];
}

function shortPaths(paths: string[], total = paths.length): string {
  const parts = paths.map((path) => path.split("/").filter(Boolean));
  return parts.slice(0, 2).map((segments, index) => {
    let depth = 1;
    // Distinguish e.g. two SKILL.md files without showing their entire home path.
    while (depth < segments.length && parts.some((other, otherIndex) =>
      otherIndex !== index && other.slice(-depth).join("/") === segments.slice(-depth).join("/"),
    )) depth++;
    const label = segments.slice(-depth).join("/") || "/";
    return label.length > 64 ? `…${label.slice(-63)}` : label;
  }).join(", ") + (paths.length && total > 2 ? ` +${total - Math.min(paths.length, 2)}` : "");
}

export function workspaceToolSummary({
  input, action, editing, status, resultData, resultCounts, resultError, entryCount,
}: {
  input: Metadata;
  action: string;
  editing: boolean;
  status: ToolCall["status"];
  resultData?: Metadata;
  resultCounts?: Record<string, number>;
  resultError?: boolean;
  entryCount: number;
}): string {
  if (status === "unknown") return "Completion not confirmed";
  const running = status === "pending" || status === "running";
  if (status === "error" || resultError) {
    return `${editing ? "Workspace change" : "Workspace read"} failed`;
  }

  // Request metadata wins. Resolved result metadata helps restored/legacy calls
  // whose request was not retained, without promoting arbitrary output to a label.
  const requestedPaths = pathsFrom(input);
  const paths = requestedPaths.length ? requestedPaths : pathsFrom(resultData ?? {});
  const count = requestedPaths.length || resultCounts?.files || paths.length;
  const label = shortPaths(paths, count);
  if (!editing && ["glob", "find", "list_dir"].includes(action)) {
    if (running) return `Scanning ${pathFrom(input) ?? pathValue(resultData?.root) ?? "workspace"}`;
    const count = resultCounts?.paths ?? (array(resultData?.paths).length || entryCount);
    return count ? `${count} result${count === 1 ? "" : "s"} found` : "No matching files";
  }

  const verbs: Record<string, [string, string]> = {
    create: ["Creating", "Created"],
    delete: ["Deleting", "Deleted"],
    format: ["Formatting", "Formatted"],
    preview: ["Previewing", "Previewed"],
  };
  const [active, done] = editing ? verbs[action] ?? ["Updating", "Updated"] : ["Reading", "Read"];
  const verb = running ? active : done;
  if (action === "batch" || action === "multi_replace" || paths.length > 1) {
    return `${verb} ${count || "multiple"} file${count === 1 ? "" : "s"}${label ? ` · ${label}` : ""}`;
  }
  return `${verb} ${label || "workspace"}`;
}
