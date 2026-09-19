import { describe, expect, it } from 'vitest';
import { matchRoute } from './router';

describe('matchRoute', () => {
  it('extracts and decodes named path segments', () => {
    expect(matchRoute('/share/:token', '/share/abc%20123')).toEqual({ token: 'abc 123' });
  });

  it('requires the whole path to match', () => {
    expect(matchRoute('/share/:token', '/share/token/extra')).toBeNull();
    expect(matchRoute('/share/:token', '/shared-plans/token')).toBeNull();
  });

  it('rejects malformed URL encoding', () => {
    expect(matchRoute('/share/:token', '/share/%E0%A4%A')).toBeNull();
  });
});
