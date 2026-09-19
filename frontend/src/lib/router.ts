import { useSyncExternalStore } from 'react';

const NAVIGATION_EVENT = 'flamel:navigation';

type NavigateOptions = {
  replace?: boolean;
};

function subscribe(onStoreChange: () => void) {
  window.addEventListener('popstate', onStoreChange);
  window.addEventListener(NAVIGATION_EVENT, onStoreChange);
  return () => {
    window.removeEventListener('popstate', onStoreChange);
    window.removeEventListener(NAVIGATION_EVENT, onStoreChange);
  };
}

function getSnapshot() {
  return `${window.location.pathname}${window.location.search}${window.location.hash}`;
}

export function useBrowserLocation() {
  useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
  return {
    pathname: window.location.pathname,
    search: window.location.search,
    hash: window.location.hash,
  };
}

export function navigate(path: string, options: NavigateOptions = {}) {
  const current = `${window.location.pathname}${window.location.search}${window.location.hash}`;
  if (path === current) return;

  if (options.replace) window.history.replaceState(null, '', path);
  else window.history.pushState(null, '', path);
  window.dispatchEvent(new Event(NAVIGATION_EVENT));
}

export function matchRoute(pattern: string, pathname: string): Record<string, string> | null {
  const patternParts = pattern.split('/').filter(Boolean);
  const pathParts = pathname.split('/').filter(Boolean);
  if (patternParts.length !== pathParts.length) return null;

  const params: Record<string, string> = {};
  for (let index = 0; index < patternParts.length; index += 1) {
    const expected = patternParts[index];
    const actual = pathParts[index];
    if (expected.startsWith(':')) {
      try {
        params[expected.slice(1)] = decodeURIComponent(actual);
      } catch {
        return null;
      }
    } else if (expected !== actual) {
      return null;
    }
  }
  return params;
}
