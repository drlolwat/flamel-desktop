import { hasHostPanel } from '../../lib/workspaceHost';
import { prettifyPath } from "../../lib/paths";
import { useEffect, useRef, useState, useCallback } from 'react';
import { Icon } from '../ui/Icons';
import { isAutonomousOrchestrationTab, useTabStore } from '../../stores/tabStore';
import { useChatStore } from '../../stores/chatStore';
import type { GitAccount, Project, SessionMeta } from '@flamel/shared';
import { apiFetch, api } from '../../lib/api';
import { useConnectionStore } from '../../stores/connectionStore';

export interface GitRepo {
  id: string; name: string; fullName: string; description: string | null;
  cloneUrl: string; sshUrl: string; updatedAt: string; isPrivate: boolean;
  defaultBranch: string; language: string | null; stars: number;
}
export type GitRepoOpenHandler = (path: string, repo: GitRepo, account: GitAccount) => void;
interface FsEntry { name: string; path: string; type: 'dir' | 'file'; is_git: boolean; modified: number; }
interface DirListing { path: string; home: string; parent: string | null; entries: FsEntry[]; }
export interface RecentProject { path: string; sessionCount: number; lastActive: string | null; }

function timeAgo(iso: string) {
  if (!iso) return '';
  const ts = new Date(iso).getTime();
  if (isNaN(ts)) return '';
  const diff = Date.now() - ts;
  const m = Math.floor(diff / 60000);
  if (m < 1) return 'just now';
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h`;
  const d = Math.floor(h / 24);
  if (d < 30) return `${d}d`;
  const wk = Math.floor(d / 7);
  if (wk < 8) return `${wk}wk`;
  return new Date(iso).toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
}
function secsAgo(s: number) { return s ? timeAgo(new Date(s * 1000).toISOString()) : ''; }
export function folderName(p: string) { return p.split('/').filter(Boolean).pop() || p; }
export function parentPath(p: string) { const parts = p.split('/').filter(Boolean); return parts.length <= 1 ? '/' : '/' + parts.slice(0, -1).join('/'); }

const LANG: Record<string, string> = {
  TypeScript:'#3178c6', JavaScript:'#f1e05a', Python:'#3572A5', Go:'#00ADD8',
  Rust:'#dea584', Ruby:'#701516', Java:'#b07219', 'C++':'#f34b7d', Shell:'#89e051', Kotlin:'#A97BFF',
};

// ── Recent item ───────────────────────────────────────────────────────────────
export function RecentItem({ proj, onClick }: { proj: RecentProject; onClick: () => void }) {
  const [h, setH] = useState(false);
  return (
    <button onClick={onClick} onMouseEnter={() => setH(true)} onMouseLeave={() => setH(false)}
      style={{ display:'flex', flexDirection:'column', gap:3, width:'100%', padding:'9px 10px', borderRadius:7,
        textAlign:'left', background: h ? 'var(--surface-2)' : 'transparent',
        border:`1px solid ${h ? 'var(--accent-border)' : 'transparent'}`, cursor:'pointer', transition:'all 0.12s' }}>
      <div style={{ display:'flex', alignItems:'center', gap:7 }}>
        <Icon.Folder size={13} style={{ color: h ? 'var(--accent)' : 'var(--text-muted)', flexShrink:0 }} />
        <span style={{ fontSize:13, fontWeight:600, color:'var(--text)', overflow:'hidden', textOverflow:'ellipsis', whiteSpace:'nowrap' }}>
          {folderName(proj.path)}
        </span>
      </div>
      <span style={{ fontSize:11, color:'var(--text-muted)', overflow:'hidden', textOverflow:'ellipsis', whiteSpace:'nowrap', paddingLeft:20 }}>
        {prettifyPath(parentPath(proj.path))}
      </span>
      <div style={{ display:'flex', gap:6, alignItems:'center', paddingLeft:20 }}>
        {proj.sessionCount > 0 && (
          <span style={{ fontSize:10, padding:'1px 5px', borderRadius:4, background:'var(--accent-glow)', color:'var(--accent)', border:'1px solid var(--accent-border)' }}>
            {proj.sessionCount}s
          </span>
        )}
        {proj.lastActive && <span style={{ fontSize:10, color:'var(--text-muted)', opacity:0.7 }}>{timeAgo(proj.lastActive)}</span>}
      </div>
    </button>
  );
}

// ── Repo row ──────────────────────────────────────────────────────────────────
function RepoRow({ repo, account, onOpen, onRepoOpen }: { repo: GitRepo; account?: GitAccount; onOpen: (p: string) => void; onRepoOpen?: GitRepoOpenHandler }) {
  const [h, setH] = useState(false);
  const [cloning, setCloning] = useState(false);
  const [error, setError] = useState('');

  async function cloneAndOpen(e: React.MouseEvent) {
    e.stopPropagation();
    setCloning(true);
    setError('');
    try {
      const res = await apiFetch('/api/git/clone', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ cloneUrl: repo.cloneUrl, name: repo.name, repositoryId: repo.id }),
      });
      const data = await res.json();
      if (!res.ok) throw new Error(data.error || 'clone failed');
      if (account && onRepoOpen) onRepoOpen(data.path, repo, account);
      else onOpen(data.path);
    } catch (err: any) {
      setError(err.message);
    } finally {
      setCloning(false);
    }
  }

  return (
    <div onMouseEnter={() => setH(true)} onMouseLeave={() => setH(false)}
      style={{ borderRadius:7, overflow:'hidden', background: h ? 'var(--surface-2)' : 'transparent',
        border:`1px solid ${h ? 'var(--accent-border)' : 'transparent'}`, transition:'all 0.12s' }}>
      <div style={{ display:'flex', alignItems:'center', gap:9, padding:'8px 10px' }}>
        <span style={{ width:8, height:8, borderRadius:'50%', flexShrink:0, background: repo.language ? (LANG[repo.language]||'#888') : 'var(--border)' }} />
        <div style={{ flex:1, minWidth:0, display:'flex', alignItems:'center', gap:7, overflow:'hidden' }}>
          <span style={{ fontSize:13, fontWeight:600, color:'var(--text)', overflow:'hidden', textOverflow:'ellipsis', whiteSpace:'nowrap', flexShrink:0 }}>{repo.name}</span>
          {repo.isPrivate && <span style={{ fontSize:10, padding:'0 4px', borderRadius:3, background:'var(--surface-2)', color:'var(--text-muted)', border:'1px solid var(--border)', flexShrink:0 }}>private</span>}
          {repo.description && <span style={{ fontSize:11, color:'var(--text-muted)', overflow:'hidden', textOverflow:'ellipsis', whiteSpace:'nowrap', opacity:0.7 }}>— {repo.description}</span>}
        </div>
        <div style={{ display:'flex', alignItems:'center', gap:7, flexShrink:0 }}>
          {repo.language && !h && <span style={{ fontSize:11, color:'var(--text-muted)' }}>{repo.language}</span>}
          {repo.stars > 0 && <span style={{ fontSize:11, color:'var(--text-muted)' }}>★{repo.stars}</span>}
          <span style={{ fontSize:11, color:'var(--text-muted)', minWidth:40, textAlign:'right' }}>{timeAgo(repo.updatedAt)}</span>
          <button onClick={cloneAndOpen} disabled={cloning}
            style={{ padding:'3px 9px', borderRadius:5, fontSize:11, fontWeight:500,
              background: h ? 'var(--accent)' : 'transparent', color: h ? '#fff' : 'var(--text-muted)',
              border:`1px solid ${h ? 'var(--accent)' : 'var(--border)'}`, cursor: cloning ? 'wait' : 'pointer', transition:'all 0.12s',
              opacity: cloning ? 0.6 : 1 }}>
            {cloning ? 'Cloning…' : 'Open'}
          </button>
        </div>
      </div>
      {error && (
        <div style={{ padding:'2px 10px 8px', fontSize:11, color:'var(--error)' }}>{error}</div>
      )}
    </div>
  );
}

// ── Repositories tab ──────────────────────────────────────────────────────────
export function ReposPanel({ accounts, repos, loading: loadingMap, errors, onOpen, onRepoOpen }: {
  accounts: GitAccount[]; repos: Record<string, GitRepo[]>;
  loading: Record<string, boolean>; errors: Record<string, string>; onOpen: (p: string) => void; onRepoOpen?: GitRepoOpenHandler;
}) {
  const [selId, setSelId] = useState<string | null>(null);
  const [q, setQ] = useState('');
  const id = selId ?? accounts[0]?.id ?? null;
  const acct = accounts.find(a => a.id === id);
  const list = id ? (repos[id] ?? []) : [];
  const loading = id ? loadingMap[id] : false;
  const error = id ? errors[id] : null;
  const filtered = q.trim() ? list.filter(r => r.name.toLowerCase().includes(q.toLowerCase()) || r.description?.toLowerCase().includes(q.toLowerCase())) : list;

  if (!accounts.length) return (
    <div style={{ display:'flex', flexDirection:'column', alignItems:'center', justifyContent:'center', flex:1, gap:10, padding:40 }}>
      <Icon.Branch size={32} style={{ color:'var(--text-muted)', opacity:0.3 }} />
      <p style={{ fontSize:13, color:'var(--text-muted)', textAlign:'center', lineHeight:1.6 }}>
        No git accounts linked.<br/>Add one in <strong style={{ color:'var(--text)' }}>Settings → Git Accounts</strong>.
      </p>
    </div>
  );

  return (
    <div style={{ display:'flex', flexDirection:'column', flex:1, minHeight:0 }}>
      {/* Account pills */}
      <div style={{ display:'flex', gap:6, padding:'10px 12px', borderBottom:'1px solid var(--border)', flexWrap:'wrap', flexShrink:0 }}>
        {accounts.map(a => {
          const on = a.id === id;
          return (
            <button key={a.id} onClick={() => setSelId(a.id)}
              style={{ display:'flex', alignItems:'center', gap:6, padding:'4px 10px', borderRadius:99, fontSize:12, fontWeight: on ? 600 : 400,
                background: on ? 'var(--accent)' : 'var(--surface-2)', color: on ? '#fff' : 'var(--text-muted)',
                border:`1px solid ${on ? 'var(--accent)' : 'var(--border)'}`, cursor:'pointer', transition:'all 0.12s' }}>
              {a.avatarUrl
                ? <img src={a.avatarUrl} alt="" style={{ width:16, height:16, borderRadius:'50%' }} />
                : (a.type === 'github' ? <span style={{ fontSize:10 }}>⊞</span> : <Icon.Logo size={10} style={{ color: 'var(--accent)' }} />)}
              <span>{a.username ? `@${a.username}` : a.label}</span>
              <span style={{ fontSize:9, padding:'1px 4px', borderRadius:3,
                background: on ? 'rgba(255,255,255,0.2)' : 'var(--surface-3)',
                color: on ? 'rgba(255,255,255,0.85)' : 'var(--text-muted)',
                border:`1px solid ${on ? 'rgba(255,255,255,0.2)' : 'var(--border)'}` }}>
                {a.type === 'github' ? 'GH' : a.type === 'flamel' ? 'FG' : 'GT'}
              </span>
              {repos[a.id] !== undefined && <span style={{ fontSize:10, opacity:0.7 }}>{repos[a.id].length}</span>}
            </button>
          );
        })}
      </div>
      {/* Search */}
      <div style={{ padding:'8px 12px', borderBottom:'1px solid var(--border)', flexShrink:0 }}>
        <div style={{ display:'flex', alignItems:'center', gap:8, padding:'5px 10px', borderRadius:6, background:'var(--surface-2)', border:'1px solid var(--border)' }}>
          <svg width="12" height="12" viewBox="0 0 16 16" fill="none">
            <circle cx="7" cy="7" r="5" stroke="var(--text-muted)" strokeWidth="1.5"/>
            <path d="M11 11l3 3" stroke="var(--text-muted)" strokeWidth="1.5" strokeLinecap="round"/>
          </svg>
          <input value={q} onChange={e => setQ(e.target.value)}
            placeholder={`Filter ${acct ? (acct.username ? `@${acct.username}` : acct.label) : ''}…`}
            style={{ flex:1, background:'transparent', border:'none', outline:'none', fontSize:12, color:'var(--text)' }} />
          {q && <button onClick={() => setQ('')} style={{ background:'none', border:'none', color:'var(--text-muted)', cursor:'pointer', padding:0, fontSize:11 }}>✕</button>}
        </div>
      </div>
      {/* List */}
      <div style={{ flex:1, overflowY:'auto', padding:'6px 8px' }}>
        {loading && <div style={{ padding:20, textAlign:'center', color:'var(--text-muted)', fontSize:12 }}>Loading repositories…</div>}
        {error && <div style={{ padding:'10px 6px', color:'var(--error, #f87171)', fontSize:12 }}>Failed to load: {error}</div>}
        {!loading && !error && repos[id!] !== undefined && filtered.length === 0 && (
          <div style={{ padding:20, textAlign:'center', color:'var(--text-muted)', fontSize:12 }}>{q ? 'No matches.' : 'No repositories found.'}</div>
        )}
        {!loading && repos[id!] === undefined && !error && (
          <div style={{ padding:20, textAlign:'center', color:'var(--text-muted)', fontSize:12 }}>Loading…</div>
        )}
        {filtered.map(r => <RepoRow key={r.id} repo={r} account={acct ?? undefined} onOpen={onOpen} onRepoOpen={onRepoOpen} />)}
      </div>
    </div>
  );
}

// ── Browse tab ────────────────────────────────────────────────────────────────
export function BrowsePanel({ onOpen }: { onOpen: (p: string) => void }) {
  const [listing, setListing] = useState<DirListing | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [hov, setHov] = useState<string | null>(null);
  const [newFolderOpen, setNewFolderOpen] = useState(false);
  const [newFolderName, setNewFolderName] = useState('');
  const [newFolderError, setNewFolderError] = useState<string | null>(null);
  const newFolderRef = useRef<HTMLInputElement>(null);

  const loadAbortRef = useRef<AbortController | null>(null);
  const loadDir = useCallback((path?: string) => {
    loadAbortRef.current?.abort();
    const ac = new AbortController();
    loadAbortRef.current = ac;
    setLoading(true); setError(null);
    const url = path ? `/api/fs/dir?path=${encodeURIComponent(path)}` : '/api/fs/dir';
    apiFetch(url, { signal: ac.signal })
      .then(r => r.json())
      .then((d: DirListing & { error?: string }) => { if (d.error) throw new Error(d.error); setListing(d); })
      .catch((e: Error) => { if (e.name !== 'AbortError') setError(e.message); })
      .finally(() => setLoading(false));
  }, []);

  useEffect(() => { loadDir(); return () => loadAbortRef.current?.abort(); }, [loadDir]);

  function openNewFolder() {
    setNewFolderName('');
    setNewFolderError(null);
    setNewFolderOpen(true);
    setTimeout(() => newFolderRef.current?.focus(), 40);
  }

  function cancelNewFolder() {
    setNewFolderOpen(false);
    setNewFolderName('');
    setNewFolderError(null);
  }

  async function createFolder(e: React.FormEvent) {
    e.preventDefault();
    const name = newFolderName.trim();
    if (!name || !listing) return;
    if (name.includes('/') || name.includes('..')) {
      setNewFolderError('Invalid name');
      return;
    }
    const newPath = `${listing.path}/${name}`;
    setNewFolderError(null);
    const res = await apiFetch('/api/fs/mkdir', {
      method: 'POST', credentials: 'include',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ path: newPath }),
    });
    const data = await res.json();
    if (!res.ok || data.error) {
      setNewFolderError(data.error || 'Failed to create folder');
      return;
    }
    cancelNewFolder();
    onOpen(data.path);
  }

  const dirs = listing?.entries.filter(e => e.type === 'dir') ?? [];
  const files = listing?.entries.filter(e => e.type === 'file') ?? [];
  // Show path relative to user home
  const homePath = listing?.home ?? '';
  const relativePath = listing ? (listing.path.startsWith(homePath) ? listing.path.slice(homePath.length) : listing.path) : '';
  const parts = relativePath.split('/').filter(Boolean);
  const isAtHome = listing ? listing.path === homePath : true;

  return (
    <div style={{ display:'flex', flexDirection:'column', flex:1, minHeight:0 }}>
      {/* Breadcrumb */}
      <div style={{ display:'flex', alignItems:'center', gap:6, padding:'8px 12px', borderBottom: newFolderOpen ? 'none' : '1px solid var(--border)', flexShrink:0, overflow:'hidden' }}>
        {listing?.parent && (
          <button onClick={() => loadDir(listing.parent!)}
            style={{ display:'flex', alignItems:'center', gap:4, padding:'3px 8px', borderRadius:5, background:'var(--surface-2)', border:'1px solid var(--border)', color:'var(--text-muted)', fontSize:11, cursor:'pointer', flexShrink:0 }}>
            ← Back
          </button>
        )}
        <div style={{ display:'flex', alignItems:'center', gap:1, fontSize:12, overflow:'hidden', flexShrink:1, flex:1 }}>
          {/* Home root */}
          <button onClick={() => loadDir(homePath)}
            style={{ background:'none', border:'none', color: isAtHome ? 'var(--text)' : 'var(--text-muted)', cursor:'pointer', padding:'0 2px', fontSize:12, fontWeight: isAtHome ? 600 : 400 }}>
            ~
          </button>
          {parts.map((part, i) => {
            const seg = homePath + '/' + parts.slice(0, i + 1).join('/');
            const last = i === parts.length - 1;
            return (
              <span key={seg} style={{ display:'flex', alignItems:'center', flexShrink: last ? 0 : 1, overflow:'hidden' }}>
                <span style={{ color:'var(--border)', padding:'0 2px', flexShrink:0 }}>/</span>
                {last
                  ? <span style={{ color:'var(--text)', fontWeight:600, whiteSpace:'nowrap' }}>{part}</span>
                  : <button onClick={() => loadDir(seg)}
                      style={{ background:'none', border:'none', color:'var(--text-muted)', cursor:'pointer', padding:'0 2px', fontSize:12, overflow:'hidden', textOverflow:'ellipsis', whiteSpace:'nowrap' }}>
                      {part}
                    </button>
                }
              </span>
            );
          })}
        </div>
        {listing && !newFolderOpen && (
          <button onClick={openNewFolder}
            style={{ display:'flex', alignItems:'center', gap:5, padding:'3px 9px', borderRadius:5, fontSize:11, fontWeight:500,
              background:'transparent', color:'var(--text-muted)', border:'1px solid var(--border)', cursor:'pointer', flexShrink:0, transition:'all 0.12s' }}
            onMouseEnter={e => { const el = e.currentTarget as HTMLElement; el.style.borderColor='var(--border-focus)'; el.style.color='var(--text)'; }}
            onMouseLeave={e => { const el = e.currentTarget as HTMLElement; el.style.borderColor='var(--border)'; el.style.color='var(--text-muted)'; }}>
            <span style={{ fontSize:14, lineHeight:1 }}>+</span> New Folder
          </button>
        )}
      </div>
      {/* New Folder inline form */}
      {newFolderOpen && (
        <form onSubmit={createFolder}
          style={{ display:'flex', alignItems:'center', gap:6, padding:'6px 12px', borderBottom:'1px solid var(--border)', background:'var(--surface-2)', flexShrink:0 }}>
          <span style={{ fontSize:11, color:'var(--text-muted)', flexShrink:0 }}>New folder in <strong style={{ color:'var(--text)' }}>{isAtHome ? '~' : (listing?.path.split('/').pop() || '~')}</strong></span>
          <input ref={newFolderRef} value={newFolderName} onChange={e => { setNewFolderName(e.target.value); setNewFolderError(null); }}
            placeholder="folder-name" onKeyDown={e => e.key === 'Escape' && cancelNewFolder()}
            style={{ flex:1, padding:'4px 8px', borderRadius:5, fontSize:12, background:'var(--bg)', color:'var(--text)',
              border:`1px solid ${newFolderError ? 'var(--error, #f87171)' : 'var(--accent)'}`, outline:'none' }} />
          {newFolderError && <span style={{ fontSize:11, color:'var(--error, #f87171)', flexShrink:0 }}>{newFolderError}</span>}
          <button type="submit" disabled={!newFolderName.trim()}
            style={{ padding:'4px 10px', borderRadius:5, fontSize:11, fontWeight:500,
              background: newFolderName.trim() ? 'var(--accent)' : 'var(--surface-2)',
              color: newFolderName.trim() ? '#fff' : 'var(--text-muted)', border:'none', cursor: newFolderName.trim() ? 'pointer' : 'not-allowed' }}>
            Create &amp; Open
          </button>
          <button type="button" onClick={cancelNewFolder}
            style={{ padding:'4px 8px', borderRadius:5, fontSize:11, background:'transparent', color:'var(--text-muted)', border:'1px solid var(--border)', cursor:'pointer' }}>
            ✕
          </button>
        </form>
      )}
      {/* Entries */}
      <div style={{ flex:1, overflowY:'auto', padding:'4px 8px' }}>
        {loading && <div style={{ padding:20, textAlign:'center', color:'var(--text-muted)', fontSize:12 }}>Loading…</div>}
        {error && <div style={{ padding:12, color:'var(--error, #f87171)', fontSize:12 }}>{error}</div>}
        {!loading && !error && listing && (
          <>
            {dirs.map(e => (
              <div key={e.path} onMouseEnter={() => setHov(e.path)} onMouseLeave={() => setHov(null)}
                onClick={() => loadDir(e.path)}
                style={{ display:'flex', alignItems:'center', gap:9, padding:'6px 10px', borderRadius:6, cursor:'pointer',
                  background: hov === e.path ? 'var(--surface-2)' : 'transparent',
                  border:`1px solid ${hov === e.path ? 'var(--accent-border)' : 'transparent'}`, transition:'all 0.1s' }}>
                <Icon.Folder size={14} style={{ color: e.is_git ? 'var(--accent)' : 'var(--text-muted)', flexShrink:0 }} />
                <span style={{ flex:1, fontSize:13, color:'var(--text)', overflow:'hidden', textOverflow:'ellipsis', whiteSpace:'nowrap' }}>{e.name}</span>
                {e.is_git && <span style={{ fontSize:10, padding:'1px 5px', borderRadius:3, background:'var(--accent-glow)', color:'var(--accent)', border:'1px solid var(--accent-border)', flexShrink:0 }}>git</span>}
                {hov !== e.path && e.modified > 0 && <span style={{ fontSize:11, color:'var(--text-muted)', flexShrink:0, minWidth:40, textAlign:'right' }}>{secsAgo(e.modified)}</span>}
                {hov === e.path && (
                  <button onClick={ev => { ev.stopPropagation(); onOpen(e.path); }}
                    style={{ padding:'3px 9px', borderRadius:5, fontSize:11, fontWeight:500, background:'var(--accent)', color:'#fff', border:'none', cursor:'pointer', flexShrink:0 }}>
                    Open
                  </button>
                )}
              </div>
            ))}
            {dirs.length > 0 && files.length > 0 && <div style={{ height:1, background:'var(--border)', margin:'4px 2px' }} />}
            {files.map(e => (
              <div key={e.path}
                style={{ display:'flex', alignItems:'center', gap:9, padding:'5px 10px', borderRadius:6, opacity:0.5 }}>
                <svg width="13" height="13" viewBox="0 0 16 16" fill="none" style={{ flexShrink:0 }}>
                  <path d="M3 2h7l4 4v9H3V2z" stroke="var(--text-muted)" strokeWidth="1.2"/>
                  <path d="M10 2v4h4" stroke="var(--text-muted)" strokeWidth="1.2"/>
                </svg>
                <span style={{ flex:1, fontSize:12, color:'var(--text-muted)', overflow:'hidden', textOverflow:'ellipsis', whiteSpace:'nowrap' }}>{e.name}</span>
              </div>
            ))}
            {dirs.length === 0 && files.length === 0 && <div style={{ padding:20, textAlign:'center', color:'var(--text-muted)', fontSize:12 }}>Empty directory.</div>}
          </>
        )}
      </div>
    </div>
  );
}

// ── Runner type ──────────────────────────────────────────────────────────────
interface Runner {
  id: string; name: string; hostname: string; status: string;
  last_heartbeat: string | null; notes: string | null;
}

// ── Runners tab ──────────────────────────────────────────────────────────────
function RunnersPanel({ onSelect, selectedRunnerId }: { onSelect: (runner: Runner | null) => void; selectedRunnerId?: string }) {
  const [runners, setRunners] = useState<Runner[]>([]);
  const [loading, setLoading] = useState(true);
  const [hov, setHov] = useState<string | null>(null);

  useEffect(() => {
    api.runners.list()
      .then(setRunners)
      .catch(() => {})
      .finally(() => setLoading(false));
  }, []);

  const online = runners.filter(r => r.status === 'online');
  const offline = runners.filter(r => r.status !== 'online');

  if (loading) return (
    <div style={{ padding: 20, textAlign: 'center', color: 'var(--text-muted)', fontSize: 12 }}>Loading Workers…</div>
  );

  if (runners.length === 0) return (
    <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', flex: 1, gap: 10, padding: 40 }}>
      <Icon.Server size={32} style={{ color: 'var(--text-muted)', opacity: 0.3 }} />
      <p style={{ fontSize: 13, color: 'var(--text-muted)', textAlign: 'center', lineHeight: 1.6 }}>
        No Workers configured.<br />Add one in <strong style={{ color: 'var(--text)' }}>Settings → Workers</strong>.
      </p>
    </div>
  );

  function RunnerRow({ r }: { r: Runner }) {
    const isOnline = r.status === 'online';
    const selected = r.id === selectedRunnerId;
    const hovering = hov === r.id;
    return (
      <div
        onMouseEnter={() => setHov(r.id)} onMouseLeave={() => setHov(null)}
        onClick={() => isOnline && onSelect(selected ? null : r)}
        style={{
          display: 'flex', alignItems: 'center', gap: 10, padding: '8px 12px', borderRadius: 7,
          cursor: isOnline ? 'pointer' : 'default', opacity: isOnline ? 1 : 0.5,
          background: selected ? 'var(--accent-glow)' : hovering && isOnline ? 'var(--surface-2)' : 'transparent',
          border: `1px solid ${selected ? 'var(--accent-border)' : hovering && isOnline ? 'var(--accent-border)' : 'transparent'}`,
          transition: 'all 0.12s',
        }}
      >
        {/* Status dot */}
        <span style={{
          width: 8, height: 8, borderRadius: '50%', flexShrink: 0,
          background: isOnline ? 'var(--success, #22c55e)' : 'var(--text-dim)',
        }} />
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 7 }}>
            <span style={{ fontSize: 13, fontWeight: 600, color: 'var(--text)' }}>{r.name}</span>
            {selected && (
              <span style={{ fontSize: 10, padding: '0 5px', borderRadius: 3, background: 'var(--accent-glow-strong)', color: 'var(--accent)', border: '1px solid var(--accent-border)' }}>
                selected
              </span>
            )}
          </div>
          <span style={{ fontSize: 11, color: 'var(--text-muted)' }}>{r.hostname || 'no hostname'}</span>
        </div>
        {r.last_heartbeat && (
          <span style={{ fontSize: 10, color: 'var(--text-muted)', flexShrink: 0 }}>{timeAgo(r.last_heartbeat)}</span>
        )}
      </div>
    );
  }

  return (
    <div style={{ flex: 1, overflowY: 'auto', padding: '8px' }}>
      {selectedRunnerId && (
        <div style={{ padding: '6px 12px', marginBottom: 6, display: 'flex', alignItems: 'center', gap: 8, borderRadius: 6, background: 'var(--accent-glow)', border: '1px solid var(--accent-border)' }}>
          <Icon.Server size={11} style={{ color: 'var(--accent)', flexShrink: 0 }} />
          <span style={{ fontSize: 11, color: 'var(--text-muted)', flex: 1 }}>
            Sessions will run on <strong style={{ color: 'var(--text)' }}>{runners.find(r => r.id === selectedRunnerId)?.name}</strong>
          </span>
          <button onClick={() => onSelect(null)} style={{ fontSize: 10, padding: '1px 6px', borderRadius: 4, background: 'transparent', color: 'var(--text-muted)', border: '1px solid var(--border)', cursor: 'pointer' }}>
            Clear
          </button>
        </div>
      )}
      {online.length > 0 && (
        <>
          <div style={{ padding: '4px 8px', fontSize: 10, fontWeight: 700, color: 'var(--text-muted)', letterSpacing: '0.1em', textTransform: 'uppercase' }}>Online</div>
          {online.map(r => <RunnerRow key={r.id} r={r} />)}
        </>
      )}
      {offline.length > 0 && (
        <>
          <div style={{ padding: '4px 8px', marginTop: online.length > 0 ? 8 : 0, fontSize: 10, fontWeight: 700, color: 'var(--text-muted)', letterSpacing: '0.1em', textTransform: 'uppercase' }}>Offline</div>
          {offline.map(r => <RunnerRow key={r.id} r={r} />)}
        </>
      )}
    </div>
  );
}

// ── Main ──────────────────────────────────────────────────────────────────────
export default function NewSessionScreen({ tabId }: { tabId?: string } = {}) {
  const { addTab, updateTab } = useTabStore();
  const [projects, setProjects] = useState<Project[]>([]);
  const [sessions, setSessions] = useState<SessionMeta[]>([]);
  const [accounts, setAccounts] = useState<GitAccount[]>([]);
  const [repos, setRepos] = useState<Record<string, GitRepo[]>>({});
  const [reposLoading, setReposLoading] = useState<Record<string, boolean>>({});
  const [reposError, setReposError] = useState<Record<string, string>>({});
  const [rightTab, setRightTab] = useState<'repos' | 'browse' | 'runners'>('browse');
  const [pathInput, setPathInput] = useState('');
  const pathRef = useRef<HTMLInputElement>(null);
  const [selectedRunner, setSelectedRunner] = useState<{ id: string; name: string } | null>(null);
  const connectionMode = useConnectionStore(s => s.mode);
  const isLocalConnection = !hasHostPanel('runners') || (import.meta.env.VITE_MODE === 'local' && connectionMode !== 'remote');

  useEffect(() => {
    const ac = new AbortController();
    Promise.all([
      apiFetch('/api/projects', { credentials:'include', signal: ac.signal }).then(r => r.json()).catch(() => []),
      apiFetch('/api/sessions', { credentials:'include', signal: ac.signal }).then(r => r.json()).catch(() => []),
      apiFetch('/api/git-accounts', { credentials:'include', signal: ac.signal }).then(r => r.json()).catch(() => []),
      apiFetch('/api/flamel-git/account', { credentials:'include', signal: ac.signal }).then(r => r.json()).catch(() => null),
    ]).then(([p, s, a, flamelAccount]) => {
      setProjects(p || []); setSessions(s || []);
      setAccounts(flamelAccount ? [flamelAccount, ...(a || [])] : (a || []));
    }).catch(() => {});
    return () => ac.abort();
  }, []);

  // Lazy: only fetch repos when the repos panel is actually visible.
  // Eager fetching held connection slots (libsoup2 allows only 2 per host),
  // blocking the chat request when the user switched back to a project tab.
  const reposAbortRef = useRef<AbortController | null>(null);
  useEffect(() => {
    if (rightTab !== 'repos' || accounts.length === 0) return;
    const ac = new AbortController();
    reposAbortRef.current = ac;
    for (const acct of accounts) {
      if (repos[acct.id] !== undefined || reposLoading[acct.id]) continue;
      setReposLoading(prev => ({ ...prev, [acct.id]: true }));
      apiFetch(`/api/git-accounts/${acct.id}/repos`, { credentials:'include', signal: ac.signal })
        .then(r => r.json().then(data => {
          if (!r.ok) throw new Error(data?.error || `HTTP ${r.status}`);
          setRepos(prev => ({ ...prev, [acct.id]: data || [] }));
        }))
        .catch(err => {
          if (err?.name === 'AbortError') return;
          setRepos(prev => ({ ...prev, [acct.id]: [] }));
          setReposError(prev => ({ ...prev, [acct.id]: err?.message || 'Failed to load' }));
        })
        .finally(() => setReposLoading(prev => ({ ...prev, [acct.id]: false })));
    }
    return () => ac.abort();
  }, [accounts, rightTab]);

  const projectMap = new Map<string, { sessionCount: number; lastActive: string }>();
  for (const s of sessions) {
    if (isAutonomousOrchestrationTab(s)) continue;
    const ex = projectMap.get(s.projectPath);
    if (!ex) projectMap.set(s.projectPath, { sessionCount:1, lastActive: s.updatedAt });
    else { ex.sessionCount++; if (s.updatedAt > ex.lastActive) ex.lastActive = s.updatedAt; }
  }
  const allPaths = new Set([...projectMap.keys(), ...projects.map(p => p.path)]);
  const projectList: RecentProject[] = Array.from(allPaths)
    .map(path => ({ path, sessionCount: projectMap.get(path)?.sessionCount ?? 0, lastActive: projectMap.get(path)?.lastActive ?? null }))
    .sort((a, b) => {
      if (a.lastActive && b.lastActive) return b.lastActive.localeCompare(a.lastActive);
      if (a.lastActive) return -1; if (b.lastActive) return 1; return 0;
    }).slice(0, 20);

  function openPath(raw: string) {
    const p = raw.trim();
    if (!p) return;
    const runnerFields = !isLocalConnection && selectedRunner ? { runnerId: selectedRunner.id, runnerName: selectedRunner.name } : {};
    if (tabId) {
      // A project chooser always starts a fresh conversation. Clearing both
      // stores prevents a stale runtime session from being attached to the new
      // project's first message after reconnect or multi-device tab restore.
      useChatStore.getState().clearChat(tabId);
      updateTab(tabId, {
        kind: 'chat',
        sessionId: undefined,
        activePlanId: undefined,
        projectPath: p,
        workspace: undefined,
        title: p.split('/').pop() || p,
        ...runnerFields,
      });
    } else {
      addTab({ kind: 'chat', projectPath: p, ...runnerFields });
    }
  }

  const tabBtn = (tab: 'repos' | 'browse' | 'runners', label: string, badge?: string) => {
    const on = rightTab === tab;
    return (
      <button onClick={() => setRightTab(tab)}
        style={{ padding:'5px 13px', borderRadius:6, fontSize:12, fontWeight: on ? 600 : 400,
          background: on ? 'var(--surface-2)' : 'transparent', color: on ? 'var(--text)' : 'var(--text-muted)',
          border:`1px solid ${on ? 'var(--border)' : 'transparent'}`, cursor:'pointer', transition:'all 0.12s',
          display:'flex', alignItems:'center', gap:5 }}>
        {label}
        {badge && <span style={{ fontSize:9, padding:'0 4px', borderRadius:3, background:'rgba(34,197,94,0.1)', color:'var(--success, #22c55e)', border:'1px solid rgba(34,197,94,0.2)' }}>{badge}</span>}
      </button>
    );
  };

  return (
    <div style={{ height:'100%', display:'flex', flexDirection:'column', background:'var(--bg, #0f0f11)', overflow:'hidden' }}>
      {/* Top bar */}
      <div style={{ flexShrink:0, padding:'12px 16px', borderBottom:'1px solid var(--border)', display:'flex', alignItems:'center', gap:14 }}>
        <form onSubmit={e => { e.preventDefault(); openPath(pathInput); }} style={{ flex:1, maxWidth:440 }}>
          <div style={{ display:'flex', alignItems:'center', gap:8, height:30, padding:'0 10px', borderRadius:6, background:'var(--surface)', border:'1px solid var(--border)' }}>
            <Icon.Folder size={12} style={{ color:'var(--text-muted)', flexShrink:0 }} />
            <input ref={pathRef} value={pathInput} onChange={e => setPathInput(e.target.value)}
              placeholder="Open path…  ~/code/my-project"
              style={{ flex:1, background:'transparent', border:'none', outline:'none', fontSize:12, color:'var(--text)' }} />
            {pathInput && (
              <button type="submit"
                style={{ padding:'2px 8px', borderRadius:4, fontSize:11, fontWeight:500, background:'var(--accent)', color:'#fff', border:'none', cursor:'pointer', flexShrink:0 }}>
                Open →
              </button>
            )}
          </div>
        </form>
      </div>

      {/* Body */}
      <div role="note" style={{ padding: '6px 16px', fontSize: 11, color: 'var(--text-muted)', borderBottom: '1px solid var(--border-soft)' }}>
        If another chat is working in this repository, your new chat gets a separate workspace from its latest saved version. Uncommitted changes and ignored files stay in the original folder.
      </div>
      <div style={{ flex:1, display:'flex', overflow:'hidden' }}>
        {/* Left: Recents */}
        <div style={{ width:252, flexShrink:0, borderRight:'1px solid var(--border)', display:'flex', flexDirection:'column', overflow:'hidden' }}>
          <div style={{ padding:'10px 14px 6px', flexShrink:0 }}>
            <span style={{ fontSize:10, fontWeight:700, color:'var(--text-muted)', letterSpacing:'0.1em', textTransform:'uppercase' }}>Recents</span>
          </div>
          <div style={{ flex:1, overflowY:'auto', padding:'0 5px 8px' }}>
            {projectList.length === 0
              ? <p style={{ fontSize:12, color:'var(--text-muted)', padding:'8px 10px', lineHeight:1.5, opacity:0.7 }}>No recent projects. Open a path above or browse files →</p>
              : projectList.map(p => <RecentItem key={p.path} proj={p} onClick={() => openPath(p.path)} />)
            }
          </div>
          <div style={{ flexShrink:0, padding:'6px 8px', borderTop:'1px solid var(--border)', display:'flex', flexDirection:'column', gap:6 }}>
            <button onClick={() => setRightTab('browse')}
              style={{ width:'100%', padding:'6px 10px', borderRadius:6, fontSize:12, background:'transparent', color:'var(--text-muted)',
                border:'1px solid var(--border)', cursor:'pointer', textAlign:'left', display:'flex', alignItems:'center', gap:7 }}
              onMouseEnter={e => { const el = e.currentTarget as HTMLElement; el.style.borderColor='var(--border-focus)'; el.style.color='var(--text)'; }}
              onMouseLeave={e => { const el = e.currentTarget as HTMLElement; el.style.borderColor='var(--border)'; el.style.color='var(--text-muted)'; }}>
              <Icon.Folder size={12} style={{ flexShrink:0 }} />
              Browse files…
            </button>
          </div>
        </div>

        {/* Right: tabbed */}
        <div style={{ flex:1, display:'flex', flexDirection:'column', overflow:'hidden', minWidth:0 }}>
          <div style={{ flexShrink:0, display:'flex', alignItems:'center', gap:4, padding:'6px 10px', borderBottom:'1px solid var(--border)' }}>
            {tabBtn('repos', 'Repositories')}
            {tabBtn('browse', 'Files')}
            {!isLocalConnection && tabBtn('runners', 'Workers', selectedRunner ? selectedRunner.name : undefined)}
          </div>
          <div style={{ flex:1, overflow:'hidden', display:'flex', flexDirection:'column' }}>
            {rightTab === 'repos' && (
              <ReposPanel accounts={accounts} repos={repos} loading={reposLoading} errors={reposError} onOpen={openPath} />
            )}
            {rightTab === 'browse' && <BrowsePanel onOpen={openPath} />}
            {!isLocalConnection && rightTab === 'runners' && (
              <RunnersPanel
                selectedRunnerId={selectedRunner?.id}
                onSelect={(r) => setSelectedRunner(r ? { id: r.id, name: r.name } : null)}
              />
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
