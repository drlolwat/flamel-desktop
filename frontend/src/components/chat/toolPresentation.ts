function inferCodexFileEditAction(input?: Record<string, unknown>): string {
  const explicitAction = input?.action;
  if (typeof explicitAction === "string" && explicitAction.trim()) {
    return explicitAction;
  }

  const changes = input?.changes;
  if (Array.isArray(changes) && changes.length > 0) {
    if (changes.length > 1) return "multi_replace";
    const firstChange = changes[0];
    const change =
      firstChange && typeof firstChange === "object"
        ? (firstChange as Record<string, unknown>)
        : undefined;
    const operation =
      typeof change?.type === "string"
        ? change.type
        : typeof change?.action === "string"
          ? change.action
          : typeof change?.operation === "string"
            ? change.operation
            : "";
    const normalized = operation.toLowerCase();
    if (["create", "add", "write"].includes(normalized)) return "create";
    if (["delete", "remove"].includes(normalized)) return "delete";
    if (["rename", "move"].includes(normalized)) return "move";
    return "replace";
  }

  if (typeof input?.content === "string") return "create";
  if (
    typeof input?.old_string === "string" ||
    typeof input?.new_string === "string"
  ) {
    return "replace";
  }
  return "";
}

// Parse mcp__{server}__{action} names into friendly display parts
export function parseToolName(
  name: string,
  actionParam?: string,
  input?: Record<string, unknown>,
): {
  server: string | null;
  action: string;
  label: string;
  isReclaude: boolean;
} {
  // Codex emits a few native item types with bare names. Render them like the
  // Claude/MCP rows without changing the stored or executed tool payload.
  if (name === "shell") {
    return {
      server: null,
      action: name,
      label: "shell · run",
      isReclaude: true,
    };
  }
  if (name === "file_edit") {
    const action = (actionParam || inferCodexFileEditAction(input)).replace(
      /_/g,
      " ",
    );
    return {
      server: null,
      action: name,
      label: action ? `file edit · ${action}` : "file edit",
      isReclaude: true,
    };
  }

  // Built-in Rust MCP servers — show action only in accent color, no server prefix
  const BUILTIN_MCP_SERVERS = new Set([
    "flamel",
    "git",
    "docs",
    "db",
    "browser",
  ]);

  // flamel_* tools (no mcp__ prefix) — drop the "flamel" word; the accent
  // color already signals these are flamel-native.
  if (name.startsWith("flamel_")) {
    const action = name.slice("flamel_".length);
    return {
      server: null,
      action,
      label: action.replace(/_/g, " "),
      isReclaude: true,
    };
  }
  const parts = name.split("__");
  if (parts[0] === "mcp" && parts.length >= 3) {
    const server = parts[1];
    const action = parts.slice(2).join("__");
    // Built-in MCP tools — show action only in accent, drop the server name.
    // Keeps the label short: "replace content", "find symbol", "git status".
    // Accent styling already communicates "this is a flamel/builtin tool".
    if (BUILTIN_MCP_SERVERS.has(server)) {
      // Strip redundant server prefix from action (e.g. flamel_replace → replace when server is flamel)
      const cleanAction = action.startsWith(`${server}_`)
        ? action.slice(server.length + 1)
        : action;
      // Append action param for consolidated tools (e.g. "file edit · replace", "shell · exec")
      const label = actionParam
        ? `${cleanAction.replace(/_/g, " ")} · ${actionParam.replace(/_/g, " ")}`
        : cleanAction.replace(/_/g, " ");
      return { server: null, action: cleanAction, label, isReclaude: true };
    }
    // External MCP servers — show server · action
    const serverLabel = server
      .replace(/_/g, " ")
      .replace(/\b\w/g, (c) => c.toUpperCase());
    return {
      server: serverLabel,
      action,
      label: action.replace(/_/g, " "),
      isReclaude: false,
    };
  }
  // Built-in tools: friendly name
  const builtins: Record<string, string> = {
    Read: "Read",
    Edit: "Edit",
    Write: "Write",
    Bash: "Bash",
    Glob: "Glob",
    Grep: "Grep",
    ToolSearch: "Tool Search",
    WebFetch: "Web Fetch",
    WebSearch: "Web Search",
  };
  return {
    server: null,
    action: name,
    label: builtins[name] || name,
    isReclaude: false,
  };
}
