import { describe, expect, it } from 'vitest';
import {
  manualModelSelection,
  modelProvider,
  normalizeReasoningEffort,
  reasoningEffortForSend,
  reasoningEffortsForModel,
  reasoningSelectionsForModel,
} from './models';

describe('GPT-5.6 model catalog', () => {
  it('supports Astra through Codex with its advertised reasoning levels', () => {
    expect(modelProvider('gpt-6-astra')).toBe('codex');
    expect(reasoningEffortsForModel('gpt-6-astra').map(option => option.id)).toEqual([
      'low', 'medium', 'high', 'xhigh', 'max', 'ultra',
    ]);
    expect(normalizeReasoningEffort('gpt-6-astra', 'invalid')).toBe('medium');
    expect(reasoningEffortForSend('gpt-6-astra', 'ultra')).toBe('ultra');
  });
  it('routes every GPT-5.6 variant to Codex', () => {
    expect(modelProvider('gpt-5.6-sol')).toBe('codex');
    expect(modelProvider('gpt-5.6-terra')).toBe('codex');
    expect(modelProvider('gpt-5.6-luna')).toBe('codex');
  });

  it('exposes ultra only on variants that advertise it', () => {
    expect(reasoningEffortsForModel('gpt-5.6-sol').map(option => option.id)).toContain('ultra');
    expect(reasoningEffortsForModel('gpt-5.6-terra').map(option => option.id)).toContain('ultra');
    expect(reasoningEffortsForModel('gpt-5.6-luna').map(option => option.id)).toEqual([
      'low',
      'medium',
      'high',
      'xhigh',
      'max',
    ]);
  });

  it('falls back to each variant default when switching from an unsupported effort', () => {
    expect(normalizeReasoningEffort('gpt-5.6-sol', 'invalid')).toBe('low');
    expect(normalizeReasoningEffort('gpt-5.6-luna', 'ultra')).toBe('medium');
  });

  it('offers automatic reasoning independently of model selection', () => {
    expect(reasoningSelectionsForModel('gpt-5.6-sol').map(option => option.id)).toEqual([
      'auto',
      'low',
      'medium',
      'high',
      'xhigh',
      'max',
      'ultra',
    ]);
  });

  it('preserves manual reasoning for programmatic sends without converting explicit Auto', () => {
    expect(reasoningEffortForSend('gpt-5.6-sol', 'max')).toBe('max');
    expect(reasoningEffortForSend('gpt-5.6-sol', undefined, 'max')).toBe('max');
    expect(reasoningEffortForSend('gpt-5.6-sol', 'xhigh', 'max')).toBe('xhigh');
    expect(reasoningEffortForSend('gpt-5.6-sol', 'auto', 'max')).toBeUndefined();
    expect(reasoningEffortForSend('auto', undefined, 'max')).toBe('max');
    expect(reasoningEffortForSend('auto', 'auto', 'max')).toBe('auto');
    expect(reasoningEffortForSend('claude-opus-4-7', undefined, 'max')).toBeUndefined();
  });

  it('converts hidden automatic routes to a concrete model', () => {
    expect(manualModelSelection('claude-opus-4-7', 'gpt-5.6-sol')).toBe('claude-opus-4-7');
    expect(manualModelSelection('auto', 'claude-sonnet-4-6')).toBe('claude-sonnet-4-6');
    expect(manualModelSelection('auto')).toBe('gpt-5.6-sol');
  });
});
