import { describe, expect, it } from "vitest";
import { shellToolSummary } from "./shellToolSummary";

describe("shell request versus process completion", () => {
  it("does not call a successful output read a completed command", () => {
    expect(shellToolSummary("read", "done", undefined, { exited: false, exit_code: null }))
      .toBe("Read output · command still running");
    expect(shellToolSummary("read", "done")).toBe("Read output");
  });

  it("uses only explicit exit evidence, including failures and signal exits", () => {
    expect(shellToolSummary("read", "done", undefined, { exited: true, exit_code: 0 }))
      .toBe("Read output · command exited with code 0");
    expect(shellToolSummary("read", "done", undefined, { exited: true, exit_code: 1 }))
      .toBe("Read output · command exited with code 1");
    expect(shellToolSummary("read", "done", undefined, { exited: true, exit_code: null }))
      .toBe("Read output · command exited");
    for (const result of [{ output: "Command completed" }, { exited: "true", exit_code: 0 }, { exit_code: 0 }]) {
      expect(shellToolSummary("read", "done", undefined, result)).toBe("Read output");
    }
  });

  it("does not treat successful control calls or process launch as process completion", () => {
    expect(shellToolSummary("write", "done")).toBe("Sent input");
    expect(shellToolSummary("status", "done")).toBe("Checked command status");
    expect(shellToolSummary("kill", "done")).toBe("Stop requested");
    expect(shellToolSummary("start", "done", "npm run dev")).toBe("Started command: npm run dev");
  });

  it("keeps read progress, failures and unknown delivery distinct from exit status", () => {
    expect(shellToolSummary("read", "running")).toBe("Reading output");
    expect(shellToolSummary("read", "pending")).toBe("Reading output");
    expect(shellToolSummary("read", "error", undefined, { exited: false })).toBe("Could not read output");
    expect(shellToolSummary("read", "unknown")).toBe("Completion not confirmed");
  });

  it("retains the actual synchronous command lifecycle", () => {
    expect(shellToolSummary("run", "running", "npm test")).toBe("Running npm test");
    expect(shellToolSummary("run", "done", "npm test")).toBe("Ran npm test");
    expect(shellToolSummary("run", "error", "npm test")).toBe("Command failed: npm test");
    expect(shellToolSummary("run", "unknown", "npm test")).toBe("Completion not confirmed");
    expect(shellToolSummary("constructor", "error")).toBe("Command failed");
  });
});
