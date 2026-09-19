import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { DisplayBlock } from "../../lib/messageProcessor";
import { processHistoryMessages } from "../../lib/historyProcessor";
import BlockList from "./BlockList";
import { captureGuidanceAnchor } from "../../lib/guidanceTranscript";

function assistant(id: string, updates: Partial<DisplayBlock> = {}): DisplayBlock {
  return { id, type: "assistant", role: "assistant", model: "gpt-6-astra", content: id, ...updates };
}

function user(id: string): DisplayBlock {
  return { id, type: "user", role: "user", content: id };
}

function render(blocks: DisplayBlock[]) {
  return renderToStaticMarkup(<BlockList tabId="header-test" blocks={blocks} />);
}

function headerCount(html: string) {
  return (html.match(/class="chat-assistant-header /g) || []).length;
}

describe("assistant turn attribution", () => {
  it("shows confirmed guidance as a user message between assistant fragments with no edit action", () => {
    const before = assistant("reply", { content: "Before." });
    const anchor = captureGuidanceAnchor([before]);
    const html = renderToStaticMarkup(<BlockList tabId="header-test"
      blocks={[{ ...before, content: "Before. After." }]}
      inputDeliveries={[{ requestId: "guidance", sessionId: "session", message: "finishhh",
        delivery: "sent", transcriptAnchor: anchor }]}
      onEditRerun={() => { throw new Error("Synthetic guidance cannot be rerun"); }} />);
    expect(html).toContain("Accepted by current run");
    expect(html.indexOf("Before.")).toBeLessThan(html.indexOf("finishhh"));
    expect(html.indexOf("finishhh")).toBeLessThan(html.indexOf("After."));
    expect(headerCount(html)).toBe(2);
    expect(html).not.toContain("Edit &amp; re-run");
  });

  it("keeps old guidance offscreen with its original history block", () => {
    const blocks = Array.from({ length: 23 }, (_, index) => assistant(`fragment-${index}`));
    const html = renderToStaticMarkup(<BlockList tabId="header-test" blocks={blocks}
      inputDeliveries={[{ requestId: "old-guidance", sessionId: "session", message: "old guidance",
        delivery: "sent", transcriptAnchor: captureGuidanceAnchor([blocks[0]]) }]} />);
    expect(html).toContain("earlier messages");
    expect(html).not.toContain("old guidance");
    expect(html).toContain("fragment-3");
    expect(headerCount(html)).toBe(1);
  });
  it("keeps the first visible reply attributed after hidden history records", () => {
    const blocks = processHistoryMessages([
      { type: "user", uuid: "user", message: { content: [{ type: "text", text: "finish the plan" }] } },
      { type: "assistant", uuid: "empty", message: { content: [] } },
      { type: "assistant", uuid: "hidden", message: { content: [
        { type: "tool_use", id: "search", name: "ToolSearch", input: {} },
      ] } },
      { type: "assistant", uuid: "reply", message: { content: [
        { type: "text", text: "I’ll continue with the checks." },
      ] } },
      { type: "assistant", uuid: "commentary", message: { content: [
        { type: "text", text: "The next check passed." },
      ] } },
    ], "gpt-6-astra");
    const html = render(blocks);
    expect(headerCount(html)).toBe(1);
    expect(html.indexOf("finish the plan")).toBeLessThan(html.indexOf("chat-assistant-header"));
    expect(html.indexOf("chat-assistant-header")).toBeLessThan(html.indexOf("I’ll continue"));
    expect(html).toContain("gpt-6-astra");
    expect(html).toContain("The next check passed.");
  });

  it("uses one header across text, tools and the final answer, then starts a new turn", () => {
    const html = render([
      user("Check it"), assistant("I’ll check."),
      assistant("tools", { content: "", tools: [{
        id: "read", name: "file_read", input: { file: "src/main.rs" }, status: "done",
      }] }),
      assistant("Done."), user("Next thing"), assistant("Next response"),
    ]);
    expect(headerCount(html)).toBe(2);
    expect(html).toContain("I’ll check.");
    expect(html).toContain("Done.");
    expect(html).toContain("Next response");
  });

  it("ignores invisible fragments without repeating the existing turn header", () => {
    const html = render([
      user("Go"), assistant("Opening reply"), assistant("empty", { content: "" }),
      { id: "system", type: "system", role: "system" }, assistant("Continuation"),
    ]);
    expect(headerCount(html)).toBe(1);
  });

  it("keeps one header as startup becomes text and another fragment starts working", () => {
    const initial = render([user("Go"), assistant("start", { content: "", isStreaming: true })]);
    const streaming = render([
      user("Go"), assistant("First text"), assistant("next", { content: "", isStreaming: true }),
    ]);
    expect(headerCount(initial)).toBe(1);
    expect(initial).toContain("Working…");
    expect(headerCount(streaming)).toBe(1);
    expect(streaming).toContain("First text");
    expect(streaming).toContain("Working…");
  });

  it("attributes the first rendered history page even when earlier turn fragments are offscreen", () => {
    const blocks = Array.from({ length: 23 }, (_, index) => assistant(`fragment-${index}`));
    blocks[3] = assistant("empty", { content: "" });
    const html = render(blocks);
    expect(html).toContain("earlier messages");
    expect(headerCount(html)).toBe(1);
    expect(html.indexOf("chat-assistant-header")).toBeLessThan(html.indexOf("fragment-4"));
  });

  it("starts a fresh attribution after an explicitly completed or interrupted response", () => {
    for (const stopReason of ["end_turn", "user_interrupted", "error"]) {
      const html = render([assistant("Finished", { stopReason }), assistant("Resumed response")]);
      expect(headerCount(html)).toBe(2);
    }
  });

  it("does not create headers for empty or redacted-only completed blocks", () => {
    const html = render([
      user("Go"), assistant("empty", { content: "  " }),
      assistant("redacted", { content: "", segments: [{ type: "thinking", text: "", redacted: true }] }),
    ]);
    expect(headerCount(html)).toBe(0);
  });
});

it("withholds rerun and continue actions while saved history is read-only", () => {
  const blocks = [user("Saved user message"), assistant("Saved response", { stopReason: "user_interrupted" })];
  const html = renderToStaticMarkup(<BlockList tabId="syncing-history" blocks={blocks} readOnly onEditRerun={() => {}} />);
  expect(html).toContain("Saved user message");
  expect(html).toContain("Saved response");
  expect(html).not.toContain("Continue →");
  expect(html).not.toContain("Edit &amp; re-run");
});
