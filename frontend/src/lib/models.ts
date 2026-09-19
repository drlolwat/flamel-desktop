export type ProviderKind = 'claude' | 'codex';

export type ModelId =
  | 'gpt-6-astra'
  | 'claude-sonnet-4-6'
  | 'claude-opus-4-7'
  | 'claude-opus-4-6'
  | 'claude-haiku-4-5'
  | 'gpt-5.6'
  | 'gpt-5.6-sol'
  | 'gpt-5.6-terra'
  | 'gpt-5.6-luna'
  | 'gpt-5.5'
  | 'gpt-5.5-pro'
  | 'gpt-5.4'
  | 'gpt-5.4-pro'
  | 'gpt-5.4-mini'
  | 'gpt-5.3-codex'
  | 'gpt-5.3-codex-spark'
  | 'gpt-5.2'
  | 'gpt-5.2-pro';

export type ModelSelection = ModelId | 'auto';
export type ReasoningSelection = ReasoningEffort | 'auto';

export type ReasoningEffort = 'low' | 'medium' | 'high' | 'xhigh' | 'max' | 'ultra';

export interface ReasoningEffortDef {
  id: ReasoningEffort;
  label: string;
  title: string;
}

export interface ReasoningSelectionDef {
  id: ReasoningSelection;
  label: string;
  title: string;
}

export interface ModelDef {
  id: ModelId;
  label: string;
  provider: ProviderKind;
  reasoningEfforts?: ReasoningEffort[];
  defaultReasoningEffort?: ReasoningEffort;
}

export const CODEX_REASONING_EFFORTS: ReasoningEffortDef[] = [
  { id: 'low', label: 'low', title: 'Fast responses with lighter reasoning' },
  { id: 'medium', label: 'med', title: 'Balanced speed and reasoning depth' },
  { id: 'high', label: 'high', title: 'Greater reasoning depth for complex problems' },
  { id: 'xhigh', label: 'xhigh', title: 'Extra-high reasoning depth' },
  { id: 'max', label: 'max', title: 'Maximum reasoning depth for the hardest problems' },
  { id: 'ultra', label: 'ultra', title: 'Maximum reasoning with automatic task delegation' },
];

const STANDARD_CODEX_EFFORTS: ReasoningEffort[] = ['low', 'medium', 'high', 'xhigh'];
const GPT_56_MAX_EFFORTS: ReasoningEffort[] = [...STANDARD_CODEX_EFFORTS, 'max'];
const GPT_56_ULTRA_EFFORTS: ReasoningEffort[] = [...GPT_56_MAX_EFFORTS, 'ultra'];

export const MODEL_GROUPS: { provider: ProviderKind; label: string; models: ModelDef[] }[] = [
  {
    provider: 'claude',
    label: 'Claude',
    models: [
      { id: 'claude-sonnet-4-6', label: 'Sonnet 4.6', provider: 'claude' },
      { id: 'claude-opus-4-7', label: 'Opus 4.7', provider: 'claude' },
      { id: 'claude-opus-4-6', label: 'Opus 4.6', provider: 'claude' },
      { id: 'claude-haiku-4-5', label: 'Haiku 4.5', provider: 'claude' },
    ],
  },
  {
    provider: 'codex',
    label: 'OpenAI Codex',
    models: [
      {
        id: 'gpt-6-astra',
        label: 'GPT-6 Astra',
        provider: 'codex',
        reasoningEfforts: ['low', 'medium', 'high', 'xhigh', 'max', 'ultra'],
        defaultReasoningEffort: 'medium',
      },
      {
        id: 'gpt-5.6-sol',
        label: 'GPT-5.6 Sol',
        provider: 'codex',
        reasoningEfforts: GPT_56_ULTRA_EFFORTS,
        defaultReasoningEffort: 'low',
      },
      {
        id: 'gpt-5.6-terra',
        label: 'GPT-5.6 Terra',
        provider: 'codex',
        reasoningEfforts: GPT_56_ULTRA_EFFORTS,
        defaultReasoningEffort: 'medium',
      },
      {
        id: 'gpt-5.6-luna',
        label: 'GPT-5.6 Luna',
        provider: 'codex',
        reasoningEfforts: GPT_56_MAX_EFFORTS,
        defaultReasoningEffort: 'medium',
      },
      {
        id: 'gpt-5.6',
        label: 'GPT-5.6 (Sol alias)',
        provider: 'codex',
        reasoningEfforts: GPT_56_ULTRA_EFFORTS,
        defaultReasoningEffort: 'low',
      },
      { id: 'gpt-5.5', label: 'GPT-5.5', provider: 'codex' },
      { id: 'gpt-5.5-pro', label: 'GPT-5.5 Pro', provider: 'codex' },
      { id: 'gpt-5.4', label: 'GPT-5.4', provider: 'codex' },
      { id: 'gpt-5.4-pro', label: 'GPT-5.4 Pro', provider: 'codex' },
      { id: 'gpt-5.4-mini', label: 'GPT-5.4 mini', provider: 'codex' },
      { id: 'gpt-5.3-codex', label: 'GPT-5.3 Codex', provider: 'codex' },
      { id: 'gpt-5.3-codex-spark', label: 'Codex Spark', provider: 'codex' },
      { id: 'gpt-5.2', label: 'GPT-5.2', provider: 'codex' },
      { id: 'gpt-5.2-pro', label: 'GPT-5.2 Pro', provider: 'codex' },
    ],
  },
];

