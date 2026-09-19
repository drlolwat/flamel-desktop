export type ToolRendererKind =
  | 'generic'
  | 'console'
  | 'plan'
  | 'agent'
  | 'file'
  | 'code'
  | 'assets'
  | 'research'
  | 'browser'
  | 'question'
  | 'diagnostics';

export type ToolRisk = 'read_only' | 'low' | 'medium' | 'high';

export type ToolFamily =
  | 'agents'
  | 'workflow'
  | 'filesystem'
  | 'source-control'
  | 'runtime'
  | 'research'
  | 'browser'
  | 'media'
  | 'data'
  | 'knowledge'
  | 'integrations'
  | 'diagnostics'
  | 'external';

export type ToolPresentationAdapter =
  | 'status'
  | 'timeline'
  | 'diff'
  | 'terminal'
  | 'tree'
  | 'gallery'
  | 'sources'
  | 'browser'
  | 'table'
  | 'document'
  | 'question'
  | 'structured';

export type ToolSafeAction =
  | 'copy-summary'
  | 'copy-output'
  | 'open-resource'
  | 'download-artifact'
  | 'open-file'
  | 'inspect-diff'
  | 'retry'
  | 'view-source';

export interface ToolCapability {
  name: string;
  aliases?: string[];
  displayName: string;
  category: string;
  risk: ToolRisk;
  renderer: ToolRendererKind;
  family: ToolFamily;
  adapter: ToolPresentationAdapter;
  iconKey: string;
  spriteKey?: string;
  safeActions: ToolSafeAction[];
  readOnly: boolean;
  destructive: boolean;
  mutatesFiles?: boolean;
  requiresPriorFileRead?: boolean;
  streams?: boolean;
}

type ToolCapabilitySeed = Omit<
  ToolCapability,
  'family' | 'adapter' | 'iconKey' | 'spriteKey' | 'safeActions'
>;

