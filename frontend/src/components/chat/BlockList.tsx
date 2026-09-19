import { useState, useCallback, useRef, useEffect } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import UserBlock from './UserBlock';
import ClaudeBlock, { isAssistantBlockVisible } from './ClaudeBlock';
import type { DisplayBlock } from '../../lib/messageProcessor';
import { isAutonomousOrchestrationTab, useTabStore } from '../../stores/tabStore';
import type { InputDelivery } from '../../lib/chatInput';
import { guidanceTranscript } from '../../lib/guidanceTranscript';

/** How many blocks to render initially (from the bottom) */
const INITIAL_VISIBLE = 20;
/** How many more blocks to load when scrolling to the top */
const LOAD_MORE = 20;

interface Props {
  tabId: string;
  readOnly?: boolean;
  blocks: DisplayBlock[];
  inputDeliveries?: InputDelivery[];
  onOpenPlan?: () => void;
  onEditRerun?: (blockIndex: number, newText: string) => void;
}

export default function BlockList({ tabId, blocks, inputDeliveries = [], onOpenPlan, onEditRerun, readOnly = false }: Props) {
  // Read isStreaming from tabStore — avoids O(n) blocks.some() scan on every render
  const isStreaming = useTabStore(s => s.tabs.find(t => t.id === tabId)?.isStreaming ?? false);
  const isOrchestrationOwned = useTabStore(s => isAutonomousOrchestrationTab(s.tabs.find(t => t.id === tabId)));
  // Number of blocks visible from the end
  const [visibleCount, setVisibleCount] = useState(INITIAL_VISIBLE);
  const sentinelRef = useRef<HTMLDivElement>(null);
  const observerRef = useRef<IntersectionObserver | null>(null);

  // Reset visible count when blocks shrink (e.g. new tab)
  useEffect(() => {
    if (blocks.length <= INITIAL_VISIBLE) setVisibleCount(INITIAL_VISIBLE);
  }, [blocks.length]);

  // Intersection observer: when the sentinel at the top becomes visible, load more
  const setupObserver = useCallback((node: HTMLDivElement | null) => {
    if (observerRef.current) observerRef.current.disconnect();
    if (!node) return;
    sentinelRef.current = node;

    observerRef.current = new IntersectionObserver(
      ([entry]) => {
        if (entry.isIntersecting) {
          setVisibleCount(prev => Math.min(prev + LOAD_MORE, blocks.length));
        }
      },
      { threshold: 0 }
    );
    observerRef.current.observe(node);
  }, [blocks.length]);

  if (blocks.length === 0 && !inputDeliveries.some(receipt => receipt.delivery === 'sent')) {
    return (
      <div className="flex items-center justify-center h-full">
        <p style={{ color: 'var(--text-dim)', fontSize: 13 }}>Start a conversation</p>
      </div>
    );
  }

  const startIdx = Math.max(0, blocks.length - visibleCount);
  // Invisible initialization/tool-only records must not consume a turn's header.
  // Group only the blocks actually on screen, including at a history-page boundary.
  const visibleBlocks = guidanceTranscript(blocks, inputDeliveries)
    .filter(({ globalIdx }) => globalIdx >= startIdx)
    .filter(({ block }) => block.type === 'user' ||
      ((block.type === 'assistant' || block.type === 'compact') && isAssistantBlockVisible(block)));
  const hasMore = startIdx > 0;

  return (
    <div className="flex flex-col py-4">
      {/* Sentinel + load-more hint at the top */}
      {hasMore && (
        <div
          ref={setupObserver}
          style={{
            padding: '8px 16px',
            textAlign: 'center',
            color: 'var(--text-dim)',
            fontSize: 11,
          }}
        >
          ↑ {startIdx} earlier message{startIdx !== 1 ? 's' : ''}
        </div>
      )}

      <AnimatePresence initial={false}>
        {visibleBlocks.map(({ block, globalIdx, guidance }, i) => {
          const previous = visibleBlocks[i - 1]?.block;
          const continuesResponse = previous?.type === 'assistant' &&
            !['end_turn', 'user_interrupted', 'max_tokens', 'error', 'process_died'].includes(previous.stopReason || '');
          return (
            <motion.div
              key={block.id}
              initial={isStreaming ? false : { y: 8, opacity: 0 }}
              animate={{ y: 0, opacity: 1 }}
              transition={{ type: 'spring', stiffness: 400, damping: 30 }}
            >
              {block.type === 'user' && (
                <UserBlock
                  block={block}
                  blockIndex={globalIdx}
                  onEditRerun={guidance || readOnly ? undefined : onEditRerun}
                  deliveryNote={guidance ? 'Accepted by current run' : undefined}
                />
              )}
              {(block.type === 'assistant' || block.type === 'compact') && (
                <ClaudeBlock
                  block={block}
                  onOpenPlan={onOpenPlan}
                  isContinuation={continuesResponse}
                />
              )}

            </motion.div>
          );
        })}
      </AnimatePresence>

      {isStreaming && (
        <div style={{ display: 'flex', alignItems: 'center', gap: 3, padding: '6px 20px 2px' }}>
          {[0, 0.18, 0.36].map((delay, i) => (
            <span
              key={i}
              style={{
                display: 'inline-block',
                width: 4, height: 4,
                borderRadius: '50%',
                background: 'var(--accent)',
                opacity: 0.3,
                animation: `dot-bounce 1.0s ease-in-out ${delay}s infinite`,
              }}
            />
          ))}
        </div>
      )}

      {!readOnly && !isOrchestrationOwned && !isStreaming && blocks[blocks.length - 1]?.stopReason === 'user_interrupted' && (
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '6px 20px 8px' }}>
          <span style={{
            fontSize: 11, fontFamily: 'var(--font-mono)',
            color: 'var(--text-dim)',
            background: 'var(--surface-2)',
            border: '1px solid var(--border)',
            borderRadius: 'var(--radius-sm)',
            padding: '2px 7px',
          }}>stopped</span>
          <button
            onClick={() => window.dispatchEvent(new CustomEvent('flamel:continue-response', { detail: { tabId } }))}
            style={{
              fontSize: 11, fontFamily: 'var(--font-mono)',
              color: 'var(--accent)',
              background: 'transparent',
              border: 'none', cursor: 'pointer', padding: 0,
              opacity: 0.8,
            }}
            onMouseEnter={e => { (e.currentTarget as HTMLElement).style.opacity = '1'; }}
            onMouseLeave={e => { (e.currentTarget as HTMLElement).style.opacity = '0.8'; }}
          >
            Continue →
          </button>
        </div>
      )}
    </div>
  );
}
