import type { ContentSegment, DisplayBlock } from "./messageProcessor";
import type { InputDelivery } from "./chatInput";

export type GuidanceAnchor = {
  blockId: string;
  segmentCount: number;
  tailTextLength: number;
};

function authoredSegments(block: DisplayBlock): ContentSegment[] {
  const segments = block.segments?.length ? [...block.segments] : [
    ...(block.content ? [{ type: "text" as const, text: block.content }] : []),
    ...(block.tools || []).map(tool => ({ type: "tool_ref" as const, toolId: tool.id })),
  ];
  if (block.thinking && !segments.some(segment => segment.type === "thinking")) {
    segments.unshift({ type: "thinking", text: block.thinking });
  }
  return segments;
}

export function captureGuidanceAnchor(blocks: DisplayBlock[]): GuidanceAnchor | undefined {
  const block = blocks[blocks.length - 1];
  if (block?.type !== "assistant") return undefined;
  const segments = authoredSegments(block);
  const tail = segments[segments.length - 1];
  return {
    blockId: block.id, segmentCount: segments.length,
    tailTextLength: tail && tail.type !== "tool_ref" ? tail.text.length : 0,
  };
}

export type TranscriptEntry = { block: DisplayBlock; globalIdx: number; guidance?: boolean };

// This is a presentation projection, never a mutation of the active assistant
// buffer. Stream deltas and tool receipts still update their original block.
export function guidanceTranscript(blocks: DisplayBlock[], receipts: InputDelivery[]): TranscriptEntry[] {
  const persisted = new Set(blocks.filter(block => block.type === "user").map(block => block.requestId));
  const fallback = captureGuidanceAnchor(blocks);
  const blockIds = new Set(blocks.map(block => block.id));
  const seen = new Set<string>();
  const pending = receipts.filter(receipt => {
    if (receipt.delivery !== "sent" || persisted.has(receipt.requestId) || seen.has(receipt.requestId)) return false;
    seen.add(receipt.requestId);
    return true;
  }).map(receipt => ({
    receipt,
    anchor: receipt.transcriptAnchor && blockIds.has(receipt.transcriptAnchor.blockId)
      ? receipt.transcriptAnchor : fallback,
  }));

  const entries = blocks.flatMap((block, globalIdx): TranscriptEntry[] => {
    const inputs = pending.filter(input => input.anchor?.blockId === block.id)
      .sort((a, b) => a.anchor!.segmentCount - b.anchor!.segmentCount || a.anchor!.tailTextLength - b.anchor!.tailTextLength);
    if (!inputs.length) return [{ block, globalIdx }];
    const segments = authoredSegments(block);
    const chunks: ContentSegment[][] = [[]];
    let segmentIndex = 0;
    let textOffset = 0;
    for (const { anchor } of inputs) {
      const count = Math.min(anchor!.segmentCount, segments.length);
      const tail = segments[count - 1];
      const splitText = tail && tail.type !== "tool_ref";
      const endIndex = splitText ? count - 1 : count;
      const chunk = chunks[chunks.length - 1];
      while (segmentIndex < endIndex) {
        const segment = segments[segmentIndex++];
        chunk.push(segment.type === "tool_ref" ? segment : { ...segment, text: segment.text.slice(textOffset) });
        textOffset = 0;
      }
      if (splitText) {
        const endOffset = Math.max(textOffset, Math.min(anchor!.tailTextLength, tail.text.length));
        if (endOffset > textOffset) chunk.push({ ...tail, text: tail.text.slice(textOffset, endOffset) });
        textOffset = endOffset;
      }
      chunks.push([]);
    }
    while (segmentIndex < segments.length) {
      const segment = segments[segmentIndex++];
      chunks[chunks.length - 1].push(segment.type === "tool_ref" ? segment : { ...segment, text: segment.text.slice(textOffset) });
      textOffset = 0;
    }
    return chunks.flatMap((chunk, index): TranscriptEntry[] => {
      const last = index === chunks.length - 1;
      const toolIds = new Set(chunk.flatMap(segment => segment.type === "tool_ref" ? [segment.toolId] : []));
      const fragment: DisplayBlock = {
        ...block, id: last ? block.id : `${block.id}:guidance:${inputs[index].receipt.requestId}`,
        segments: chunk,
        content: chunk.flatMap(segment => segment.type === "text" ? [segment.text] : []).join(""),
        thinking: chunk.flatMap(segment => segment.type === "thinking" ? [segment.text] : []).join("\n"),
        tools: block.tools?.filter(tool => toolIds.has(tool.id)),
        isStreaming: last && block.isStreaming,
        usage: last ? block.usage : undefined,
        stopReason: last ? block.stopReason : undefined,
        statusText: last ? block.statusText : undefined,
        recoveryNote: index === 0 ? block.recoveryNote : undefined,
        recoverySummary: index === 0 ? block.recoverySummary : undefined,
      };
      return [
        { block: fragment, globalIdx },
        ...(!last ? [guidanceEntry(inputs[index].receipt, globalIdx)] : []),
      ];
    });
  });
  return [...entries, ...pending.filter(input => !input.anchor).map(input => guidanceEntry(input.receipt, blocks.length))];
}

function guidanceEntry(receipt: InputDelivery, globalIdx: number): TranscriptEntry {
  return {
    globalIdx, guidance: true,
    block: { id: `guidance:${receipt.requestId}`, requestId: receipt.requestId,
      type: "user", role: "user", content: receipt.message },
  };
}