const TOOL_CAPABILITY_SEEDS: ToolCapabilitySeed[] = [
  {
    name: 'Agent',
    displayName: 'Sub-agent',
    category: 'agent',
    risk: 'high',
    renderer: 'agent',
    readOnly: false,
    destructive: true,
    mutatesFiles: true,
    streams: true,
  },
  {
    name: 'AgentBatch',
    displayName: 'Agent batch',
    category: 'agent',
    risk: 'high',
    renderer: 'agent',
    readOnly: false,
    destructive: true,
    mutatesFiles: true,
    streams: true,
  },
  {
    name: 'AgentBatchStatus',
    displayName: 'Agent batch status',
    category: 'agent',
    risk: 'read_only',
    renderer: 'agent',
    readOnly: true,
    destructive: false,
    streams: true,
  },
  {
    name: 'plan',
    displayName: 'Plan',
    category: 'workflow',
    risk: 'medium',
    renderer: 'plan',
    readOnly: false,
    destructive: false,
  },
  {
    name: 'file_history',
    aliases: ['revert_file', 'list_file_edits', 'checkpoint', 'rollback'],
    displayName: 'File history',
    category: 'filesystem',
    risk: 'medium',
    renderer: 'file',
    readOnly: false,
    destructive: true,
    mutatesFiles: true,
  },
  {
    name: 'flamel_context_stats',
    displayName: 'Context stats',
    category: 'diagnostics',
    risk: 'read_only',
    renderer: 'diagnostics',
    readOnly: true,
    destructive: false,
  },
  {
    name: 'diagnostics',
    aliases: ['runtime_logs'],
    displayName: 'Diagnostics',
    category: 'diagnostics',
    risk: 'read_only',
    renderer: 'diagnostics',
    readOnly: true,
    destructive: false,
  },
  {
    name: 'capability_search',
    displayName: 'Capability search',
    category: 'integrations',
    risk: 'read_only',
    renderer: 'generic',
    readOnly: true,
    destructive: false,
  },
  {
    name: 'capability_execute',
    displayName: 'Capability execute',
    category: 'integrations',
    risk: 'medium',
    renderer: 'generic',
    readOnly: false,
    destructive: false,
  },
  {
    name: 'project_intelligence',
    displayName: 'Project intelligence',
    category: 'code',
    risk: 'read_only',
    renderer: 'code',
    readOnly: true,
    destructive: false,
  },
  {
    name: 'skill_search',
    displayName: 'Skill search',
    category: 'knowledge',
    risk: 'read_only',
    renderer: 'generic',
    readOnly: true,
    destructive: false,
  },
  {
    name: 'work',
    displayName: 'Engineering work',
    category: 'workflow',
    risk: 'low',
    renderer: 'generic',
    readOnly: false,
    destructive: false,
  },
  {
    name: 'code_nav',
    aliases: [
      'flamel_search',
      'get_symbols_overview',
      'find_symbol',
      'find_referencing_symbols',
      'get_imports',
      'gather_context',
      'dependency_graph',
    ],
    displayName: 'Code navigation',
    category: 'code',
    risk: 'read_only',
    renderer: 'code',
    readOnly: true,
    destructive: false,
  },
  {
    name: 'shell',
    aliases: [
      'flamel_shell',
      'shell_run',
      'shell_start',
      'shell_read',
      'shell_write',
      'shell_kill',
      'shell_status',
      'multi_shell',
      'bash',
      'exec_command',
      'command_execution',
      'terminal',
    ],
    displayName: 'Shell',
    category: 'command',
    risk: 'high',
    renderer: 'console',
    readOnly: false,
    destructive: true,
    mutatesFiles: true,
    streams: true,
  },
  {
    name: 'file_edit',
    aliases: [
      'flamel_replace',
      'replace_content',
      'multi_replace',
      'replace_symbol_body',
      'insert_before_symbol',
      'insert_after_symbol',
      'create_text_file',
      'diff_apply',
      'format_file',
      'preview_edit',
    ],
    displayName: 'File edit',
    category: 'filesystem',
    risk: 'high',
    renderer: 'file',
    readOnly: false,
    destructive: true,
    mutatesFiles: true,
    requiresPriorFileRead: true,
  },
  {
    name: 'decision_checkpoint',
    displayName: 'Decision checkpoint',
    category: 'workflow',
    risk: 'low',
    renderer: 'generic',
    readOnly: false,
    destructive: false,
  },
  {
    name: 'AskUserQuestion',
    displayName: 'Ask user',
    category: 'workflow',
    risk: 'low',
    renderer: 'question',
    readOnly: false,
    destructive: false,
  },
  {
    name: 'file_read',
    aliases: ['flamel_read', 'read_file', 'batch_read', 'file_summary', 'flamel_glob', 'find_file', 'list_dir'],
    displayName: 'File read',
    category: 'filesystem',
    risk: 'read_only',
    renderer: 'file',
    readOnly: true,
    destructive: false,
  },
  {
    name: 'code_quality',
    aliases: ['build_errors', 'run_tests', 'explain_error', 'find_code_issues'],
    displayName: 'Code quality',
    category: 'code',
    risk: 'medium',
    renderer: 'console',
    readOnly: false,
    destructive: false,
    streams: true,
  },
  {
    name: 'get_recent_changes',
    displayName: 'Recent changes',
    category: 'git',
    risk: 'read_only',
    renderer: 'code',
    readOnly: true,
    destructive: false,
  },
  {
    name: 'assets',
    aliases: ['media', 'fetch_image', 'fetch_model3d', 'create_model3d'],
    displayName: 'Assets',
    category: 'assets',
    risk: 'medium',
    renderer: 'assets',
    readOnly: false,
    destructive: false,
    mutatesFiles: true,
  },
  {
    name: 'git',
    displayName: 'Git',
    category: 'source-control',
    risk: 'high',
    renderer: 'code',
    readOnly: false,
    destructive: true,
    mutatesFiles: true,
  },
  {
    name: 'web_search',
    aliases: ['WebSearch', 'WebFetch', 'webSearch', 'web_fetch'],
    displayName: 'Web research',
    category: 'research',
    risk: 'read_only',
    renderer: 'research',
    readOnly: true,
    destructive: false,
    streams: true,
  },
  {
    name: 'browser_session',
    aliases: ['browser'],
    displayName: 'Browser session',
    category: 'testing',
    risk: 'medium',
    renderer: 'browser',
    readOnly: false,
    destructive: false,
    streams: true,
  },
  {
    name: 'browser_run',
    displayName: 'Browser run',
    category: 'testing',
    risk: 'medium',
    renderer: 'browser',
    readOnly: false,
    destructive: false,
    streams: true,
  },
  {
    name: 'browser_inspect',
    displayName: 'Browser inspection',
    category: 'testing',
    risk: 'read_only',
    renderer: 'browser',
    readOnly: true,
    destructive: false,
    streams: true,
  },
  {
    name: 'db',
    aliases: ['database'],
    displayName: 'Database',
    category: 'data',
    risk: 'read_only',
    renderer: 'generic',
    readOnly: true,
    destructive: false,
  },
  {
    name: 'memory',
    displayName: 'Project memory',
    category: 'knowledge',
    risk: 'medium',
    renderer: 'generic',
    readOnly: false,
    destructive: true,
  },
  {
    name: 'resolve_library',
    displayName: 'Resolve library',
    category: 'knowledge',
    risk: 'read_only',
    renderer: 'generic',
    readOnly: true,
    destructive: false,
  },
  {
    name: 'get_docs',
    displayName: 'Library docs',
    category: 'knowledge',
    risk: 'read_only',
    renderer: 'generic',
    readOnly: true,
    destructive: false,
  },
  {
    name: 'search_flamel_docs',
    displayName: 'Search Flamel docs',
    category: 'knowledge',
    risk: 'read_only',
    renderer: 'generic',
    readOnly: true,
    destructive: false,
  },
  {
    name: 'get_flamel_doc',
    displayName: 'Flamel docs',
    category: 'knowledge',
    risk: 'read_only',
    renderer: 'generic',
    readOnly: true,
    destructive: false,
  },
  {
    name: 'skill_catalog',
    displayName: 'Skill catalog',
    category: 'integrations',
    risk: 'medium',
    renderer: 'generic',
    readOnly: false,
    destructive: false,
  },
  {
    name: 'assistant',
    displayName: 'Assistant',
    category: 'workflow',
    risk: 'medium',
    renderer: 'generic',
    readOnly: false,
    destructive: false,
    streams: true,
  },
  {
    name: 'job',
    displayName: 'Job',
    category: 'workflow',
    risk: 'medium',
    renderer: 'plan',
    readOnly: false,
    destructive: false,
    streams: true,
  },
  {
    name: 'image_gen',
    aliases: ['imagegen', 'image_generation', 'imageGeneration'],
    displayName: 'Image generation',
    category: 'assets',
    risk: 'medium',
    renderer: 'assets',
    readOnly: false,
    destructive: false,
    mutatesFiles: true,
    streams: true,
  },
  {
    name: 'view_image',
    displayName: 'Image preview',
    category: 'assets',
    risk: 'read_only',
    renderer: 'assets',
    readOnly: true,
    destructive: false,
  },
  {
    name: 'time',
    displayName: 'Time',
    category: 'data',
    risk: 'read_only',
    renderer: 'generic',
    readOnly: true,
    destructive: false,
  },
];

