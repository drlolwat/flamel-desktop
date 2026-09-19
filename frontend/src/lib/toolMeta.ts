export interface ToolMeta {
  icon: string;
  label: string;
  color: string;
}

const TOOL_META: Record<string, ToolMeta> = {
  read_file: { icon: '📄', label: 'read_file', color: 'var(--text-muted)' },
  write_file: { icon: '📝', label: 'write_file', color: 'var(--warning)' },
  edit_file: { icon: '✏️', label: 'edit_file', color: 'var(--accent)' },
  bash: { icon: '>', label: 'bash', color: 'var(--success)' },
  list_directory: { icon: '📁', label: 'list_directory', color: 'var(--text-muted)' },
  glob: { icon: '🔍', label: 'glob', color: 'var(--text-muted)' },
  grep: { icon: '🔎', label: 'grep', color: 'var(--text-muted)' },
  task: { icon: '⚡', label: 'task', color: 'var(--accent)' },
  web_fetch: { icon: '🌐', label: 'web_fetch', color: 'var(--accent)' },
  web_search: { icon: '🔍', label: 'web_search', color: 'var(--accent)' },
  todo_write: { icon: '📋', label: 'todo_write', color: 'var(--success)' },
  todo_read: { icon: '📋', label: 'todo_read', color: 'var(--text-muted)' },
};

export function getToolMeta(toolName: string): ToolMeta {
  const key = toolName.toLowerCase().replace(/-/g, '_');
  return TOOL_META[key] || { icon: '🔧', label: toolName, color: 'var(--accent)' };
}
