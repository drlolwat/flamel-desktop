import { useEffect, useMemo, useRef, useState, type KeyboardEvent, type ComponentType } from 'react';
import {
  ChevronRight, ChevronDown,
  Folder, FolderOpen,
  FileCode, FileText, FileJson, File, Image, Terminal,
  Settings, Database, Package, GitBranch, Lock,
  RefreshCw, FolderRoot, Search, X, Eye, EyeOff, Copy, ExternalLink,
} from 'lucide-react';
import type { FileNode, GitStatus } from '@flamel/shared';
import {
  collectFiles,
  createFileReference,
  filterFileTree,
  gitStatusByPath,
  relativeFilePath,
  sortFileNodes,
  type FileSortMode,
} from '../../stores/projectFileStore';

interface FileIconSpec { icon: ComponentType<any>; color: string }

function getFileSpec(name: string): FileIconSpec {
  const ext = name.split('.').pop()?.toLowerCase() ?? '';
  const lower = name.toLowerCase();

  if (lower === 'dockerfile') return { icon: FileCode, color: '#2496ed' };
  if (lower === 'makefile') return { icon: Settings, color: '#6d8086' };
  if (['package.json', 'package-lock.json', 'bun.lock', 'yarn.lock', 'pnpm-lock.yaml'].includes(lower))
    return { icon: Package, color: '#cb3837' };
  if (['.gitignore', '.gitattributes'].includes(lower)) return { icon: GitBranch, color: '#f54d27' };
  if (lower === '.env' || lower.startsWith('.env.')) return { icon: Lock, color: '#ecd53f' };

  if (['ts'].includes(ext)) return { icon: FileCode, color: '#3178c6' };
  if (['tsx'].includes(ext)) return { icon: FileCode, color: '#61dafb' };
  if (['js', 'mjs', 'cjs'].includes(ext)) return { icon: FileCode, color: '#f7df1e' };
  if (['jsx'].includes(ext)) return { icon: FileCode, color: '#61dafb' };
  if (['json', 'jsonc', 'json5'].includes(ext)) return { icon: FileJson, color: '#cbcb41' };
  if (['css'].includes(ext)) return { icon: FileCode, color: '#42a5f5' };
  if (['scss', 'sass', 'less'].includes(ext)) return { icon: FileCode, color: '#cd6799' };
  if (['html', 'htm'].includes(ext)) return { icon: FileCode, color: '#e44d26' };
  if (['md', 'mdx'].includes(ext)) return { icon: FileText, color: '#519aba' };
  if (['py'].includes(ext)) return { icon: FileCode, color: '#3572a5' };
  if (['rs'].includes(ext)) return { icon: FileCode, color: '#dea584' };
  if (['go'].includes(ext)) return { icon: FileCode, color: '#00add8' };
  if (['java'].includes(ext)) return { icon: FileCode, color: '#b07219' };
  if (['kt', 'kts'].includes(ext)) return { icon: FileCode, color: '#a97bff' };
  if (['sh', 'bash', 'zsh', 'fish'].includes(ext)) return { icon: Terminal, color: '#89e051' };
  if (['yml', 'yaml'].includes(ext)) return { icon: Settings, color: '#cb171e' };
  if (['toml'].includes(ext)) return { icon: Settings, color: '#9c4221' };
  if (['sql'].includes(ext)) return { icon: Database, color: '#dad8d8' };
  if (['graphql', 'gql'].includes(ext)) return { icon: FileCode, color: '#e10098' };
  if (['svg'].includes(ext)) return { icon: Image, color: '#ffb13b' };
  if (['png', 'jpg', 'jpeg', 'gif', 'webp', 'ico', 'bmp'].includes(ext)) return { icon: Image, color: '#a074c4' };
  if (['vue'].includes(ext)) return { icon: FileCode, color: '#42b883' };
  if (['svelte'].includes(ext)) return { icon: FileCode, color: '#ff3e00' };
  if (['lock'].includes(ext)) return { icon: Lock, color: '#6b737c' };
  if (['gradle'].includes(ext)) return { icon: Settings, color: '#02303a' };
  if (['xml', 'plist'].includes(ext)) return { icon: FileCode, color: '#e37933' };
  if (['c', 'h'].includes(ext)) return { icon: FileCode, color: '#555597' };
  if (['cpp', 'cc', 'cxx', 'hpp'].includes(ext)) return { icon: FileCode, color: '#9c33a3' };
  if (['cs'].includes(ext)) return { icon: FileCode, color: '#178600' };
  if (['rb'].includes(ext)) return { icon: FileCode, color: '#701516' };
  if (['php'].includes(ext)) return { icon: FileCode, color: '#4f5d95' };
  if (['swift'].includes(ext)) return { icon: FileCode, color: '#fa7343' };
  if (['dart'].includes(ext)) return { icon: FileCode, color: '#00b4ab' };
  if (['txt'].includes(ext)) return { icon: FileText, color: '#8b949e' };

  return { icon: File, color: '#8b8b8b' };
}

