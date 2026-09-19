import type { ToolCall } from "../../lib/messageProcessor";

const operations: Record<string, readonly [string, string, string]> = {
  read: ["Reading output", "Read output", "Could not read output"],
  write: ["Sending input", "Sent input", "Could not send input"],
  status: ["Checking command status", "Checked command status", "Could not check command status"],
  kill: ["Requesting stop", "Stop requested", "Could not stop command"],
  start: ["Starting command", "Started command", "Could not start command"],
};

// A successful read/write/status request is not proof that its process exited.
// Only report a process outcome when the shell's receipt explicitly supplies it.
function processNote(result: unknown): string {
  if (!result || typeof result !== "object" || Array.isArray(result)) return "";
  const receipt = result as Record<string, unknown>;
  if (receipt.exited === false) return " · command still running";
  if (receipt.exited !== true) return "";
  const code = receipt.exit_code;
  return typeof code === "number" && Number.isInteger(code)
    ? ` · command exited with code ${code}`
    : " · command exited";
}

export function shellToolSummary(
  action: string,
  status: ToolCall["status"],
  command?: string,
  result?: unknown,
): string {
  if (status === "unknown") return "Completion not confirmed";
  const running = status === "pending" || status === "running";
  const failed = status === "error";
  const operation = Object.hasOwn(operations, action) ? operations[action] : undefined;
  if (operation) {
    const label = operation[failed ? 2 : running ? 0 : 1];
    const detail = action === "start" && command ? `: ${command}` : "";
    return label + detail + (!failed && !running ? processNote(result) : "");
  }
  if (failed) return command ? `Command failed: ${command}` : "Command failed";
  if (running) return command ? `Running ${command}` : "Command is running";
  return command ? `Ran ${command}` : "Command completed";
}
