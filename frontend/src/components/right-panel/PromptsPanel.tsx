import { useState, useEffect } from 'react';
import { api } from '../../lib/api';
import type { SavedPrompt } from '@flamel/shared';

interface Props {
  onInsert?: (content: string) => void;
}

export default function PromptsPanel({ onInsert }: Props) {
  const [prompts, setPrompts] = useState<SavedPrompt[]>([]);
  const [adding, setAdding] = useState(false);
  const [newTitle, setNewTitle] = useState('');
  const [newContent, setNewContent] = useState('');
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    api.prompts.list().then(setPrompts).catch(() => {});
  }, []);

  async function handleSave() {
    if (!newTitle.trim() || !newContent.trim()) return;
    setSaving(true);
    try {
      const p = await api.prompts.create(newTitle.trim(), newContent.trim());
      setPrompts(prev => [p, ...prev]);
      setNewTitle('');
      setNewContent('');
      setAdding(false);
    } catch {}
    finally { setSaving(false); }
  }

  async function handleDelete(id: string) {
    await api.prompts.delete(id).catch(() => {});
    setPrompts(prev => prev.filter(p => p.id !== id));
  }

  async function handleUse(prompt: SavedPrompt) {
    await api.prompts.use(prompt.id).catch(() => {});
    onInsert?.(prompt.content);
    setPrompts(prev => prev.map(p => p.id === prompt.id ? { ...p, useCount: p.useCount + 1 } : p));
  }

  return (
    <div className="flex flex-col h-full overflow-hidden">
      <div className="flex-1 overflow-y-auto">
        {prompts.length === 0 && !adding ? (
          <div className="flex flex-col items-center justify-center h-40 gap-2">
            <p style={{ color: 'var(--text-muted)', fontSize: 12 }}>No saved prompts</p>
          </div>
        ) : (
          <div className="flex flex-col gap-1 p-2">
            {prompts.map(prompt => (
              <div
                key={prompt.id}
                className="rounded p-2 group"
                style={{ background: 'var(--surface-2)', border: '1px solid var(--border)' }}
              >
                <div className="flex items-start justify-between gap-2 mb-1">
                  <span style={{ color: 'var(--text)', fontSize: 12, fontWeight: 500 }}>{prompt.title}</span>
                  <div className="flex gap-1 opacity-0 group-hover:opacity-100">
                    <button
                      onClick={() => handleDelete(prompt.id)}
                      style={{ color: 'var(--error)', fontSize: 11 }}
                      title="Delete"
                    >
                      ×
                    </button>
                  </div>
                </div>
                <p className="truncate mb-2" style={{ color: 'var(--text-muted)', fontSize: 11 }}>
                  {prompt.content}
                </p>
                <button
                  onClick={() => handleUse(prompt)}
                  className="w-full py-1 rounded text-xs"
                  style={{ background: 'var(--surface)', color: 'var(--text-muted)', border: '1px solid var(--border)' }}
                >
                  Insert ↵
                </button>
              </div>
            ))}
          </div>
        )}

        {adding && (
          <div className="p-2 flex flex-col gap-2">
            <input
              placeholder="Title"
              value={newTitle}
              onChange={e => setNewTitle(e.target.value)}
              autoFocus
              className="w-full px-2 py-1.5 rounded text-xs outline-none"
              style={{ background: 'var(--surface-2)', border: '1px solid var(--border)', color: 'var(--text)' }}
            />
            <textarea
              placeholder="Prompt content..."
              value={newContent}
              onChange={e => setNewContent(e.target.value)}
              rows={4}
              className="w-full px-2 py-1.5 rounded text-xs outline-none resize-none"
              style={{ background: 'var(--surface-2)', border: '1px solid var(--border)', color: 'var(--text)', fontFamily: 'var(--font-sans)' }}
            />
            <div className="flex gap-2">
              <button
                onClick={() => { setAdding(false); setNewTitle(''); setNewContent(''); }}
                className="flex-1 py-1.5 rounded text-xs"
                style={{ background: 'var(--surface-2)', color: 'var(--text-muted)', border: '1px solid var(--border)' }}
              >
                Cancel
              </button>
              <button
                onClick={handleSave}
                disabled={saving || !newTitle.trim() || !newContent.trim()}
                className="flex-1 py-1.5 rounded text-xs disabled:opacity-40"
                style={{ background: 'var(--accent)', color: '#fff' }}
              >
                Save
              </button>
            </div>
          </div>
        )}
      </div>

      {!adding && (
        <div className="p-2 shrink-0" style={{ borderTop: '1px solid var(--border)' }}>
          <button
            onClick={() => setAdding(true)}
            className="w-full py-1.5 rounded text-xs"
            style={{ background: 'var(--surface-2)', color: 'var(--text-muted)', border: '1px solid var(--border)' }}
          >
            + New prompt
          </button>
        </div>
      )}
    </div>
  );
}
