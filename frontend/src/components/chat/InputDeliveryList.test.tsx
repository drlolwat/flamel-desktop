import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { InputDelivery } from "../../lib/chatInput";
import type { DisplayBlock } from "../../lib/messageProcessor";
import { captureGuidanceAnchor } from "../../lib/guidanceTranscript";
import { useChatStore } from "../../stores/chatStore";
import BlockList from "./BlockList";
import { InputDeliveryList } from "./InputDeliveryList";

const sent: InputDelivery = {
  requestId: "guidance-1", sessionId: "session", message: "Finish the plan", delivery: "sent",
};
const assistant: DisplayBlock = {
  id: "reply", type: "assistant", role: "assistant", content: "Working on it.",
};
const render = (receipts: InputDelivery[]) => renderToStaticMarkup(<InputDeliveryList receipts={receipts} />);

describe("guidance delivery tray", () => {
  it("removes the entire tray when every receipt is accepted", () => {
    expect(render([sent])).toBe("");
    expect(render([])).toBe("");
  });

  it.each(["saving", "waiting", "queued", "uncertain", "failed"] as const)(
    "keeps %s guidance visible even when a newer message is accepted",
    delivery => {
      const pending: InputDelivery = {
        ...sent, requestId: "pending", message: "Keep the tests", delivery,
        note: "Delivery has not been confirmed.",
      };
      const html = render([pending, sent]);
      expect(html).toContain("Keep the tests");
      if (delivery !== "queued") expect(html).toContain("Delivery has not been confirmed.");
      expect(html).not.toContain("Finish the plan");
      expect(html).not.toContain("Accepted by current run");
      expect(html.split("Keep the tests")).toHaveLength(2);
      expect(html).not.toContain("<details");
    },
  );

  it("removes a queued receipt once that exact request is in the conversation", () => {
    const receipts: InputDelivery[] = [
      { ...sent, delivery: "queued" },
      { ...sent, requestId: "next", delivery: "queued", message: "Do this later" },
    ];
    const html = renderToStaticMarkup(<InputDeliveryList receipts={receipts} deliveredRequestIds={[sent.requestId]} />);
    expect(html).not.toContain(sent.message);
    expect(html.split("Do this later")).toHaveLength(2);
    expect(receipts).toHaveLength(2);
  });

  it.each(["live", "reconnected", "persisted", "empty history"])(
    "retains accepted guidance once in the %s transcript, not in the tray",
    state => {
      const receipt = { ...sent, transcriptAnchor: captureGuidanceAnchor([assistant]) };
      const blocks: DisplayBlock[] = state === "empty history" ? []
        : state === "persisted" ? [{ id: "saved", type: "user", role: "user", content: sent.message, requestId: sent.requestId }, assistant]
        : [{ ...assistant, id: state === "reconnected" ? "restored-reply" : assistant.id }];
      const receipts = [receipt];
      const original = structuredClone(receipts);
      const transcript = renderToStaticMarkup(<BlockList tabId="delivery-test" blocks={blocks} inputDeliveries={receipts} />);
      expect(transcript.split(sent.message)).toHaveLength(2);
      expect(render(receipts)).toBe("");
      expect(receipts).toEqual(original);
    },
  );

  it("clears on acknowledgement without deleting the receipt needed by chat history", () => {
    const tab = "delivery-transition";
    const store = useChatStore.getState();
    store.setSessionId(tab, sent.sessionId);
    store.addBlock(tab, assistant);
    store.recordInputDelivery(tab, { ...sent, delivery: "waiting" });
    expect(render(store.getChat(tab).inputDeliveries!)).toContain("Waiting for delivery");

    store.recordInputDelivery(tab, sent);
    const chat = store.getChat(tab);
    expect(render(chat.inputDeliveries!)).toBe("");
    expect(chat.inputDeliveries).toHaveLength(1);
    const transcript = renderToStaticMarkup(<BlockList tabId={tab} blocks={chat.blocks} inputDeliveries={chat.inputDeliveries} />);
    expect(transcript).toContain(sent.message);
    store.clearChat(tab);
  });
});

it("clears consumed legacy queued inputs even when history has no request IDs", () => {
  const receipts: InputDelivery[] = [{ ...sent, delivery: "queued", runStartedAt: "2026-09-08T02:00:00Z" }];
  const blocks: DisplayBlock[] = [{ id: "legacy", type: "user", role: "user", content: sent.message }, assistant];
  expect(render(receipts)).toBe("");
  const html = renderToStaticMarkup(<BlockList tabId="legacy-queued" blocks={blocks} inputDeliveries={receipts} />);
  expect(html.split(sent.message)).toHaveLength(2);
});