export const ALL_MODELS = MODEL_GROUPS.flatMap(group => group.models);
export const CODEX_MODELS = ALL_MODELS.filter(model => model.provider === 'codex');
export const DEFAULT_MODEL: ModelId = 'gpt-5.6-sol';

export function manualModelSelection(
  requestedModel?: ModelSelection | string | null,
  resolvedModel?: ModelId | string | null,
): ModelId {
  if (requestedModel && requestedModel !== 'auto') return requestedModel as ModelId;
  if (resolvedModel && resolvedModel !== 'auto') return resolvedModel as ModelId;
  return DEFAULT_MODEL;
}

export function modelLabel(model: ModelSelection | string): string {
  if (model === 'auto') return 'Auto';
  return ALL_MODELS.find(definition => definition.id === model)?.label ?? model;
}

export function modelProvider(model: ModelId | string): ProviderKind {
  if (model.startsWith('gpt-') || model.startsWith('o')) return 'codex';
  return ALL_MODELS.find(definition => definition.id === model)?.provider ?? 'codex';
}

export function reasoningEffortForSend(
  model: ModelSelection | string,
  selection?: ReasoningSelection | string | null,
  fallbackSelection?: ReasoningSelection | string | null,
): ReasoningEffort | 'auto' | undefined {
  const effectiveSelection = selection ?? fallbackSelection;
  if (!effectiveSelection || effectiveSelection === 'auto') {
    return model === 'auto' ? 'auto' : undefined;
  }
  if (model === 'auto' || modelProvider(model) === 'codex') {
    return effectiveSelection as ReasoningEffort;
  }
  return undefined;
}

export function firstModelForProvider(provider: ProviderKind): ModelId {
  return MODEL_GROUPS.find(group => group.provider === provider)?.models[0]?.id ?? 'claude-sonnet-4-6';
}

export function reasoningEffortsForModel(model: ModelId | string): ReasoningEffortDef[] {
  const supported = ALL_MODELS.find(definition => definition.id === model)?.reasoningEfforts
    ?? STANDARD_CODEX_EFFORTS;
  return CODEX_REASONING_EFFORTS.filter(effort => supported.includes(effort.id));
}

export function reasoningSelectionsForModel(model: ModelId | string): ReasoningSelectionDef[] {
  return [
    {
      id: 'auto',
      label: 'auto',
      title: 'Choose reasoning depth from the task and active Plan item floor',
    },
    ...reasoningEffortsForModel(model),
  ];
}

export function defaultReasoningEffortForModel(model: ModelId | string): ReasoningEffort {
  return ALL_MODELS.find(definition => definition.id === model)?.defaultReasoningEffort ?? 'high';
}

export function normalizeReasoningEffort(
  model: ModelId | string,
  effort?: ReasoningEffort | string | null,
): ReasoningEffort {
  const supported = reasoningEffortsForModel(model);
  if (effort && supported.some(option => option.id === effort)) return effort as ReasoningEffort;
  return defaultReasoningEffortForModel(model);
}
