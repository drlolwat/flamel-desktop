import { useState, useEffect, useRef, useCallback } from 'react';
import { useTodoStore } from '../../stores/todoStore';
import { api } from '../../lib/api';
import { Icon } from '../ui/Icons';
import type { TodoItem } from '@flamel/shared';

interface Props {
  projectPath: string;
}

export default function TodoChip({ projectPath }: Props) {
  const { todosByProject, setTodos, addTodo, updateTodo, removeTodo, clearDone } = useTodoStore();
  const todos: TodoItem[] = todosByProject[projectPath] || [];
  const [open, setOpen] = useState(false);
  const [inputVal, setInputVal] = useState('');
  const [loading, setLoading] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);

  const remaining = todos.filter(t => !t.done).length;
  const doneCount = todos.filter(t => t.done).length;

  // Load todos for this project
  useEffect(() => {
    if (!projectPath) return;
    api.todos.list(projectPath).then(items => setTodos(projectPath, items)).catch(() => {});
  }, [projectPath]);

  // Close on outside click
  useEffect(() => {
    if (!open) return;
    function handler(e: MouseEvent) {
      if (panelRef.current && !panelRef.current.contains(e.target as Node)) setOpen(false);
    }
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [open]);

  // Focus input when opened
  useEffect(() => {
    if (open) setTimeout(() => inputRef.current?.focus(), 50);
  }, [open]);

  const handleAdd = useCallback(async () => {
    const text = inputVal.trim();
    if (!text || loading) return;
    setLoading(true);
    try {
      const todo = await api.todos.create(text, projectPath);
      addTodo(projectPath, todo);
      setInputVal('');
    } catch {}
    setLoading(false);
    inputRef.current?.focus();
  }, [inputVal, loading, projectPath]);

  const handleToggle = useCallback(async (todo: TodoItem) => {
    const next = !todo.done;
    updateTodo(projectPath, todo.id, { done: next });
    try {
      await api.todos.update(todo.id, { done: next });
    } catch {
      updateTodo(projectPath, todo.id, { done: todo.done });
    }
  }, [projectPath]);

  const handleDelete = useCallback(async (id: string) => {
    removeTodo(projectPath, id);
    try { await api.todos.delete(id); } catch {}
  }, [projectPath]);

  const handleClearDone = useCallback(async () => {
    clearDone(projectPath);
    try { await api.todos.clearDone(projectPath); } catch {}
  }, [projectPath]);

  // Show nothing if no todos and panel is closed (just a subtle add link)
  const showChip = todos.length > 0 || open;

  return (
    <div style={{ position: 'relative' }} ref={panelRef}>
      {/* Chip / trigger */}
      <button
        onClick={() => setOpen(o => !o)}
        style={{
          display: 'inline-flex', alignItems: 'center', gap: 5,
          padding: '3px 8px 3px 6px',
          borderRadius: 'var(--radius-sm)',
          background: open ? 'var(--surface-2)' : 'transparent',
          border: `1px solid ${open || showChip ? 'var(--border-soft)' : 'transparent'}`,
          color: remaining > 0 ? 'var(--text-muted)' : 'var(--text-dim)',
          fontSize: 11, cursor: 'pointer',
          transition: 'all var(--transition-fast)',
        }}
        onMouseEnter={e => { e.currentTarget.style.background = 'var(--surface-2)'; e.currentTarget.style.borderColor = 'var(--border-soft)'; }}
        onMouseLeave={e => { if (!open) { e.currentTarget.style.background = 'transparent'; e.currentTarget.style.borderColor = showChip ? 'var(--border-soft)' : 'transparent'; } }}
        title="Todos"
      >
        <Icon.Check size={11} style={{ color: remaining > 0 ? 'var(--accent)' : 'var(--text-dim)', opacity: 0.8 }} />
        {showChip ? (
          <span>
            {remaining > 0 ? `${remaining} todo${remaining !== 1 ? 's' : ''}` : 'todos done'}
            {doneCount > 0 && remaining > 0 && (
              <span style={{ color: 'var(--text-dim)', marginLeft: 4 }}>· {doneCount} done</span>
            )}
          </span>
        ) : (
          <span style={{ color: 'var(--text-dim)' }}>add todos</span>
        )}
      </button>

      {/* Popover panel */}
      {open && (
        <div
          style={{
            position: 'absolute',
            bottom: 'calc(100% + 6px)',
            left: 0,
            width: 300,
            background: 'var(--surface)',
            border: '1px solid var(--border)',
            borderRadius: 'var(--radius-md)',
            boxShadow: '0 8px 24px rgba(0,0,0,0.25)',
            zIndex: 100,
            overflow: 'hidden',
          }}
        >
          {/* Add input */}
          <div style={{
            display: 'flex', alignItems: 'center', gap: 6,
            padding: '8px 10px',
            borderBottom: todos.length > 0 ? '1px solid var(--border-soft)' : 'none',
          }}>
            <input
              ref={inputRef}
              value={inputVal}
              onChange={e => setInputVal(e.target.value)}
              onKeyDown={e => { if (e.key === 'Enter') handleAdd(); if (e.key === 'Escape') setOpen(false); }}
              placeholder="Add a todo…"
              style={{
                flex: 1, background: 'transparent', border: 'none', outline: 'none',
                color: 'var(--text)', fontSize: 12,
                fontFamily: 'inherit',
              }}
            />
            <button
              onClick={handleAdd}
              disabled={!inputVal.trim() || loading}
              style={{
                display: 'flex', alignItems: 'center', justifyContent: 'center',
                width: 22, height: 22, borderRadius: 'var(--radius-sm)',
                background: inputVal.trim() ? 'var(--accent)' : 'var(--surface-2)',
                border: 'none', cursor: inputVal.trim() ? 'pointer' : 'default',
                color: inputVal.trim() ? '#fff' : 'var(--text-dim)',
                transition: 'all var(--transition-fast)',
                flexShrink: 0,
              }}
            >
              <Icon.Plus size={12} />
            </button>
          </div>

          {/* Todo list */}
          {todos.length > 0 && (
            <div style={{ maxHeight: 260, overflowY: 'auto' }}>
              {todos.map(todo => (
                <TodoRow
                  key={todo.id}
                  todo={todo}
                  onToggle={() => handleToggle(todo)}
                  onDelete={() => handleDelete(todo.id)}
                />
              ))}
            </div>
          )}

          {/* Footer */}
          {doneCount > 0 && (
            <div style={{
              padding: '6px 10px',
              borderTop: '1px solid var(--border-soft)',
              display: 'flex', justifyContent: 'flex-end',
            }}>
              <button
                onClick={handleClearDone}
                style={{
                  fontSize: 10, color: 'var(--text-dim)', background: 'none',
                  border: 'none', cursor: 'pointer', padding: '2px 4px',
                  borderRadius: 'var(--radius-sm)',
                  transition: 'color var(--transition-fast)',
                }}
                onMouseEnter={e => (e.currentTarget.style.color = 'var(--text)')}
                onMouseLeave={e => (e.currentTarget.style.color = 'var(--text-dim)')}
              >
                Clear {doneCount} done
              </button>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function TodoRow({ todo, onToggle, onDelete }: { todo: TodoItem; onToggle: () => void; onDelete: () => void }) {
  const [hovered, setHovered] = useState(false);

  return (
    <div
      style={{
        display: 'flex', alignItems: 'flex-start', gap: 8,
        padding: '6px 10px',
        background: hovered ? 'var(--surface-2)' : 'transparent',
        transition: 'background var(--transition-fast)',
      }}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
    >
      {/* Checkbox */}
      <button
        onClick={onToggle}
        style={{
          flexShrink: 0, marginTop: 1,
          width: 14, height: 14, borderRadius: 3,
          border: `1px solid ${todo.done ? 'var(--success)' : 'var(--border)'}`,
          background: todo.done ? 'var(--success-glow)' : 'transparent',
          cursor: 'pointer',
          display: 'flex', alignItems: 'center', justifyContent: 'center',
          transition: 'all var(--transition-fast)',
        }}
      >
        {todo.done && <Icon.Check size={9} style={{ color: 'var(--success)' }} />}
      </button>

      {/* Text */}
      <span style={{
        flex: 1, fontSize: 12, lineHeight: 1.45,
        color: todo.done ? 'var(--text-dim)' : 'var(--text)',
        textDecoration: todo.done ? 'line-through' : 'none',
        opacity: todo.done ? 0.6 : 1,
        wordBreak: 'break-word',
      }}>
        {todo.text}
      </span>

      {/* Delete */}
      {hovered && (
        <button
          onClick={onDelete}
          style={{
            flexShrink: 0, marginTop: 1,
            width: 14, height: 14,
            display: 'flex', alignItems: 'center', justifyContent: 'center',
            background: 'none', border: 'none', cursor: 'pointer',
            color: 'var(--text-dim)',
            transition: 'color var(--transition-fast)',
          }}
          onMouseEnter={e => (e.currentTarget.style.color = 'var(--error)')}
          onMouseLeave={e => (e.currentTarget.style.color = 'var(--text-dim)')}
        >
          <Icon.Close size={9} />
        </button>
      )}
    </div>
  );
}
