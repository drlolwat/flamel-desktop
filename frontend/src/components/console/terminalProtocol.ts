export function encodeTerminalInput(value: string): string {
  return btoa(Array.from(new TextEncoder().encode(value), byte => String.fromCharCode(byte)).join(''));
}

export function terminalSocketUrl(base: string, id: string, ticket: string, cursor: number): string {
  const url = new URL(`/api/terminal/connect/${encodeURIComponent(id)}`, base);
  url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
  url.searchParams.set('ticket', ticket);
  if (cursor) url.searchParams.set('afterSequence', String(cursor));
  return url.toString();
}
