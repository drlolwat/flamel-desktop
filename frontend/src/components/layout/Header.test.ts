import { describe, expect, it } from 'vitest';
import { settingsTargetFromSearch } from './Header';

describe('settingsTargetFromSearch', () => {
  it('maps Chattle links to the same Assistant object', () => {
    expect(
      settingsTargetFromSearch(
        '?settings=assistant&view=automations&object=automation-4&assistant=assistant-2',
      ),
    ).toBe('assistant:automations:automation-4:assistant-2');
  });

  it('opens the Capability Hub for skills and ignores unrelated queries', () => {
    expect(settingsTargetFromSearch('?settings=integrations&view=skills')).toBe(
      'integrations:skills',
    );
    expect(settingsTargetFromSearch('?view=jobs')).toBeUndefined();
  });
});