export function shortToolName(toolName: string): string {
  const withoutMcpPrefix = toolName.includes('__') ? toolName.split('__').pop()! : toolName;
  return withoutMcpPrefix.includes('.') ? withoutMcpPrefix.split('.').pop()! : withoutMcpPrefix;
}

function presentationForSeed(
  seed: ToolCapabilitySeed,
): Pick<ToolCapability, 'family' | 'adapter' | 'iconKey' | 'spriteKey' | 'safeActions'> {
  if (seed.renderer === 'agent') {
    return {
      family: 'agents',
      adapter: 'timeline',
      iconKey: 'users',
      spriteKey: 'tool-agent',
      safeActions: ['copy-summary', 'retry'],
    };
  }
  if (seed.renderer === 'plan') {
    return {
      family: 'workflow',
      adapter: 'status',
      iconKey: 'list-checks',
      spriteKey: 'tool-workflow',
      safeActions: ['copy-summary'],
    };
  }
  if (seed.renderer === 'console') {
    return {
      family: 'runtime',
      adapter: 'terminal',
      iconKey: 'terminal',
      spriteKey: 'tool-terminal',
      safeActions: ['copy-output', 'retry'],
    };
  }
  if (seed.renderer === 'file') {
    const editsFiles = seed.name === 'file_edit' || seed.name === 'file_history';
    return {
      family: 'filesystem',
      adapter: editsFiles ? 'diff' : 'tree',
      iconKey: editsFiles ? 'file-diff' : 'files',
      spriteKey: editsFiles ? 'tool-diff' : 'tool-files',
      safeActions: editsFiles
        ? ['inspect-diff', 'open-file', 'copy-summary']
        : ['open-file', 'copy-summary'],
    };
  }
  if (seed.renderer === 'code') {
    const showsDiff = seed.name === 'git' || seed.name === 'get_recent_changes';
    return {
      family: seed.category === 'source-control' || seed.category === 'git'
        ? 'source-control'
        : 'filesystem',
      adapter: showsDiff ? 'diff' : 'tree',
      iconKey: showsDiff ? 'git-compare' : 'braces',
      spriteKey: showsDiff ? 'tool-diff' : 'tool-code',
      safeActions: showsDiff
        ? ['inspect-diff', 'copy-summary']
        : ['open-file', 'copy-summary'],
    };
  }
  if (seed.renderer === 'assets') {
    return {
      family: 'media',
      adapter: 'gallery',
      iconKey: seed.name === 'image_gen' ? 'sparkles' : 'image',
      spriteKey: seed.name === 'image_gen' ? 'tool-image-generation' : 'tool-media',
      safeActions: ['download-artifact', 'open-resource', 'copy-summary'],
    };
  }
  if (seed.renderer === 'research') {
    return {
      family: 'research',
      adapter: 'sources',
      iconKey: 'search',
      spriteKey: 'tool-research',
      safeActions: ['view-source', 'open-resource', 'copy-summary'],
    };
  }
  if (seed.renderer === 'browser') {
    return {
      family: 'browser',
      adapter: 'browser',
      iconKey: 'globe',
      spriteKey: 'tool-browser',
      safeActions: ['open-resource', 'copy-summary', 'retry'],
    };
  }
  if (seed.renderer === 'question') {
    return {
      family: 'workflow',
      adapter: 'question',
      iconKey: 'message-circle-question',
      spriteKey: 'tool-question',
      safeActions: ['copy-summary'],
    };
  }
  if (seed.renderer === 'diagnostics') {
    return {
      family: 'diagnostics',
      adapter: 'status',
      iconKey: 'activity',
      spriteKey: 'tool-diagnostics',
      safeActions: ['copy-output', 'copy-summary'],
    };
  }

  const genericFamily: ToolFamily =
    seed.category === 'data'
      ? 'data'
      : seed.category === 'knowledge'
        ? 'knowledge'
        : seed.category === 'integrations'
          ? 'integrations'
          : seed.category === 'workflow'
            ? 'workflow'
            : 'external';
  return {
    family: genericFamily,
    adapter:
      genericFamily === 'data'
        ? 'table'
        : genericFamily === 'knowledge'
          ? 'document'
          : genericFamily === 'workflow'
            ? 'status'
            : 'structured',
    iconKey:
      genericFamily === 'data'
        ? 'table-2'
        : genericFamily === 'knowledge'
          ? 'book-open'
          : genericFamily === 'workflow'
            ? 'workflow'
            : 'plug',
    spriteKey: `tool-${genericFamily}`,
    safeActions: ['copy-summary', 'open-resource'],
  };
}

