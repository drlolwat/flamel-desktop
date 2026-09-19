import { describe, expect, it } from "vitest";
import {
  completedQuestionRequestId,
  normalizePlanInputItems,
  normalizeWorkflowEvent,
  parsePartialPlanInput,
  pendingQuestionFromWorkflowEvent,
} from "./workflowEvents";

describe("plan input normalization", () => {
  it("normalizes alternate plan item shapes without blank rows", () => {
    expect(normalizePlanInputItems([
      { step: "Inspect the API", status: "pending", phase: "Backend" },
      JSON.stringify({ content: "Fix the panel", status: "completed", group: "Frontend" }),
    ])).toEqual([
      expect.objectContaining({ text: "Inspect the API", status: "todo", section: "Backend" }),
      expect.objectContaining({ text: "Fix the panel", status: "done", section: "Frontend" }),
    ]);
  });

  it("parses JSON-encoded item arrays", () => {
    expect(normalizePlanInputItems(JSON.stringify([
      { text: "Forward MCP auth", status: "in_progress" },
    ]))).toEqual([
      expect.objectContaining({ text: "Forward MCP auth", status: "in_progress" }),
    ]);
  });

  it("normalizes completed objects from partial tool JSON", () => {
    const parsed = parsePartialPlanInput(
      '{"action":"create","title":"Plan","items":[{"step":"Visible row","status":"pending"}]}',
    );
    expect(parsed?.items).toEqual([
      expect.objectContaining({ text: "Visible row", status: "todo" }),
    ]);
  });
});

describe("question lifecycle normalization", () => {
  it("normalizes durable and legacy requested events through one parser", () => {
    const durable = normalizeWorkflowEvent({
      type: "workflow_event",
      kind: "question.requested",
      source: "backend",
      payload: {
        requestId: "ask-1",
        sessionId: "session-1",
        runId: "run-1",
        status: "pending",
        question: "Which path?",
        choices: [
          { label: "A", description: "Fast" },
          { label: "B", description: "Safe" },
        ],
        allowFreeText: false,
      },
    });
    expect(pendingQuestionFromWorkflowEvent(durable)).toEqual({
      requestId: "ask-1",
      sessionId: "session-1",
      runId: "run-1",
      status: "pending",
      question: "Which path?",
      choices: [
        { label: "A", description: "Fast" },
        { label: "B", description: "Safe" },
      ],
      allowFreeText: false,
      multiSelect: false,
      previewMarkdown: undefined,
    });

    const legacy = normalizeWorkflowEvent({
      type: "ask_question",
      requestId: "ask-2",
      question: "Continue?",
    });
    expect(pendingQuestionFromWorkflowEvent(legacy)?.requestId).toBe("ask-2");
  });

  it("clears only the matching resolved or cancelled request", () => {
    expect(
      completedQuestionRequestId(
        normalizeWorkflowEvent({
          type: "question_resolved",
          requestId: "ask-1",
        }),
      ),
    ).toBe("ask-1");
    expect(
      completedQuestionRequestId(
        normalizeWorkflowEvent({
          type: "question_cancelled",
          requestId: "ask-2",
        }),
      ),
    ).toBe("ask-2");
  });
});
