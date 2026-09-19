export function resolveExecutionSessionId(
  queuedSessionId: string | undefined,
  tabSessionId: string | undefined,
  chatSessionId: string | undefined,
  forceNewSession = false,
): string | undefined {
  if (forceNewSession) return undefined;
  return tabSessionId || chatSessionId || queuedSessionId;
}