export const FLAMEL_TOOL_CAPABILITIES: ToolCapability[] = TOOL_CAPABILITY_SEEDS.map((seed) => ({
  ...seed,
  ...presentationForSeed(seed),
}));

function assistantActionCapability(
  base: ToolCapability,
  action: string | undefined,
): ToolCapability {
  if (!action) return base;
  if (action === 'assistant.status' || action.startsWith('diagnostics.')) {
    return {
      ...base,
      displayName: 'Assistant diagnostics',
      category: 'diagnostics',
      family: 'diagnostics',
      adapter: 'status',
      iconKey: 'activity',
      spriteKey: 'tool-diagnostics',
      safeActions: ['copy-output', 'copy-summary'],
      risk: 'read_only',
      readOnly: true,
      destructive: false,
    };
  }
  if (action.startsWith('brain.')) {
    return {
      ...base,
      displayName: 'Assistant memory',
      category: 'knowledge',
      family: 'knowledge',
      adapter: 'document',
      iconKey: 'brain',
      spriteKey: 'tool-knowledge',
      safeActions: ['copy-summary', 'open-resource'],
    };
  }
  if (action.startsWith('skill.') || action.startsWith('pack.') || action.startsWith('connection.')) {
    return {
      ...base,
      displayName: 'Assistant integration',
      category: 'integrations',
      family: 'integrations',
      adapter: 'structured',
      iconKey: 'blocks',
      spriteKey: 'tool-integrations',
      safeActions: ['copy-summary', 'open-resource'],
    };
  }
  if (action.startsWith('automation.') || action.startsWith('job.')) {
    return {
      ...base,
      displayName: 'Assistant workflow',
      category: 'workflow',
      family: 'workflow',
      adapter: 'timeline',
      iconKey: 'workflow',
      spriteKey: 'tool-workflow',
      safeActions: ['copy-summary', 'retry'],
    };
  }
  return base;
}

