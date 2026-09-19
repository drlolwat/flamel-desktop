import { useState, useRef } from 'react';
import { useContextMenu } from '../ui/ContextMenu';
import { Icon } from '../ui/Icons';
import type { DisplayBlock } from '../../lib/messageProcessor';

interface Props {
  block: DisplayBlock;
  blockIndex: number;
  onEditRerun?: (blockIndex: number, newText: string) => void;
  deliveryNote?: string;
}

export default function UserBlock({ block, blockIndex, onEditRerun, deliveryNote }: Props) {
  const [editing, setEditing] = useState(false);
  const [editValue, setEditValue] = useState('');
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const onContextMenu = useContextMenu(() => [
    {
      label: 'Copy message',
      icon: '⎘',
      onClick: () => navigator.clipboard.writeText(block.content || ''),
    },
    ...(onEditRerun ? [{
      label: 'Edit & re-run',
      icon: '✎',
      onClick: startEdit,
    }] : []),
  ]);

  function startEdit() {
    if (!onEditRerun) return;
    setEditValue(block.content || '');
    setEditing(true);
    setTimeout(() => {
      if (textareaRef.current) {
        textareaRef.current.focus();
        textareaRef.current.style.height = 'auto';
        textareaRef.current.style.height = textareaRef.current.scrollHeight + 'px';
        textareaRef.current.setSelectionRange(textareaRef.current.value.length, textareaRef.current.value.length);
      }
    }, 0);
  }

  function cancelEdit() {
    setEditing(false);
    setEditValue('');
  }

  function submitEdit() {
    const trimmed = editValue.trim();
    if (!trimmed || !onEditRerun) return;
    setEditing(false);
    onEditRerun(blockIndex, trimmed);
  }

  function handleKeyDown(e: React.KeyboardEvent<HTMLTextAreaElement>) {
    if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); submitEdit(); }
    if (e.key === 'Escape') cancelEdit();
  }

  return (
    <div
      onContextMenu={onContextMenu}
      style={{ padding: '6px 16px', position: 'relative' }}
      className="group"
    >
      {/* Label row */}
      <div className="flex items-center gap-1.5" style={{ marginBottom: 5 }}>
        <Icon.User size={11} style={{ color: 'var(--accent)', flexShrink: 0 }} />
        <span style={{ color: 'var(--accent)', fontSize: 12, fontWeight: 600, letterSpacing: '-0.01em' }}>You</span>
        {deliveryNote && <span style={{ color: 'var(--text-muted)', fontSize: 11, marginLeft: 4 }}>{deliveryNote}</span>}

        {/* Edit button — visible on hover */}
        {!editing && onEditRerun && (
          <button
            onClick={startEdit}
            title="Edit & re-run"
            className="opacity-0 group-hover:opacity-100"
            style={{
              marginLeft: 4,
              padding: '1px 5px',
              background: 'transparent',
              border: 'none',
              color: 'var(--text-dim)',
              cursor: 'pointer',
              fontSize: 10,
              fontFamily: 'var(--font-mono)',
              borderRadius: 'var(--radius-sm)',
              transition: 'opacity var(--transition-fast), color var(--transition-fast)',
            }}
            onMouseEnter={e => { (e.currentTarget as HTMLElement).style.color = 'var(--text-muted)'; }}
            onMouseLeave={e => { (e.currentTarget as HTMLElement).style.color = 'var(--text-dim)'; }}
          >
            edit
          </button>
        )}
      </div>

      {/* Images */}
      {block.images && block.images.length > 0 && (
        <div className="flex flex-wrap gap-2" style={{ marginBottom: 8 }}>
          {block.images.map((img, i) => (
            <img
              key={i}
              src={`data:${img.mediaType};base64,${img.data}`}
              alt={img.name || `image ${i + 1}`}
              style={{
                maxHeight: 160, maxWidth: 240,
                borderRadius: 'var(--radius-md)',
                objectFit: 'cover',
                border: '1px solid var(--border-soft)',
              }}
            />
          ))}
        </div>
      )}

      {editing ? (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
          <textarea
            ref={textareaRef}
            value={editValue}
            onChange={e => {
              setEditValue(e.target.value);
              e.target.style.height = 'auto';
              e.target.style.height = e.target.scrollHeight + 'px';
            }}
            onKeyDown={handleKeyDown}
            style={{
              width: '100%',
              background: 'var(--surface-2)',
              border: '1px solid var(--border-focus)',
              borderRadius: 'var(--radius-md)',
              color: 'var(--text)',
              fontSize: 13,
              lineHeight: 1.65,
              padding: '6px 10px',
              fontFamily: 'var(--font-sans)',
              resize: 'none',
              outline: 'none',
              minHeight: 40,
            }}
          />
          <div style={{ display: 'flex', gap: 6 }}>
            <button
              onClick={submitEdit}
              disabled={!editValue.trim()}
              style={{
                padding: '4px 12px', fontSize: 11, fontFamily: 'var(--font-sans)',
                background: editValue.trim() ? 'var(--accent)' : 'var(--surface-3)',
                color: editValue.trim() ? '#fff' : 'var(--text-dim)',
                border: 'none', borderRadius: 'var(--radius-md)',
                cursor: editValue.trim() ? 'pointer' : 'not-allowed',
              }}
            >
              Re-run ↵
            </button>
            <button
              onClick={cancelEdit}
              style={{
                padding: '4px 12px', fontSize: 11, fontFamily: 'var(--font-sans)',
                background: 'transparent', color: 'var(--text-dim)',
                border: '1px solid var(--border-soft)', borderRadius: 'var(--radius-md)',
                cursor: 'pointer',
              }}
            >
              Cancel
            </button>
            <span style={{ fontSize: 10, color: 'var(--text-dim)', alignSelf: 'center', fontFamily: 'var(--font-mono)' }}>
              context resets from here
            </span>
          </div>
        </div>
      ) : (
        <p style={{ color: 'var(--text)', fontSize: 13, lineHeight: 1.65, whiteSpace: 'pre-wrap', wordBreak: 'break-word', margin: 0 }}>
          {block.content}
        </p>
      )}
    </div>
  );
}
