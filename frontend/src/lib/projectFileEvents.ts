import { useProjectFileStore } from '../stores/projectFileStore';

export interface ProjectFileInvalidation {
  paths: string[];
  refreshTree: boolean;
  refreshGit: boolean;
  reason: 'tool_result' | 'workflow_event';
  toolName?: string;
  action?: string;
  eventKind?: string;
}

const EDIT_TOOL_NAMES = new Set([
  'Edit',
  'Write',
  'file_edit',
  'flamel_replace',
  'replace_content',
  'multi_replace',
  'diff_apply',
  'create_text_file',
  'mcp__flamel__file_edit',
  'mcp__flamel__replace_content',
  'mcp__flamel__multi_replace',
  'mcp__flamel__diff_apply',
  'mcp__flamel__create_text_file',
]);

const MUTATING_FILE_ACTIONS = new Set([
  'replace',
  'replace_content',
  'replace_symbol',
  'insert_before',
  'insert_after',
  'create',
  'delete',
  'multi_replace',
  'diff_apply',
  'format',
  'create_text_file',
]);

const TREE_CHANGING_ACTIONS = new Set(['create', 'delete', 'diff_apply', 'create_text_file']);

const pendingRefreshes = new Map<string, {
  paths: Set<string>;
  refreshTree: boolean;
  refreshGit: boolean;
  timer: ReturnType<typeof setTimeout>;
}>();

function bareToolName(name: string): string {
  if (name.includes('__')) return name.split('__').pop() || name;
  return name;
}

function stringValue(value: unknown): string | null {
  return typeof value === 'string' && value.trim() ? value.trim() : null;
}

function addPath(paths: Set<string>, value: unknown) {
  const path = stringValue(value);
  if (path) paths.add(path);
}

