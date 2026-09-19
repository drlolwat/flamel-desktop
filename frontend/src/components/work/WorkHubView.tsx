import { useEffect, useMemo, useState } from 'react';
import type { WorkItem } from '@flamel/shared';
import { api } from '../../lib/api';
import { Icon } from '../ui/Icons';

const columns = [
  { id: 'backlog', label: 'Backlog' },
  { id: 'todo', label: 'Ready' },
  { id: 'in_progress', label: 'In progress' },
  { id: 'review', label: 'Review' },
  { id: 'done', label: 'Done' },
];

const button: React.CSSProperties = { minHeight: 30, padding: '0 10px', display: 'inline-flex', alignItems: 'center', gap: 6, border: '1px solid var(--border)', borderRadius: 'var(--radius-md)', background: 'var(--surface-2)', color: 'var(--text-muted)', fontSize: 11, fontWeight: 650 };

export default function WorkHubView({ projectPath, onClose }: { projectPath?: string; onClose: () => void }) {
  const [items, setItems] = useState<WorkItem[]>([]);
  const [query, setQuery] = useState('');
  const [draft, setDraft] = useState('');
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');

  async function load() {
    if (!projectPath) return;
    setLoading(true);
    try { setItems((await api.work.list(projectPath, query || undefined)).items); setError(''); }
    catch (e) { setError(e instanceof Error ? e.message : 'Could not load project work'); }
    finally { setLoading(false); }
  }
  useEffect(() => { const timer = setTimeout(() => void load(), 150); return () => clearTimeout(timer); }, [projectPath, query]);

  async function create() {
    if (!projectPath || !draft.trim()) return;
    try { const item = await api.work.create({ projectPath, title: draft.trim(), status: 'backlog' }); setItems(current => [...current, item]); setDraft(''); }
    catch (e) { setError(e instanceof Error ? e.message : 'Could not create work item'); }
  }
  async function move(item: WorkItem, status: string) {
    const previous = items; setItems(current => current.map(candidate => candidate.id === item.id ? { ...candidate, status } : candidate));
    try { const updated = await api.work.update(item.id, { status }); setItems(current => current.map(candidate => candidate.id === item.id ? updated : candidate)); }
    catch (e) { setItems(previous); setError(e instanceof Error ? e.message : 'Could not move work item'); }
  }
  const grouped = useMemo(() => Object.fromEntries(columns.map(column => [column.id, items.filter(item => item.status === column.id)])), [items]);

  return <div className="fixed inset-0 h-screen overflow-hidden" style={{ background: 'var(--bg)', zIndex: 9000, display: 'flex', flexDirection: 'column' }}>
    <header style={{ minHeight: 52, padding: '0 16px', display: 'flex', alignItems: 'center', gap: 12, borderBottom: '1px solid var(--border-soft)' }}>
      <button onClick={onClose} style={{ ...button, width: 30, padding: 0, justifyContent: 'center' }} title="Close work hub"><Icon.Close size={14} /></button>
      <Icon.Plan size={15} style={{ color: 'var(--accent)' }} />
      <div style={{ minWidth: 0 }}><strong style={{ fontSize: 13 }}>Engineering work</strong><p style={{ fontSize: 10, color: 'var(--text-dim)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{projectPath || 'Open a project to manage its work'}</p></div>
      <div style={{ marginLeft: 'auto', width: 'min(320px, 36vw)' }}><input value={query} onChange={event => setQuery(event.target.value)} placeholder="Search work" style={{ width: '100%', minHeight: 30, padding: '0 9px', border: '1px solid var(--border)', borderRadius: 'var(--radius-md)', background: 'var(--surface-2)', color: 'var(--text)', fontSize: 11 }} /></div>
    </header>
    {!projectPath ? <div style={{ flex: 1, display: 'grid', placeItems: 'center', color: 'var(--text-muted)', fontSize: 12 }}>Open a project session first.</div> : <>
      <div style={{ padding: '10px 16px', display: 'flex', gap: 8, borderBottom: '1px solid var(--border-soft)' }}><input value={draft} onChange={event => setDraft(event.target.value)} onKeyDown={event => { if (event.key === 'Enter') void create(); }} placeholder="Add work item" style={{ flex: 1, minHeight: 32, padding: '0 10px', border: '1px solid var(--border)', borderRadius: 'var(--radius-md)', background: 'var(--surface)', color: 'var(--text)', fontSize: 12 }} /><button onClick={() => void create()} disabled={!draft.trim()} style={{ ...button, background: 'var(--accent)', color: '#fff', borderColor: 'var(--accent)' }}><Icon.Plus size={12} /> Add</button></div>
      {error && <div style={{ padding: '7px 16px', color: 'var(--error)', fontSize: 11, borderBottom: '1px solid var(--border-soft)' }}>{error}</div>}
      <main style={{ flex: 1, overflow: 'auto', padding: 12 }}><div style={{ display: 'grid', gridTemplateColumns: 'repeat(5, minmax(220px, 1fr))', gap: 10, minWidth: 1160, height: '100%' }}>
        {columns.map(column => <section key={column.id} onDragOver={event => event.preventDefault()} onDrop={event => { const item = items.find(candidate => candidate.id === event.dataTransfer.getData('text/work-id')); if (item) void move(item, column.id); }} style={{ minHeight: 300, background: 'var(--surface)', borderLeft: '1px solid var(--border-soft)', borderRight: '1px solid var(--border-soft)' }}>
          <div style={{ height: 38, padding: '0 10px', display: 'flex', alignItems: 'center', borderTop: `2px solid ${column.id === 'in_progress' ? 'var(--accent)' : 'var(--border)'}`, borderBottom: '1px solid var(--border-soft)' }}><strong style={{ fontSize: 11 }}>{column.label}</strong><span style={{ marginLeft: 'auto', color: 'var(--text-dim)', fontSize: 10 }}>{grouped[column.id]?.length || 0}</span></div>
          <div style={{ padding: 7, display: 'flex', flexDirection: 'column', gap: 7 }}>{grouped[column.id]?.map(item => <article key={item.id} draggable onDragStart={event => event.dataTransfer.setData('text/work-id', item.id)} style={{ padding: 10, border: '1px solid var(--border)', borderRadius: 'var(--radius-md)', background: 'var(--surface-2)', cursor: 'grab' }}><div style={{ display: 'flex', gap: 7, alignItems: 'flex-start' }}><span style={{ marginTop: 4, width: 6, height: 6, borderRadius: '50%', background: item.priority === 'urgent' ? 'var(--error)' : item.priority === 'high' ? 'var(--warning)' : 'var(--text-dim)' }} /><strong style={{ flex: 1, fontSize: 11, lineHeight: 1.4 }}>{item.title}</strong></div>{item.description && <p style={{ marginTop: 6, fontSize: 10, lineHeight: 1.4, color: 'var(--text-muted)', display: '-webkit-box', WebkitLineClamp: 3, WebkitBoxOrient: 'vertical', overflow: 'hidden' }}>{item.description}</p>}<div style={{ marginTop: 8, display: 'flex', gap: 5, flexWrap: 'wrap', fontSize: 9, color: 'var(--text-dim)' }}>{item.milestone && <span>{item.milestone}</span>}{item.assigneeId && <span>{item.assigneeKind === 'agent' ? 'Agent' : 'Owner'}: {item.assigneeId}</span>}{item.releaseStatus && <span>{item.releaseStatus}</span>}</div></article>)}</div>
        </section>)}
      </div>{loading && <div style={{ position: 'fixed', right: 18, bottom: 18, color: 'var(--text-dim)', fontSize: 10 }}>Refreshing…</div>}</main>
    </>}
  </div>;
}