function statusColor(status?: string): string {
  if (status === 'A') return 'var(--success)';
  if (status === 'D') return 'var(--error)';
  if (status === '?') return 'var(--accent)';
  if (status === 'M') return 'var(--warning)';
  return 'var(--text-dim)';
}

function quickScore(projectPath: string, node: FileNode, query: string): number {
  const q = query.trim().toLowerCase();
  if (!q) return 999;
  const name = node.name.toLowerCase();
  const relative = relativeFilePath(projectPath, node.path).toLowerCase();
  if (name === q) return 0;
  if (name.startsWith(q)) return 1;
  if (relative.startsWith(q)) return 2;
  if (name.includes(q)) return 3;
  if (relative.includes(q)) return 4;
  return 999;
}

interface FileRowProps {
  projectPath: string;
  node: FileNode;
  depth: number;
  activeFilePath: string | null;
  selectedPath: string | null;
  expandedPaths: Set<string>;
  gitByPath: Record<string, { status: string; added?: number; removed?: number }>;
  query: string;
  sortMode: FileSortMode;
  onOpenFile: (path: string) => void;
  onToggleExpanded: (path: string) => void;
  onSetExpanded: (path: string, expanded: boolean) => void;
  onSelectPath: (path: string) => void;
  onContextMenu: (node: FileNode, x: number, y: number) => void;
}

function FileRow({
  projectPath,
  node,
  depth,
  activeFilePath,
  selectedPath,
  expandedPaths,
  gitByPath,
  query,
  sortMode,
  onOpenFile,
  onToggleExpanded,
  onSetExpanded,
  onSelectPath,
  onContextMenu,
}: FileRowProps) {
  const isDir = node.type === 'directory';
  const isForcedOpen = Boolean(query.trim());
  const isOpen = isDir && (isForcedOpen || expandedPaths.has(node.path));
  const isActive = !isDir && node.path === activeFilePath;
  const isSelected = node.path === selectedPath;
  const relativePath = relativeFilePath(projectPath, node.path);
  const git = gitByPath[node.path] ?? gitByPath[relativePath];
  const status = node.modified ? 'M' : node.gitStatus ?? git?.status;
  const indent = depth * 10;
  const rowBackground = isActive || isSelected ? 'var(--surface-2)' : 'transparent';

  const spec = isDir ? null : getFileSpec(node.name);
  const IconComp = isDir ? (isOpen ? FolderOpen : Folder) : spec!.icon;
  const iconColor = isDir ? 'var(--accent)' : spec!.color;

  const guides = Array.from({ length: depth }, (_, i) => (
    <span
      key={i}
      style={{
        position: 'absolute',
        left: i * 10 + 16,
        top: 0,
        bottom: 0,
        width: 1,
        background: 'var(--border-soft)',
        pointerEvents: 'none',
      }}
    />
  ));

  const activate = () => {
    onSelectPath(node.path);
    if (isDir) onToggleExpanded(node.path);
    else onOpenFile(node.path);
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      activate();
      return;
    }
    if (!isDir) return;
    if (event.key === 'ArrowRight') {
      event.preventDefault();
      onSetExpanded(node.path, true);
    } else if (event.key === 'ArrowLeft') {
      event.preventDefault();
      onSetExpanded(node.path, false);
    }
  };

  return (
    <div role="none">
      <div
        role="treeitem"
        aria-selected={isSelected || isActive}
        aria-expanded={isDir ? isOpen : undefined}
        tabIndex={0}
        className="flex items-center relative outline-none"
        style={{
          height: 24,
          paddingLeft: indent + 4,
          paddingRight: 8,
          cursor: 'pointer',
          background: rowBackground,
          userSelect: 'none',
          borderLeft: isActive ? '2px solid var(--accent)' : '2px solid transparent',
        }}
        title={relativePath || node.path}
        onClick={activate}
        onFocus={() => onSelectPath(node.path)}
        onKeyDown={handleKeyDown}
        onContextMenu={(event) => {
          event.preventDefault();
          onSelectPath(node.path);
          onContextMenu(node, event.clientX, event.clientY);
        }}
        onMouseEnter={e => { if (!isActive && !isSelected) e.currentTarget.style.background = 'var(--surface-2)'; }}
        onMouseLeave={e => { e.currentTarget.style.background = rowBackground; }}
      >
        {guides}
        <span style={{ width: 16, flexShrink: 0, display: 'flex', alignItems: 'center', justifyContent: 'center', position: 'relative', zIndex: 1 }}>
          {isDir
            ? (isOpen
              ? <ChevronDown size={14} color="var(--text-muted)" />
              : <ChevronRight size={14} color="var(--text-muted)" />)
            : null}
        </span>
        <span style={{ display: 'flex', alignItems: 'center', marginRight: 6, flexShrink: 0, position: 'relative', zIndex: 1 }}>
          <IconComp size={15} color={iconColor} />
        </span>
        <span
          className="truncate"
          style={{
            color: isActive ? 'var(--text)' : 'var(--text-muted)',
            fontSize: 12,
            fontFamily: 'var(--font-mono)',
            position: 'relative',
            zIndex: 1,
            letterSpacing: 0,
            minWidth: 0,
          }}
        >
          {node.name}
        </span>
        {status && (
          <span
            style={{
              marginLeft: 'auto',
              color: statusColor(status),
              fontSize: 10,
              fontFamily: 'var(--font-mono)',
              fontWeight: 700,
              flexShrink: 0,
            }}
          >
            {status}
          </span>
        )}
      </div>

      {isDir && isOpen && node.children && (
        <div role="group">
          {sortFileNodes(node.children, sortMode).map((child) => (
            <FileRow
              key={child.path}
              projectPath={projectPath}
              node={child}
              depth={depth + 1}
              activeFilePath={activeFilePath}
              selectedPath={selectedPath}
              expandedPaths={expandedPaths}
              gitByPath={gitByPath}
              query={query}
              sortMode={sortMode}
              onOpenFile={onOpenFile}
              onToggleExpanded={onToggleExpanded}
              onSetExpanded={onSetExpanded}
              onSelectPath={onSelectPath}
              onContextMenu={onContextMenu}
            />
          ))}
        </div>
      )}
    </div>
  );
}