function addPathsFromDiff(paths: Set<string>, diff: unknown) {
  const text = stringValue(diff);
  if (!text) return;
  for (const line of text.split('\n')) {
    if (!line.startsWith('--- ') && !line.startsWith('+++ ')) continue;
    const raw = line.slice(4).trim();
    if (!raw || raw === '/dev/null') continue;
    const withoutPrefix = raw.replace(/^[ab]\//, '');
    addPath(paths, withoutPrefix);
  }
}

function addPathsFromPayload(paths: Set<string>, payload: Record<string, unknown>) {
  addPath(paths, payload.path);
  addPath(paths, payload.file);
  addPath(paths, payload.relative_path);
  addPath(paths, payload.relativePath);
  addPath(paths, payload.file_path);
  addPath(paths, payload.filePath);
  addPathsFromCollection(paths, payload.paths);
  addPathsFromCollection(paths, payload.files);
}

function addPathsFromCollection(paths: Set<string>, value: unknown) {
  if (!Array.isArray(value)) return;
  for (const item of value) {
    if (typeof item === 'string') {
      addPath(paths, item);
      continue;
    }
    if (!item || typeof item !== 'object') continue;
    const record = item as Record<string, unknown>;
    addPath(paths, record.file);
    addPath(paths, record.path);
    addPath(paths, record.relative_path);
    addPath(paths, record.file_path);
    addPath(paths, record.old_path);
    addPath(paths, record.new_path);
  }
}

function inferAction(toolName: string, input: Record<string, unknown>): string {
  const explicit = stringValue(input.action);
  if (explicit) return explicit;
  const bare = bareToolName(toolName);
  if (bare === 'Write' || bare === 'create_text_file') return 'create';
  if (bare === 'Edit' || bare === 'flamel_replace') return 'replace';
  if (bare === 'replace_content') return 'replace_content';
  if (bare === 'multi_replace') return 'multi_replace';
  if (bare === 'diff_apply') return 'diff_apply';
  if (bare === 'file_edit') {
    if (Array.isArray(input.edits)) return 'multi_replace';
    if (stringValue(input.diff)) return 'diff_apply';
    if (stringValue(input.content)) return 'create';
    if (stringValue(input.relative_path) || stringValue(input.file)) return 'replace';
  }
  return bare;
}

export function extractProjectFileInvalidation(
  toolName: string,
  input: Record<string, unknown> | undefined,
): ProjectFileInvalidation | null {
  const toolInput = input || {};
  const action = inferAction(toolName, toolInput);
  const bare = bareToolName(toolName);
  const isKnownEditTool = EDIT_TOOL_NAMES.has(toolName) || EDIT_TOOL_NAMES.has(bare);
  const isMutatingAction = MUTATING_FILE_ACTIONS.has(action);
  if (!isKnownEditTool || !isMutatingAction || action === 'preview') return null;

  const paths = new Set<string>();
  addPath(paths, toolInput.file_path);
  addPath(paths, toolInput.relative_path);
  addPath(paths, toolInput.path);
  addPath(paths, toolInput.file);
  addPathsFromCollection(paths, toolInput.edits);
  addPathsFromCollection(paths, toolInput.changes);
  addPathsFromCollection(paths, toolInput.files);
  addPathsFromDiff(paths, toolInput.diff);

  const refreshTree = TREE_CHANGING_ACTIONS.has(action) || Array.isArray(toolInput.changes) || action === 'multi_replace';
  return {
    paths: Array.from(paths),
    refreshTree,
    refreshGit: true,
    reason: 'tool_result',
    toolName: bare,
    action,
  };
}

export function scheduleProjectFileRefresh(projectPath: string, invalidation: ProjectFileInvalidation) {
  if (!projectPath) return;
  useProjectFileStore.getState().recordFileMutation(projectPath, invalidation.paths, {
    source: invalidation.reason,
    toolName: invalidation.toolName,
    action: invalidation.action,
    eventKind: invalidation.eventKind,
  });
  const existing = pendingRefreshes.get(projectPath);
  if (existing) {
    for (const path of invalidation.paths) existing.paths.add(path);
    existing.refreshTree = existing.refreshTree || invalidation.refreshTree;
    existing.refreshGit = existing.refreshGit || invalidation.refreshGit;
    clearTimeout(existing.timer);
    existing.timer = setTimeout(() => flushProjectFileRefresh(projectPath), 180);
    return;
  }

  pendingRefreshes.set(projectPath, {
    paths: new Set(invalidation.paths),
    refreshTree: invalidation.refreshTree,
    refreshGit: invalidation.refreshGit,
    timer: setTimeout(() => flushProjectFileRefresh(projectPath), 180),
  });
}

export function flushProjectFileRefresh(projectPath: string) {
  const pending = pendingRefreshes.get(projectPath);
  if (!pending) return;
  pendingRefreshes.delete(projectPath);
  const store = useProjectFileStore.getState();
  if (pending.refreshTree) store.loadProject(projectPath, { force: true });
  if (pending.refreshGit) store.refreshGitStatus(projectPath);
}

export function maybeRefreshProjectFilesFromTool(
  projectPath: string,
  toolName: string,
  input: Record<string, unknown> | undefined,
  isError?: boolean,
): boolean {
  if (isError) return false;
  const invalidation = extractProjectFileInvalidation(toolName, input);
  if (!invalidation) return false;
  scheduleProjectFileRefresh(projectPath, invalidation);
  return true;
}

export function maybeRefreshProjectFilesFromWorkflowEvent(
  event: { kind: string; payload?: Record<string, unknown> },
  fallbackProjectPath: string,
): boolean {
  if (!event.kind.startsWith('file.')) return false;
  const payload = event.payload || {};
  const projectPath = stringValue(payload.projectPath) || stringValue(payload.project_path) || fallbackProjectPath;
  if (!projectPath) return false;

  const paths = new Set<string>();
  addPathsFromPayload(paths, payload);
  const refreshTree = Boolean(payload.refreshTree)
    || event.kind === 'file.created'
    || event.kind === 'file.deleted'
    || event.kind === 'file.moved'
    || event.kind === 'file.tree_changed'
    || event.kind === 'file.refresh_requested';
  const refreshGit = payload.refreshGit !== false;

  scheduleProjectFileRefresh(projectPath, {
    paths: Array.from(paths),
    refreshTree,
    refreshGit,
    reason: 'workflow_event',
    eventKind: event.kind,
  });
  return true;
}

