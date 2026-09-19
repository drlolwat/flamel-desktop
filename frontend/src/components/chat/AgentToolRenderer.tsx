import React, { useState } from "react";
import type { ToolCall } from "../../lib/messageProcessor";
import { InteractiveConsole } from "../console/InteractiveConsole";
import { nestedCommandEventsToCommandEvents } from "../console/consoleAdapters";
import { Icon } from "../ui/Icons";
import {
  buildNestedAgentRunTree,
  type NestedAgentRunNode,
} from "./agentRunTree";
import { parseToolName } from "./toolPresentation";

export const AgentRow = React.memo(function AgentRow({
  tool,
}: {
  tool: ToolCall;
}) {
  const [expanded, setExpanded] = useState(
    tool.status === "running" || tool.status === "pending",
  );
  const isRunning = tool.status === "running" || tool.status === "pending";
  const isError = tool.status === "error";

  // Input may still be streaming — fall back to partialInput for the subtype/desc
  // so the row isn't rendered blank while the JSON is building up.
  let rawSubtype = (tool.input?.subagent_type || "") as string;
  let rawDesc = (tool.input?.description || "") as string;
  if ((!rawSubtype || !rawDesc) && tool.partialInput) {
    const m = /"subagent_type"\s*:\s*"([^"]*)"/.exec(tool.partialInput);
    if (m && !rawSubtype) rawSubtype = m[1];
    const d = /"description"\s*:\s*"([^"]*)"/.exec(tool.partialInput);
    if (d && !rawDesc) rawDesc = d[1];
  }
  // Always show a badge — default to "agent" if subagent_type never arrived.
  const subtype = (rawSubtype || "agent").replace(/-/g, " ");
  const desc = rawDesc || "(no description)";
  const label = desc.length > 120 ? desc.slice(0, 117) + "…" : desc;
  const hasOutput = !!tool.output;
  const hasNested = !!(tool.nestedEvents && tool.nestedEvents.length > 0);
  // Always expandable — user should be able to peek inside at any time,
  // even while the agent is still running with no nested events yet.
  const expandable = true;

  return (
    <div
      style={{
        margin: "3px 0 3px 8px",
        borderRadius: 10,
        background: "var(--surface-2)",
        border: "1px solid var(--border-soft)",
        overflow: "hidden",
      }}
    >
      <div
        className="flex items-center"
        style={{
          gap: 6,
          padding: "5px 10px",
          cursor: expandable ? "pointer" : "default",
        }}
        onClick={() => expandable && setExpanded((e) => !e)}
        onMouseEnter={(e) => {
          if (expandable)
            e.currentTarget.style.background = "rgba(255,255,255,0.03)";
        }}
        onMouseLeave={(e) => {
          e.currentTarget.style.background = "transparent";
        }}
      >
        {isRunning ? (
          <span
            className="spinner"
            style={{
              width: 8,
              height: 8,
              border: "1.5px solid var(--accent)",
              borderTopColor: "transparent",
              borderRadius: "50%",
              display: "inline-block",
              flexShrink: 0,
            }}
          />
        ) : isError ? (
          <span
            style={{
              color: "var(--error)",
              fontSize: 11,
              flexShrink: 0,
              lineHeight: 1,
            }}
          >
            ✕
          </span>
        ) : (
          <span
            style={{
              color: "var(--success)",
              fontSize: 11,
              flexShrink: 0,
              lineHeight: 1,
            }}
          >
            ✓
          </span>
        )}

        {subtype && (
          <span
            style={{
              fontSize: 10,
              fontWeight: 600,
              padding: "1px 6px",
              borderRadius: 99,
              flexShrink: 0,
              background: "var(--accent-glow)",
              color: "var(--accent)",
              border: "1px solid var(--accent-border)",
              letterSpacing: "0.02em",
            }}
          >
            {subtype}
          </span>
        )}

        {label && (
          <span
            style={{
              fontSize: 11,
              color: "var(--text-muted)",
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
              flex: 1,
            }}
          >
            {label}
          </span>
        )}

        {expandable && (
          <span
            style={{
              color: "var(--text-dim)",
              display: "flex",
              flexShrink: 0,
              opacity: 0.5,
              transition: "transform 0.15s",
              transform: expanded ? "rotate(180deg)" : "none",
            }}
          >
            <Icon.ChevronDown size={10} />
          </span>
        )}
      </div>

      {expanded && expandable && (
        <div style={{ borderTop: "1px solid var(--border-soft)" }}>
          {tool.nestedEvents && tool.nestedEvents.length > 0 ? (
            <NestedAgentEventList events={tool.nestedEvents} />
          ) : isRunning && !tool.output ? (
            <div
              style={{
                padding: "8px 12px",
                fontSize: 11,
                color: "var(--text-dim)",
                fontStyle: "italic",
              }}
            >
              Agent starting up…
            </div>
          ) : null}
          {tool.output &&
            (() => {
              const raw = tool.output;
              const backendTruncated = raw.includes(
                "[... sub-agent output truncated at 50KB ...]",
              );
              const DISPLAY_CAP = 8000;
              const displayed =
                raw.length > DISPLAY_CAP ? raw.slice(0, DISPLAY_CAP) : raw;
              return (
                <>
                  {tool.nestedEvents && tool.nestedEvents.length > 0 && (
                    <div
                      style={{
                        padding: "3px 12px",
                        fontSize: 10,
                        color: "var(--text-dim)",
                        borderTop: "1px dashed var(--border-soft)",
                        letterSpacing: "0.03em",
                        textTransform: "uppercase",
                      }}
                    >
                      Final output
                    </div>
                  )}
                  <pre
                    style={{
                      margin: 0,
                      padding: "6px 12px 8px",
                      fontSize: 11,
                      color: "var(--text-muted)",
                      fontFamily: "var(--font-mono)",
                      maxHeight: 360,
                      overflowY: "auto",
                      whiteSpace: "pre-wrap",
                      wordBreak: "break-word",
                    }}
                  >
                    {displayed}
                  </pre>
                  {backendTruncated && (
                    <div
                      style={{
                        padding: "4px 12px 6px",
                        fontSize: 10,
                        color: "var(--warning, #f59e0b)",
                        borderTop: "1px dashed var(--border-soft)",
                        fontStyle: "italic",
                      }}
                    >
                      Sub-agent output was capped at 50 KB by the runner.
                    </div>
                  )}
                </>
              );
            })()}
        </div>
      )}
    </div>
  );
});

