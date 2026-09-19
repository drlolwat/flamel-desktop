import { describe, expect, it } from 'vitest';
import { parseReasoningReroute, reasoningChipLabel, routingTabUpdates } from './chatRouting';

describe('reasoning routing receipts', () => {
  it('never replaces a manual choice with an effective floor or an absent requested value', () => {
    const manual = { model: 'gpt-6-astra', reasoningEffort: 'high' };
    expect({ ...manual, ...routingTabUpdates({ reasoningEffort: 'xhigh', requestedReasoningEffort: null }) }.reasoningEffort).toBe('high');
    expect({ ...manual, ...routingTabUpdates({ model: 'gpt-5.6-sol', reasoningEffort: 'xhigh' }) }.model).toBe('gpt-6-astra');
    const routed = routingTabUpdates({ requestedReasoningEffort: 'high', reasoningEffort: 'xhigh' });
    expect(routed).toEqual({ reasoningEffort: 'high', resolvedReasoningEffort: 'xhigh' });
    expect(reasoningChipLabel('gpt-6-astra', 'high', 'xhigh')).toBe('high');
  });

  it('shows actual Auto effort without turning Auto into a manual selection', () => {
    const routed = routingTabUpdates({ requestedReasoningEffort: 'auto', reasoningEffort: 'xhigh' });
    expect(routed.reasoningEffort).toBe('auto');
    expect(reasoningChipLabel('gpt-6-astra', 'auto', routed.resolvedReasoningEffort)).toBe('Auto · xhigh');
  });

  it('keeps reroute progress account-session scoped and separate from model prose', () => {
    const event = { type: 'reasoning_reroute', sessionId: 'session-1', requestId: 'next-run',
      requestedReasoningEffort: 'auto', note: 'Auto is switching reasoning from high to xhigh and continuing the plan.' };
    expect(parseReasoningReroute(event, 'session-1')?.note).toBe(event.note);
    expect(parseReasoningReroute(event, 'other')).toBeUndefined();
    expect(parseReasoningReroute({ ...event, requestedReasoningEffort: 'high' }, 'session-1')).toBeUndefined();
  });
});