interface Props {
  projectPath: string;
  tree: FileNode[];
  activeFilePath: string | null;
  selectedPath: string | null;
  expandedPaths: string[];
  search: string;
  showHidden: boolean;
  sortMode: FileSortMode;
  scrollTop: number;
  gitStatus: GitStatus | null;
  loading: boolean;
  error?: string | null;
  onRefresh: () => void;
  onOpenFile: (path: string) => void;
  onToggleExpanded: (path: string) => void;
  onSetExpanded: (path: string, expanded: boolean) => void;
  onSelectPath: (path: string | null) => void;
  onSearchChange: (search: string) => void;
  onShowHiddenChange: (showHidden: boolean) => void;
  onSortModeChange: (mode: FileSortMode) => void;
  onScrollChange: (scrollTop: number) => void;
}

export default function FileTree({
  projectPath,
  tree,
  activeFilePath,
  selectedPath,
  expandedPaths,
  search,
  showHidden,
  sortMode,
  scrollTop,
  gitStatus,
  loading,
  error,
  onRefresh,
  onOpenFile,
  onToggleExpanded,
  onSetExpanded,
  onSelectPath,
  onSearchChange,
  onShowHiddenChange,
  onSortModeChange,
  onScrollChange,
}: Props) {
  const projectName = projectPath.split(/[\\/]/).pop() || projectPath;
  const listRef = useRef<HTMLDivElement>(null);
  const scrollFrame = useRef<number | null>(null);
  const [menu, setMenu] = useState<{ node: FileNode; x: number; y: number } | null>(null);
  const query = search.trim();
  const filteredTree = useMemo(() => filterFileTree(tree, query, sortMode), [tree, query, sortMode]);
  const expanded = useMemo(() => new Set(expandedPaths), [expandedPaths]);
  const gitByPath = useMemo(() => gitStatusByPath(gitStatus, projectPath), [gitStatus, projectPath]);
  const quickMatches = useMemo(() => {
    if (!query) return [];
    return collectFiles(tree)
      .map((node) => ({ node, score: quickScore(projectPath, node, query) }))
      .filter(({ score }) => score < 999)
      .sort((a, b) => a.score - b.score || relativeFilePath(projectPath, a.node.path).localeCompare(relativeFilePath(projectPath, b.node.path)))
      .slice(0, 6)
      .map(({ node }) => node);
  }, [tree, projectPath, query]);

  useEffect(() => {
    const closeMenu = () => setMenu(null);
    window.addEventListener('click', closeMenu);
    window.addEventListener('blur', closeMenu);
    return () => {
      window.removeEventListener('click', closeMenu);
      window.removeEventListener('blur', closeMenu);
    };
  }, []);

  useEffect(() => {
    const el = listRef.current;
    if (!el || Math.abs(el.scrollTop - scrollTop) < 2) return;
    el.scrollTop = scrollTop;
  }, [projectPath, loading, scrollTop]);

  useEffect(() => () => {
    if (scrollFrame.current !== null) cancelAnimationFrame(scrollFrame.current);
  }, []);

  const openFirstMatch = () => {
    const first = quickMatches[0];
    if (first) onOpenFile(first.path);
  };

  const copyPath = (path: string) => {
    navigator.clipboard?.writeText(path).catch(() => {});
  };

  const revealPath = (path: string) => {
    window.electronAPI?.openPath?.(path).catch(() => {});
  };

  const setTreeScroll = () => {
    if (scrollFrame.current !== null) return;
    scrollFrame.current = requestAnimationFrame(() => {
      scrollFrame.current = null;
      onScrollChange(listRef.current?.scrollTop ?? 0);
    });
  };

  const actionButtonStyle: React.CSSProperties = {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    width: 24,
    height: 24,
    border: '1px solid transparent',
    borderRadius: 'var(--radius-sm)',
    color: 'var(--text-dim)',
    background: 'transparent',
    cursor: 'pointer',
    flexShrink: 0,
  };

  return (
    <div className="flex flex-col h-full overflow-hidden" style={{ background: 'var(--surface)', color: 'var(--text-muted)' }}>
      <div
        className="flex items-center justify-between px-2 shrink-0"
        style={{ height: 32, borderBottom: '1px solid var(--border-soft)' }}
      >
        <div className="flex items-center gap-1.5 overflow-hidden">
          <FolderRoot size={13} color="var(--accent)" style={{ flexShrink: 0 }} />
          <span className="panel-section-title truncate">{projectName}</span>
        </div>
        <div className="flex items-center gap-0.5">
          <button
            onClick={() => onShowHiddenChange(!showHidden)}
            style={{ ...actionButtonStyle, color: showHidden ? 'var(--accent)' : 'var(--text-dim)' }}
            title={showHidden ? 'Hide hidden files' : 'Show hidden files'}
          >
            {showHidden ? <Eye size={13} /> : <EyeOff size={13} />}
          </button>
          <button
            onClick={onRefresh}
            style={{ ...actionButtonStyle, opacity: loading ? 0.4 : 1 }}
            title="Refresh"
          >
            <RefreshCw size={13} style={{ display: 'block', animation: loading ? 'spin 1s linear infinite' : 'none' }} />
          </button>
        </div>
      </div>

      <div className="shrink-0" style={{ padding: '8px 8px 6px', borderBottom: '1px solid var(--border-soft)' }}>
        <div className="flex items-center gap-1.5" style={{ minWidth: 0 }}>
          <div className="flex items-center gap-1.5 flex-1" style={{
            minWidth: 0,
            height: 28,
            padding: '0 7px',
            background: 'var(--surface-2)',
            border: '1px solid var(--border-soft)',
            borderRadius: 'var(--radius-sm)',
          }}>
            <Search size={13} color="var(--text-dim)" style={{ flexShrink: 0 }} />
            <input
              value={search}
              onChange={(event) => onSearchChange(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === 'Enter') openFirstMatch();
                if (event.key === 'Escape') onSearchChange('');
              }}
              placeholder="Find files"
              aria-label="Find files"
              style={{
                width: '100%',
                minWidth: 0,
                background: 'transparent',
                border: 'none',
                outline: 'none',
                color: 'var(--text)',
                fontSize: 12,
                fontFamily: 'var(--font-sans)',
              }}
            />
            {search && (
              <button
                onClick={() => onSearchChange('')}
                title="Clear search"
                style={{ color: 'var(--text-dim)', background: 'transparent', border: 'none', cursor: 'pointer', padding: 0, display: 'flex' }}
              >
                <X size={13} />
              </button>
            )}
          </div>
          <select
            value={sortMode}
            onChange={(event) => onSortModeChange(event.target.value as FileSortMode)}
            title="Sort files"
            aria-label="Sort files"
            style={{
              height: 28,
              width: 64,
              background: 'var(--surface-2)',
              border: '1px solid var(--border-soft)',
              borderRadius: 'var(--radius-sm)',
              color: 'var(--text-muted)',
              fontSize: 11,
              outline: 'none',
            }}
          >
            <option value="name">Name</option>
            <option value="type">Type</option>
          </select>
        </div>

        {quickMatches.length > 0 && (
          <div style={{ marginTop: 6, display: 'flex', flexDirection: 'column', gap: 1 }}>
            {quickMatches.map((node) => {
              const ref = createFileReference(projectPath, node, 'tree');
              return (
                <button
                  key={node.path}
                  onClick={() => onOpenFile(node.path)}
                  className="flex items-center gap-1.5 text-left"
                  style={{
                    minHeight: 24,
                    padding: '2px 6px',
                    border: 'none',
                    borderRadius: 'var(--radius-sm)',
                    background: node.path === activeFilePath ? 'var(--surface-3)' : 'transparent',
                    color: 'var(--text-muted)',
                    cursor: 'pointer',
                    minWidth: 0,
                  }}
                  title={ref.relativePath}
                  onMouseEnter={e => { e.currentTarget.style.background = 'var(--surface-2)'; }}
                  onMouseLeave={e => { e.currentTarget.style.background = node.path === activeFilePath ? 'var(--surface-3)' : 'transparent'; }}
                >
                  <File size={13} style={{ flexShrink: 0 }} />
                  <span className="truncate" style={{ fontSize: 11, fontFamily: 'var(--font-mono)', minWidth: 0 }}>{ref.relativePath}</span>
                </button>
              );
            })}
          </div>
        )}
      </div>

      {error && (
        <div style={{ padding: '8px 10px', color: 'var(--error)', fontSize: 11, borderBottom: '1px solid var(--border-soft)' }}>
          {error}
        </div>
      )}

      <div
        ref={listRef}
        role="tree"
        className="flex-1 overflow-y-auto"
        style={{ paddingTop: 4, paddingBottom: 16 }}
        onScroll={setTreeScroll}
      >
        {filteredTree.map(node => (
          <FileRow
            key={node.path}
            projectPath={projectPath}
            node={node}
            depth={0}
            activeFilePath={activeFilePath}
            selectedPath={selectedPath}
            expandedPaths={expanded}
            gitByPath={gitByPath}
            query={query}
            sortMode={sortMode}
            onOpenFile={onOpenFile}
            onToggleExpanded={onToggleExpanded}
            onSetExpanded={onSetExpanded}
            onSelectPath={(path) => onSelectPath(path)}
            onContextMenu={(node, x, y) => setMenu({ node, x, y })}
          />
        ))}
        {!loading && filteredTree.length === 0 && (
          <div style={{ padding: '12px 10px', color: 'var(--text-dim)', fontSize: 12 }}>
            {query ? 'No files match' : 'No files found'}
          </div>
        )}
      </div>

      {menu && (
        <div
          role="menu"
          style={{
            position: 'fixed',
            top: menu.y,
            left: menu.x,
            zIndex: 60,
            minWidth: 180,
            padding: 4,
            background: 'var(--surface-2)',
            border: '1px solid var(--border)',
            borderRadius: 'var(--radius-md)',
            boxShadow: 'var(--shadow-lg)',
          }}
          onClick={(event) => event.stopPropagation()}
        >
          <ContextMenuButton icon={<File size={13} />} label="Open" onClick={() => { onOpenFile(menu.node.path); setMenu(null); }} />
          <ContextMenuButton icon={<Copy size={13} />} label="Copy path" onClick={() => { copyPath(menu.node.path); setMenu(null); }} />
          <ContextMenuButton icon={<Copy size={13} />} label="Copy relative path" onClick={() => { copyPath(relativeFilePath(projectPath, menu.node.path)); setMenu(null); }} />
          {window.electronAPI?.openPath && (
            <ContextMenuButton icon={<ExternalLink size={13} />} label="Reveal in system" onClick={() => { revealPath(menu.node.path); setMenu(null); }} />
          )}
        </div>
      )}
    </div>
  );
}

function ContextMenuButton({ icon, label, onClick }: { icon: React.ReactNode; label: string; onClick: () => void }) {
  return (
    <button
      role="menuitem"
      onClick={onClick}
      className="w-full flex items-center gap-2 text-left"
      style={{
        height: 26,
        padding: '0 8px',
        border: 'none',
        borderRadius: 'var(--radius-sm)',
        background: 'transparent',
        color: 'var(--text-muted)',
        fontSize: 12,
        cursor: 'pointer',
      }}
      onMouseEnter={e => { e.currentTarget.style.background = 'var(--surface-3)'; e.currentTarget.style.color = 'var(--text)'; }}
      onMouseLeave={e => { e.currentTarget.style.background = 'transparent'; e.currentTarget.style.color = 'var(--text-muted)'; }}
    >
      <span style={{ display: 'flex', color: 'var(--text-dim)', flexShrink: 0 }}>{icon}</span>
      <span className="truncate">{label}</span>
    </button>
  );
}