/** Batch of sub-agents rendered as a grouped card with per-agent pills and a progress bar. */
export const AgentBatchRow = React.memo(function AgentBatchRow({
  tool,
}: {
  tool: ToolCall;
}) {
  const [expanded, setExpanded] = useState(true); // batches start expanded
  const toolIsRunning = tool.status === "running" || tool.status === "pending";
  const toolIsError = tool.status === "error";

  // Parse agent specs from input (may still be streaming via partialInput)
  let batchDesc = (tool.input?.description || "") as string;
  let agentSpecs: Array<{ description: string; subagent_type: string }> = [];
  if (Array.isArray(tool.input?.agents)) {
    agentSpecs = (tool.input.agents as any[]).map((a: any) => ({
      description: a.description || "(unnamed)",
      subagent_type: (a.subagent_type || "agent").replace(/-/g, " "),
    }));
  } else if (tool.partialInput) {
    // Try to extract from partial JSON while streaming
    const dm = /"description"\s*:\s*"([^"]*)"/.exec(tool.partialInput);
    if (dm && !batchDesc) batchDesc = dm[1];
  }

  // Parse batch results from output (after completion)
  type SubAgentResult = {
    index: number;
    description: string;
    status: string;
    duration: string;
    output: string;
    error?: string;
  };
  let subResults: SubAgentResult[] = [];
  let batchRunIds: string[] = [];
  let conflicts: string[] = [];
  let reviewPrompt = "";

  if (tool.output) {
    const lines = tool.output.split("\n");
    // Parse AGENT_BATCH:{id} and AGENT_RUNS:{ids} headers
    for (const line of lines) {
      if (line.startsWith("AGENT_RUNS:")) {
        batchRunIds = line
          .slice("AGENT_RUNS:".length)
          .split(",")
          .filter(Boolean);
      }
    }

    // Parse agent sections: --- Agent N: desc [status] (duration) ---
    const agentSectionRe = /^--- Agent (\d+): (.+?) \[(\w+)\] \(([^)]+)\) ---$/;
    let currentAgent: SubAgentResult | null = null;
    let inConflicts = false;
    let inReview = false;

    for (const line of lines) {
      if (
        line.startsWith("AGENT_BATCH:") ||
        line.startsWith("AGENT_RUNS:") ||
        line.startsWith("=== AGENT BATCH RESULTS ===")
      )
        continue;
      if (line.startsWith("Batch:") || line.startsWith("Completed:")) continue;

      const m = agentSectionRe.exec(line);
      if (m) {
        if (currentAgent) subResults.push(currentAgent);
        currentAgent = {
          index: parseInt(m[1]) - 1,
          description: m[2],
          status: m[3],
          duration: m[4],
          output: "",
          error: undefined,
        };
        inConflicts = false;
        inReview = false;
        continue;
      }

      if (line === "=== FILE CONFLICTS ===") {
        if (currentAgent) {
          subResults.push(currentAgent);
          currentAgent = null;
        }
        inConflicts = true;
        inReview = false;
        continue;
      }
      if (line === "=== REVIEW PROMPT ===") {
        if (currentAgent) {
          subResults.push(currentAgent);
          currentAgent = null;
        }
        inConflicts = false;
        inReview = true;
        continue;
      }

      if (inConflicts && line.trim()) {
        conflicts.push(line);
      } else if (inReview) {
        reviewPrompt += line + "\n";
      } else if (currentAgent) {
        if (line.startsWith("ERROR: ")) {
          currentAgent.error = line.slice(7);
        } else {
          currentAgent.output += line + "\n";
        }
      }
    }
    if (currentAgent) subResults.push(currentAgent);
  }

  // Live progress from nested events (batch_start + batch_agent_done)
  // These stream in while the batch is running, before output exists.
  type LiveAgent = {
    index: number;
    run_id?: string;
    description: string;
    subagent_type: string;
    status: string;
    duration_ms?: number;
    error?: string;
    events: Array<Record<string, unknown>>;
  };
  let liveAgents: LiveAgent[] = [];
  let liveDoneCount = 0;
  let liveFailedCount = 0;
  if (tool.nestedEvents) {
    const parentByRunId = new Map<string, string>();
    for (const ev of tool.nestedEvents) {
      if (
        typeof ev.run_id === "string" &&
        typeof ev.parent_run_id === "string"
      ) {
        parentByRunId.set(ev.run_id, ev.parent_run_id);
      }
    }
    for (const ev of tool.nestedEvents) {
      const evType = ev.type as string;
      if (evType === "batch_start" && Array.isArray(ev.agents)) {
        liveAgents = (ev.agents as any[]).map((a: any) => ({
          index: a.index ?? 0,
          run_id: typeof a.run_id === "string" ? a.run_id : undefined,
          description: a.description || "(unnamed)",
          subagent_type: (a.subagent_type || "agent").replace(/-/g, " "),
          status: "running",
          events: [],
        }));
      }
      if (evType === "batch_agent_done" && typeof ev.agent_index === "number") {
        const idx = ev.agent_index as number;
        const agent = liveAgents[idx];
        if (agent) {
          agent.status = (ev.status as string) || "done";
          agent.duration_ms = (ev.duration_ms as number) || undefined;
          if (typeof ev.run_id === "string" && ev.run_id)
            agent.run_id = ev.run_id;
        }
      }
    }
    for (const ev of tool.nestedEvents) {
      const rid = ev.run_id;
      if (typeof rid !== "string" || rid === tool.runId) continue;
      let ownerRunId = rid;
      const visited = new Set<string>();
      while (!liveAgents.some((agent) => agent.run_id === ownerRunId)) {
        if (visited.has(ownerRunId)) break;
        visited.add(ownerRunId);
        const parentRunId = parentByRunId.get(ownerRunId);
        if (!parentRunId) break;
        ownerRunId = parentRunId;
      }
      const agent = liveAgents.find((a) => a.run_id === ownerRunId);
      if (!agent) continue;
      agent.events.push(ev);
      if (ev.type === "nested_done" && agent.status === "running") {
        agent.status = "success";
      } else if (ev.type === "nested_error") {
        agent.status = "failed";
        agent.error = (ev.error as string | undefined) || agent.error;
      }
    }
    liveDoneCount = liveAgents.filter(
      (a) =>
        a.status === "success" ||
        a.status === "done" ||
        a.status === "completed",
    ).length;
    liveFailedCount = liveAgents.filter(
      (a) => a.status === "failed" || a.status === "timeout",
    ).length;
  }

  // Build display: prefer parsed subResults (after completion), fall back to live events, then input specs
  const hasOutput = subResults.length > 0;
  const hasLive = liveAgents.length > 0;
  const agentCount = hasOutput
    ? subResults.length
    : hasLive
      ? liveAgents.length
      : agentSpecs.length;
  const doneCount = hasOutput
    ? subResults.filter((r) => r.status === "success").length
    : liveDoneCount;
  const failedCount = hasOutput
    ? subResults.filter((r) => r.status === "failed" || r.status === "timeout")
        .length
    : liveFailedCount;
  const completedCount = hasOutput
    ? subResults.length
    : liveDoneCount + liveFailedCount;
  const liveRunningCount = liveAgents.filter(
    (a) => a.status === "running",
  ).length;
  const isRunning = toolIsRunning || liveRunningCount > 0;
  const isError = toolIsError && liveRunningCount === 0;

  const progressPct = agentCount > 0 ? (completedCount / agentCount) * 100 : 0;
  const allDone = !isRunning && hasOutput;

  return (
    <div
      style={{
        margin: "6px 0 6px 8px",
        borderRadius: 12,
        background: "var(--surface-2)",
        border: "1px solid var(--border-soft)",
        overflow: "hidden",
      }}
    >
      {/* Header */}
      <div
        className="flex items-center"
        style={{
          gap: 8,
          padding: "7px 12px",
          cursor: "pointer",
        }}
        onClick={() => setExpanded((e) => !e)}
        onMouseEnter={(e) => {
          e.currentTarget.style.background = "rgba(255,255,255,0.03)";
        }}
        onMouseLeave={(e) => {
          e.currentTarget.style.background = "transparent";
        }}
      >
        {isRunning ? (
          <span
            className="spinner"
            style={{
              width: 10,
              height: 10,
              border: "1.5px solid var(--accent)",
              borderTopColor: "transparent",
              borderRadius: "50%",
              display: "inline-block",
              flexShrink: 0,
            }}
          />
        ) : isError || failedCount > 0 ? (
          <span
            style={{
              color:
                failedCount > 0 && doneCount > 0
                  ? "var(--warning, #f59e0b)"
                  : "var(--error)",
              fontSize: 12,
              flexShrink: 0,
              lineHeight: 1,
            }}
          >
            {failedCount > 0 && doneCount > 0 ? "⚠" : "✕"}
          </span>
        ) : (
          <span
            style={{
              color: "var(--success)",
              fontSize: 12,
              flexShrink: 0,
              lineHeight: 1,
            }}
          >
            ✓
          </span>
        )}

        <span
          style={{
            fontSize: 10,
            fontWeight: 600,
            padding: "1px 6px",
            borderRadius: 99,
            flexShrink: 0,
            background: "var(--accent-glow)",
            color: "var(--accent)",
            border: "1px solid var(--accent-border)",
            letterSpacing: "0.02em",
          }}
        >
          batch
        </span>

        <span
          style={{
            fontSize: 11,
            color: "var(--text-muted)",
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
            flex: 1,
          }}
        >
          {batchDesc || "(unnamed batch)"}
        </span>

        {agentCount > 0 && (
          <span
            style={{ fontSize: 10, color: "var(--text-dim)", flexShrink: 0 }}
          >
            {allDone
              ? `${doneCount}/${agentCount}`
              : `${completedCount}/${agentCount}`}
          </span>
        )}

        <span
          style={{
            color: "var(--text-dim)",
            display: "flex",
            flexShrink: 0,
            opacity: 0.5,
            transition: "transform 0.15s",
            transform: expanded ? "rotate(180deg)" : "none",
          }}
        >
          <Icon.ChevronDown size={10} />
        </span>
      </div>

      {/* Progress bar */}
      {agentCount > 1 && (
        <div
          style={{
            height: 2,
            background: "rgba(255,255,255,0.05)",
            margin: "0 12px",
          }}
        >
          <div
            style={{
              height: "100%",
              background:
                failedCount > 0 ? "var(--warning, #f59e0b)" : "var(--accent)",
              width: `${progressPct}%`,
              transition: "width 0.3s ease",
              borderRadius: 1,
            }}
          />
        </div>
      )}

      {/* Expanded content */}
      {expanded && (
        <div style={{ padding: "4px 0" }}>
          {/* Render sub-agent pills — prefer output-parsed results, then live events, then input specs */}
          {(hasOutput
            ? subResults.map((r) => ({
                description: r.description,
                subagent_type:
                  agentSpecs[r.index]?.subagent_type ||
                  liveAgents[r.index]?.subagent_type ||
                  "agent",
                status: r.status,
                duration: r.duration,
                output: r.output,
                error: r.error,
                nestedEvents: liveAgents[r.index]?.events || [],
              }))
            : hasLive
              ? liveAgents.map((a) => ({
                  description: a.description,
                  subagent_type: a.subagent_type,
                  status: a.status,
                  duration: a.duration_ms
                    ? `${(a.duration_ms / 1000).toFixed(1)}s`
                    : "",
                  output: "",
                  error: a.error,
                  nestedEvents: a.events,
                }))
              : agentSpecs.map((a) => ({
                  description: a.description,
                  subagent_type: a.subagent_type,
                  status: isRunning ? "running" : "pending",
                  duration: "",
                  output: "",
                  error: undefined,
                  nestedEvents: [],
                }))
          ).map((agent, i) => (
            <SubAgentPill
              key={i}
              description={agent.description}
              subtype={agent.subagent_type}
              status={agent.status}
              duration={agent.duration}
              output={agent.output}
              error={agent.error}
              nestedEvents={agent.nestedEvents}
              isRunning={agent.status === "running"}
              isDone={
                agent.status === "success" ||
                agent.status === "done" ||
                agent.status === "completed"
              }
              isFailed={agent.status === "failed" || agent.status === "timeout"}
            />
          ))}

          {/* File conflicts */}
          {conflicts.length > 0 && (
            <div
              style={{
                margin: "4px 10px",
                padding: "6px 10px",
                borderRadius: 8,
                background: "rgba(245,158,11,0.08)",
                border: "1px solid rgba(245,158,11,0.2)",
              }}
            >
              <div
                style={{
                  fontSize: 10,
                  fontWeight: 600,
                  color: "var(--warning, #f59e0b)",
                  marginBottom: 4,
                  textTransform: "uppercase",
                  letterSpacing: "0.03em",
                }}
              >
                File Conflicts
              </div>
              {conflicts.map((c, i) => (
                <div
                  key={i}
                  style={{
                    fontSize: 11,
                    color: "var(--text-muted)",
                    lineHeight: 1.4,
                  }}
                >
                  {c}
                </div>
              ))}
            </div>
          )}

          {/* Review prompt indicator */}
          {reviewPrompt.trim() && (
            <div
              style={{
                margin: "4px 10px",
                padding: "6px 10px",
                borderRadius: 8,
                background: "var(--accent-glow)",
                border: "1px solid var(--accent-border)",
              }}
            >
              <div
                style={{
                  fontSize: 10,
                  fontWeight: 600,
                  color: "var(--accent)",
                  marginBottom: 2,
                  textTransform: "uppercase",
                  letterSpacing: "0.03em",
                }}
              >
                Review Prompt
              </div>
              <div
                style={{
                  fontSize: 11,
                  color: "var(--text-muted)",
                  lineHeight: 1.4,
                  whiteSpace: "pre-wrap",
                }}
              >
                {reviewPrompt.trim()}
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  );
});

/** Individual sub-agent pill inside an AgentBatchRow. */
const SubAgentPill = React.memo(function SubAgentPill({
  description,
  subtype,
  status,
  duration,
  output,
  error,
  nestedEvents,
  isRunning,
  isDone,
  isFailed,
}: {
  description: string;
  subtype: string;
  status: string;
  duration: string;
  output: string;
  error?: string;
  nestedEvents?: Array<Record<string, unknown>>;
  isRunning: boolean;
  isDone: boolean;
  isFailed: boolean;
}) {
  const [expanded, setExpanded] = useState(isRunning);
  const hasNestedEvents = !!(nestedEvents && nestedEvents.length > 0);
  const hasContent = !!output?.trim() || !!error || hasNestedEvents;

  return (
    <div
      style={{
        margin: "2px 10px",
        borderRadius: 8,
        background: "rgba(255,255,255,0.02)",
        border: "1px solid var(--border-soft)",
      }}
    >
      <div
        className="flex items-center"
        style={{ gap: 6, padding: "4px 8px", cursor: "pointer" }}
        onClick={() => setExpanded((e) => !e)}
        onMouseEnter={(e) => {
          e.currentTarget.style.background = "rgba(255,255,255,0.03)";
        }}
        onMouseLeave={(e) => {
          e.currentTarget.style.background = "transparent";
        }}
      >
        {isRunning ? (
          <span
            className="spinner"
            style={{
              width: 7,
              height: 7,
              border: "1.5px solid var(--accent)",
              borderTopColor: "transparent",
              borderRadius: "50%",
              display: "inline-block",
              flexShrink: 0,
            }}
          />
        ) : isFailed ? (
          <span
            style={{
              color:
                status === "timeout"
                  ? "var(--warning, #f59e0b)"
                  : "var(--error)",
              fontSize: 10,
              flexShrink: 0,
              lineHeight: 1,
            }}
          >
            {status === "timeout" ? "⏱" : "✕"}
          </span>
        ) : isDone ? (
          <span
            style={{
              color: "var(--success)",
              fontSize: 10,
              flexShrink: 0,
              lineHeight: 1,
            }}
          >
            ✓
          </span>
        ) : (
          <span
            style={{
              color: "var(--text-dim)",
              fontSize: 10,
              flexShrink: 0,
              lineHeight: 1,
            }}
          >
            ○
          </span>
        )}

        <span
          style={{
            fontSize: 9,
            fontWeight: 600,
            padding: "0px 5px",
            borderRadius: 99,
            flexShrink: 0,
            background: "var(--accent-glow)",
            color: "var(--accent)",
            border: "1px solid var(--accent-border)",
            letterSpacing: "0.02em",
          }}
        >
          {subtype}
        </span>

        <span
          style={{
            fontSize: 10,
            color: "var(--text-muted)",
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
            flex: 1,
          }}
        >
          {description}
        </span>

        {duration && (
          <span
            style={{ fontSize: 9, color: "var(--text-dim)", flexShrink: 0 }}
          >
            {duration}
          </span>
        )}

        <span
          style={{
            color: "var(--text-dim)",
            display: "flex",
            flexShrink: 0,
            opacity: 0.4,
            transition: "transform 0.15s",
            transform: expanded ? "rotate(180deg)" : "none",
          }}
        >
          <Icon.ChevronDown size={8} />
        </span>
      </div>

      {expanded && hasContent && (
        <div style={{ borderTop: "1px solid var(--border-soft)" }}>
          {hasNestedEvents && (
            <NestedAgentEventList events={nestedEvents || []} />
          )}
          {error && (
            <div
              style={{
                padding: "4px 8px",
                fontSize: 10,
                color: "var(--error)",
                fontStyle: "italic",
              }}
            >
              {error}
            </div>
          )}
          {output?.trim() && (
            <pre
              style={{
                margin: 0,
                padding: "4px 8px 6px",
                fontSize: 10,
                color: "var(--text-muted)",
                fontFamily: "var(--font-mono)",
                maxHeight: 200,
                overflowY: "auto",
                whiteSpace: "pre-wrap",
                wordBreak: "break-word",
              }}
            >
              {output.trim().length > 4000
                ? output.trim().slice(0, 4000) + "\n…(capped)"
                : output.trim()}
            </pre>
          )}
        </div>
      )}

      {expanded && !hasContent && isRunning && (
        <div
          style={{
            borderTop: "1px solid var(--border-soft)",
            padding: "6px 8px",
            fontSize: 10,
            color: "var(--text-dim)",
            fontStyle: "italic",
          }}
        >
          Agent running…
        </div>
      )}
    </div>
  );
});

const NestedAgentRunBranch = React.memo(function NestedAgentRunBranch({
  node,
  level,
}: {
  node: NestedAgentRunNode;
  level: number;
}) {
  const start = node.events.find(
    (event) => event.type === "nested_start" || event.type === "batch_start",
  );
  const failure = node.events.find((event) => event.type === "nested_error");
  const finished = node.events.some(
    (event) => event.type === "nested_done" || event.type === "batch_done",
  );
  const [expanded, setExpanded] = useState(true);
  const description =
    (start?.description as string | undefined) || "Delegated task";
  const subtype = String(
    start?.subagent_type || (start?.type === "batch_start" ? "batch" : "agent"),
  ).replace(/-/g, " ");
  const depth = typeof start?.depth === "number" ? start.depth : level + 1;
  const timelineEvents = node.events.filter(
    (event) =>
      event.type !== "nested_start" &&
      event.type !== "nested_done" &&
      event.type !== "nested_error" &&
      event.type !== "batch_start" &&
      event.type !== "batch_done",
  );

  return (
    <div
      style={{
        marginLeft: level ? 12 : 0,
        borderLeft: `1px solid ${level ? "var(--accent-border)" : "var(--border-soft)"}`,
      }}
    >
      <button
        type="button"
        onClick={() => setExpanded((value) => !value)}
        style={{
          width: "100%",
          minHeight: 30,
          display: "grid",
          gridTemplateColumns: "14px auto minmax(0, 1fr) auto",
          alignItems: "center",
          gap: 7,
          padding: "4px 8px",
          border: 0,
          background: "transparent",
          color: "var(--text-muted)",
          cursor: "pointer",
          textAlign: "left",
        }}
      >
        <span
          style={{
            display: "flex",
            color: failure
              ? "var(--error)"
              : finished
                ? "var(--success)"
                : "var(--accent)",
          }}
        >
          {failure ? (
            "✕"
          ) : finished ? (
            "✓"
          ) : (
            <span
              className="spinner"
              style={{
                width: 9,
                height: 9,
                border: "1.5px solid currentColor",
                borderTopColor: "transparent",
                borderRadius: "50%",
              }}
            />
          )}
        </span>
        <span
          style={{
            color: "var(--accent)",
            fontSize: 9,
            fontWeight: 600,
            textTransform: "uppercase",
          }}
        >
          {subtype}
        </span>
        <span
          style={{
            minWidth: 0,
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
            fontSize: 10,
          }}
        >
          {description}
        </span>
        <span
          style={{
            display: "flex",
            alignItems: "center",
            gap: 5,
            color: "var(--text-dim)",
            fontSize: 9,
          }}
        >
          depth {depth}
          <Icon.ChevronDown
            size={10}
            style={{
              transform: expanded ? "rotate(180deg)" : "none",
              transition: "transform 0.15s",
            }}
          />
        </span>
      </button>
      {expanded && (
        <div style={{ paddingLeft: 6 }}>
          {failure && (
            <div
              style={{
                padding: "3px 8px 5px",
                color: "var(--error)",
                fontSize: 10,
              }}
            >
              {String(failure.error || "Sub-agent failed")}
            </div>
          )}
          {timelineEvents.length > 0 && (
            <NestedAgentEventList events={timelineEvents} flat />
          )}
          {node.children.map((child) => (
            <NestedAgentRunBranch
              key={child.runId}
              node={child}
              level={level + 1}
            />
          ))}
        </div>
      )}
    </div>
  );
});

/// Compact timeline of events forwarded from a child provider subprocess.
const NestedAgentEventList = React.memo(function NestedAgentEventList({
  events,
  flat = false,
}: {
  events: Array<Record<string, unknown>>;
  flat?: boolean;
}) {
  const runTree = buildNestedAgentRunTree(events);
  const hasDescendants = runTree.some((node) => node.children.length > 0);
  if (!flat && hasDescendants) {
    return (
      <div
        style={{ padding: "5px 8px 7px", maxHeight: 420, overflowY: "auto" }}
      >
        {runTree.map((node) => (
          <NestedAgentRunBranch key={node.runId} node={node} level={0} />
        ))}
      </div>
    );
  }

  // Flatten to a list of simple rows. Walk each event and pull the interesting
  // bits: nested_start/done/error, assistant text + tool_use blocks, tool_result.
  type Row =
    | {
        kind: "start";
        description?: string;
        subagent_type?: string;
        depth?: number;
      }
    | { kind: "done" }
    | { kind: "error"; error?: string }
    | {
        kind: "progress";
        elapsedMs?: number;
        idleMs?: number;
        idleTimeoutSecs?: number;
      }
    | { kind: "text"; text: string }
    | { kind: "tool_use"; id: string; name: string; arg: string }
    | { kind: "tool_result"; id: string; isError: boolean; preview: string }
    | { kind: "raw"; line: string }
    | { kind: "other"; type: string };

  const commandEvents = React.useMemo(
    () => nestedCommandEventsToCommandEvents(events),
    [events],
  );
  const rows: Row[] = [];
  const seenUse = new Set<string>();
  for (const ev of events) {
    const t = (ev?.type as string) || "";
    if (t === "nested_start") {
      rows.push({
        kind: "start",
        description: ev.description as string | undefined,
        subagent_type: ev.subagent_type as string | undefined,
        depth: ev.depth as number | undefined,
      });
    } else if (t === "nested_done") {
      rows.push({ kind: "done" });
    } else if (t === "nested_error") {
      rows.push({ kind: "error", error: ev.error as string | undefined });
    } else if (t === "nested_progress") {
      rows.push({
        kind: "progress",
        elapsedMs: ev.elapsed_ms as number | undefined,
        idleMs: ev.idle_ms as number | undefined,
        idleTimeoutSecs: ev.idle_timeout_secs as number | undefined,
      });
    } else if (t === "nested_raw") {
      rows.push({ kind: "raw", line: (ev.line as string) || "" });
    } else if (t === "assistant") {
      const msg = ev.message as any;
      const content = (msg?.content as any[]) || [];
      for (const b of content) {
        if (!b) continue;
        if (b.type === "text") {
          const txt = (b.text as string) || "";
          if (txt.trim()) rows.push({ kind: "text", text: txt });
        } else if (b.type === "tool_use") {
          if (seenUse.has(b.id)) continue;
          seenUse.add(b.id);
          const inp = (b.input as Record<string, unknown>) || {};
          // Prefer a meaningful single argument for the preview.
          const arg =
            (inp.file_path as string) ||
            (inp.relative_path as string) ||
            (inp.path as string) ||
            (inp.pattern as string) ||
            (inp.command as string) ||
            (inp.description as string) ||
            (inp.name_path as string) ||
            (inp.query as string) ||
            "";
          rows.push({
            kind: "tool_use",
            id: b.id as string,
            name: (b.name as string) || "?",
            arg,
          });
        }
      }
    } else if (t === "item.started" || t === "item.completed") {
      const item = ev.item as any;
      const itemType = (item?.type as string) || "item";
      const itemId = (item?.id as string) || `${itemType}-${rows.length}`;
      if (itemType === "agent_message") {
        const text = (item?.text as string) || "";
        if (text.trim()) rows.push({ kind: "text", text });
      } else if (itemType === "mcp_tool_call" || itemType === "mcpToolCall") {
        const server = (item?.server as string) || "";
        const tool = (item?.tool as string) || "tool";
        const name = server ? `mcp__${server}__${tool}` : tool;
        const rawArgs = item?.arguments;
        const arg =
          typeof rawArgs === "string"
            ? rawArgs
            : rawArgs && rawArgs !== null
              ? JSON.stringify(rawArgs)
              : (item?.status as string) || "";
        if (t === "item.started" || item?.status === "in_progress") {
          if (!seenUse.has(itemId)) {
            seenUse.add(itemId);
            rows.push({ kind: "tool_use", id: itemId, name, arg });
          }
        } else {
          const result =
            item?.result ?? item?.output ?? item?.status ?? "completed";
          const preview =
            typeof result === "string"
              ? result
              : JSON.stringify(result).slice(0, 200);
          rows.push({
            kind: "tool_result",
            id: itemId,
            isError: item?.status === "failed" || !!item?.error,
            preview,
          });
        }
      } else if (itemType === "tool_call" || itemType === "command_execution") {
        const name =
          (item?.name as string) ||
          (item?.command as string) ||
          itemType.replace(/_/g, " ");
        const arg =
          (item?.arguments as string) ||
          (item?.command as string) ||
          (item?.status as string) ||
          "";
        if (t === "item.started") {
          if (!seenUse.has(itemId)) {
            seenUse.add(itemId);
            rows.push({ kind: "tool_use", id: itemId, name, arg });
          }
        } else {
          rows.push({
            kind: "tool_result",
            id: itemId,
            isError: item?.status === "failed",
            preview:
              (item?.output as string) ||
              (item?.status as string) ||
              "completed",
          });
        }
      } else if (t === "item.completed" && itemType === "reasoning") {
        rows.push({ kind: "other", type: "reasoning" });
      }
    } else if (t === "message_delta" || t === "agent_message.delta") {
      const text = (ev.delta as string) || "";
      if (text.trim()) rows.push({ kind: "text", text });
    } else if (t === "command_execution.output_delta") {
      // Render command deltas through the unified console below.
    } else if (t === "user") {
      // tool_result is delivered as a user message with content blocks.
      const msg = ev.message as any;
      const content = (msg?.content as any[]) || [];
      for (const b of content) {
        if (!b || b.type !== "tool_result") continue;
        const raw =
          typeof b.content === "string"
            ? b.content
            : Array.isArray(b.content)
              ? b.content
                  .map((x: any) => (x?.type === "text" ? x.text : ""))
                  .join("")
              : "";
        const preview =
          (raw as string).trim().split("\n")[0]?.slice(0, 140) || "";
        rows.push({
          kind: "tool_result",
          id: (b.tool_use_id as string) || "",
          isError: !!b.is_error,
          preview,
        });
      }
    } else if (t === "result" || t === "system" || t === "stream_event") {
      // Noise — terminal/init/partial events. Skip.
    } else if (t) {
      rows.push({ kind: "other", type: t });
    }
  }

  if (rows.length === 0 && commandEvents.length === 0) {
    return (
      <div
        style={{
          padding: "6px 12px 8px",
          fontSize: 11,
          color: "var(--text-dim)",
          fontStyle: "italic",
        }}
      >
        (sub-agent emitted no events)
      </div>
    );
  }

  return (
    <div
      style={{
        padding: "4px 8px 6px",
        maxHeight: 360,
        overflowY: "auto",
        fontSize: 11,
        fontFamily: "var(--font-mono)",
      }}
    >
      {commandEvents.length > 0 && (
        <InteractiveConsole
          events={commandEvents}
          connected
          title="Sub-agent console"
          height={150}
          compact
        />
      )}
      {rows.map((row, i) => {
        if (row.kind === "start") {
          const label = [row.subagent_type, row.description]
            .filter(Boolean)
            .join(" · ");
          return (
            <div
              key={i}
              style={{
                padding: "2px 6px",
                color: "var(--text-dim)",
                fontSize: 10,
              }}
            >
              ▸ sub-agent started {row.depth ? `(depth ${row.depth})` : ""}
              {label && (
                <span style={{ marginLeft: 6, opacity: 0.7 }}>— {label}</span>
              )}
            </div>
          );
        }
        if (row.kind === "done") {
          return (
            <div
              key={i}
              style={{
                padding: "2px 6px",
                color: "var(--success)",
                fontSize: 10,
              }}
            >
              ✓ sub-agent finished
            </div>
          );
        }
        if (row.kind === "error") {
          return (
            <div
              key={i}
              style={{
                padding: "2px 6px",
                color: "var(--error)",
                fontSize: 10,
              }}
            >
              ✕ {row.error || "sub-agent error"}
            </div>
          );
        }
        if (row.kind === "progress") {
          const elapsed = row.elapsedMs
            ? `${Math.round(row.elapsedMs / 1000)}s`
            : "";
          const idle = row.idleMs
            ? `${Math.round(row.idleMs / 1000)}s idle`
            : "active";
          const timeout = row.idleTimeoutSecs
            ? ` / ${row.idleTimeoutSecs}s`
            : "";
          return (
            <div
              key={i}
              style={{
                padding: "2px 6px",
                color: "var(--text-dim)",
                fontSize: 10,
              }}
            >
              ↻ running {elapsed && `for ${elapsed}`} · {idle}
              {timeout}
            </div>
          );
        }
        if (row.kind === "text") {
          const txt =
            row.text.length > 400 ? row.text.slice(0, 397) + "…" : row.text;
          return (
            <div
              key={i}
              style={{
                padding: "3px 6px",
                color: "var(--text-muted)",
                whiteSpace: "pre-wrap",
                wordBreak: "break-word",
                borderLeft: "2px solid var(--border-soft)",
                margin: "2px 0 2px 2px",
                fontStyle: "italic",
              }}
            >
              {txt}
            </div>
          );
        }
        if (row.kind === "tool_use") {
          const { label, server, isReclaude } = parseToolName(row.name);
          return (
            <div
              key={i}
              className="flex items-center"
              style={{ gap: 6, padding: "2px 6px", color: "var(--text-muted)" }}
            >
              <span style={{ color: "var(--accent)" }}>▸</span>
              {server && (
                <span style={{ fontSize: 9, opacity: 0.6 }}>{server}</span>
              )}
              <span
                style={{
                  fontWeight: 600,
                  color: isReclaude ? "var(--accent)" : "var(--text)",
                }}
              >
                {label}
              </span>
              {row.arg && (
                <span
                  style={{
                    opacity: 0.7,
                    overflow: "hidden",
                    textOverflow: "ellipsis",
                    whiteSpace: "nowrap",
                    flex: 1,
                  }}
                >
                  {row.arg.length > 80 ? row.arg.slice(0, 77) + "…" : row.arg}
                </span>
              )}
            </div>
          );
        }
        if (row.kind === "tool_result") {
          return (
            <div
              key={i}
              className="flex items-center"
              style={{
                gap: 6,
                padding: "1px 6px 1px 14px",
                color: row.isError ? "var(--error)" : "var(--success)",
                fontSize: 10,
              }}
            >
              <span>{row.isError ? "✕" : "✓"}</span>
              {row.preview && (
                <span
                  style={{
                    opacity: 0.7,
                    overflow: "hidden",
                    textOverflow: "ellipsis",
                    whiteSpace: "nowrap",
                    flex: 1,
                    color: "var(--text-dim)",
                  }}
                >
                  {row.preview}
                </span>
              )}
            </div>
          );
        }
        if (row.kind === "raw") {
          return (
            <div
              key={i}
              style={{
                padding: "1px 6px",
                color: "var(--text-dim)",
                opacity: 0.6,
                fontSize: 10,
              }}
            >
              {row.line.length > 160 ? row.line.slice(0, 157) + "…" : row.line}
            </div>
          );
        }
        // other
        return (
          <div
            key={i}
            style={{
              padding: "1px 6px",
              color: "var(--text-dim)",
              opacity: 0.5,
              fontSize: 10,
            }}
          >
            {row.type}
          </div>
        );
      })}
    </div>
  );
});
