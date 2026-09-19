import { describe, expect, it } from 'vitest';
import { parseMcpArguments } from './mcpArguments';

describe('custom MCP arguments', () => {
  it('keeps ordinary npx arguments', () => {
    expect(parseMcpArguments('  -y @vendor/mcp --flag  ')).toEqual(['-y', '@vendor/mcp', '--flag']);
  });
  it('preserves quoted paths, empty arguments, and literal command text', () => {
    expect(parseMcpArguments('"/path with spaces/tool.js" --name=\'two words\' "" $(whoami)')).toEqual(['/path with spaces/tool.js', '--name=two words', '', '$(whoami)']);
  });
  it('preserves Windows path separators', () => {
    expect(parseMcpArguments(String.raw`"C:\Program Files\Tools\mcp.js" C:\Users\test`)).toEqual([String.raw`C:\Program Files\Tools\mcp.js`, String.raw`C:\Users\test`]);
  });
  it('accepts an exact JSON argv array', () => {
    const args = ['C:\\directory with spaces\\', '', '"quoted"', '$HOME'];
    expect(parseMcpArguments(JSON.stringify(args))).toEqual(args);
  });
  it('rejects incomplete quotes and malformed JSON instead of saving broken argv', () => {
    for (const input of ['"unfinished', '["unfinished"', '[1]', '[{}]']) {
      expect(() => parseMcpArguments(input)).toThrow();
    }
  });
});