function externalToolCapability(toolName: string): ToolCapability {
  return {
    name: shortToolName(toolName),
    displayName: shortToolName(toolName),
    category: 'external',
    risk: 'high',
    renderer: 'generic',
    family: 'external',
    adapter: 'structured',
    iconKey: 'plug',
    spriteKey: 'tool-external',
    safeActions: ['copy-summary', 'copy-output'],
    readOnly: false,
    destructive: true,
  };
}

export function capabilityForTool(
  toolName: string,
  input?: Record<string, unknown>,
): ToolCapability {
  const unprefixed = toolName.includes('__') ? toolName.split('__').pop()! : toolName;
  const dottedAssistantAction = unprefixed.startsWith('assistant.') ? unprefixed : undefined;
  const short = dottedAssistantAction ? 'assistant' : shortToolName(toolName);
  const lower = short.toLowerCase();
  const capability = FLAMEL_TOOL_CAPABILITIES.find((candidate) =>
    matchesCapability(candidate, lower),
  );
  if (!capability) return externalToolCapability(toolName);
  if (capability.name !== 'assistant') return capability;
  const action =
    dottedAssistantAction ?? (typeof input?.action === 'string' ? input.action : undefined);
  return assistantActionCapability(capability, action);
}

export function rendererForTool(toolName: string): ToolRendererKind {
  return capabilityForTool(toolName).renderer;
}

export function isToolRenderer(toolName: string, renderer: ToolRendererKind): boolean {
  return rendererForTool(toolName) === renderer;
}

export function isShellLikeToolName(toolName: string): boolean {
  return isToolRenderer(toolName, 'console');
}

export function displayNameForTool(toolName: string): string {
  return capabilityForTool(toolName).displayName;
}

function matchesCapability(capability: ToolCapability, lowerShortName: string): boolean {
  if (capability.name.toLowerCase() === lowerShortName) return true;
  return Boolean(capability.aliases?.some((alias) => alias.toLowerCase() === lowerShortName));
}
