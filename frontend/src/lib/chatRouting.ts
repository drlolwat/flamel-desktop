import { manualModelSelection, reasoningSelectionsForModel, type ModelId, type ModelSelection, type ReasoningEffort, type ReasoningSelection } from './models';

export type ReasoningReroute = { sessionId: string; requestId: string; note: string };
type RoutingUpdate = {
  model?: ModelSelection;
  reasoningEffort?: ReasoningSelection;
  resolvedModel?: ModelId;
  resolvedReasoningEffort?: ReasoningEffort;
  routingReason?: string;
};
const EFFORTS = ['low', 'medium', 'high', 'xhigh', 'max', 'ultra'];

export function routingTabUpdates(value: unknown): RoutingUpdate {
  if (!value || typeof value !== 'object') return {};
  const routing = value as Record<string, unknown>;
  const updates: RoutingUpdate = {};
  if (typeof routing.requestedModel === 'string' && routing.requestedModel) {
    updates.model = manualModelSelection(routing.requestedModel, typeof routing.model === 'string' ? routing.model : undefined);
  }
  if (typeof routing.requestedReasoningEffort === 'string'
    && ['auto', ...EFFORTS].includes(routing.requestedReasoningEffort)) {
    updates.reasoningEffort = routing.requestedReasoningEffort as ReasoningSelection;
  }
  if (typeof routing.model === 'string' && routing.model && routing.model !== 'auto') updates.resolvedModel = routing.model as ModelId;
  if (typeof routing.reasoningEffort === 'string' && EFFORTS.includes(routing.reasoningEffort)) {
    updates.resolvedReasoningEffort = routing.reasoningEffort as ReasoningEffort;
  }
  if (typeof routing.reason === 'string') updates.routingReason = routing.reason;
  return updates;
}

export function reasoningChipLabel(model: string, requested: ReasoningSelection, effective?: ReasoningEffort): string {
  if (requested === 'auto') return effective ? `Auto · ${effective}` : 'Auto';
  return reasoningSelectionsForModel(model).find(option => option.id === requested)?.label ?? requested;
}

export function parseReasoningReroute(value: unknown, sessionId: string): ReasoningReroute | undefined {
  if (!value || typeof value !== 'object') return undefined;
  const event = value as Record<string, unknown>;
  if (event.type !== 'reasoning_reroute' || event.sessionId !== sessionId
    || event.requestedReasoningEffort !== 'auto' || typeof event.requestId !== 'string'
    || !event.requestId || typeof event.note !== 'string' || !event.note.trim()) return undefined;
  return { sessionId, requestId: event.requestId, note: event.note };
}
