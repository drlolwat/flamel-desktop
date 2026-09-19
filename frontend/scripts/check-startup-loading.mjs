// Real browser proof for the startup barrier. Only surrounding workspace chrome
// is stubbed; Shell, SessionRuntime, realtime parsing, chat rendering and stores
// are the production code. Every API and WebSocket stays inside this fixture.
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { createServer } from 'vite';

const requireBrowser = createRequire(new URL('../../forge/web/package.json', import.meta.url));
const { chromium, expect: browserExpect } = requireBrowser('@playwright/test');
const expect = browserExpect.configure({ timeout: 20_000 });
const frontend = fileURLToPath(new URL('../', import.meta.url));
const fixture = `
import React from 'react';
import { createRoot } from 'react-dom/client';
import Shell from '/src/components/layout/Shell.tsx';
import StartupOverlay from '/src/components/StartupOverlay.tsx';
import { useTabStore } from '/src/stores/tabStore.ts';
import { useChatStore } from '/src/stores/chatStore.ts';
import { useAppStore } from '/src/stores/appStore.ts';
import { useSessionReadinessStore } from '/src/stores/sessionReadinessStore.ts';
import { chatRuntime } from '/src/lib/chatRuntime.ts';
import '/src/styles/globals.css';
useAppStore.setState({ leftPanelOpen: false, rightPanelOpen: false });
const tab = { id:'test-tab', kind:'chat', sessionId:'test-session', bindingGeneration:0,
  title:'Test chat', projectPath:'/fixture', model:'gpt-6-astra', reasoningEffort:'high',
  ponytailEnabled:false, permissionMode:'default', isStreaming:false };
useTabStore.setState({ tabs:[tab], activeTabId:tab.id });
window.inspectStartup = () => ({
  tab:useTabStore.getState().tabs[0], chat:useChatStore.getState().chats[tab.id],
  readiness:useSessionReadinessStore.getState().entries[tab.id],
  queued:chatRuntime.forTab(tab.id).queue.map(item => item.id),
});
window.openSecondChat = () => useTabStore.getState().addTab({ ...tab, id:'second-tab', sessionId:'second-session' });
createRoot(document.getElementById('root')).render(React.createElement(React.Fragment, null, React.createElement(StartupOverlay), React.createElement(Shell)));
`;
const chrome = new Set(['Header', 'TabBar', 'SettingsModal', 'Sidebar', 'RightPanel', 'StatusBar', 'NewSessionScreen', 'WorkHubView']);
const server = await createServer({
  root: frontend,
  configFile: fileURLToPath(new URL('../vite.config.ts', import.meta.url)),
  server: { host: '127.0.0.1', port: 0 },
  plugins: [{
    name: 'isolated-startup-proof',
    enforce: 'pre',
    resolveId(id, importer) {
      if (id === '/__startup.tsx') return '\0startup.tsx';
      const name = id.split('/').at(-1);
      if (importer?.endsWith('/layout/Shell.tsx') && chrome.has(name)) return '\0empty-chrome.tsx';
      if (importer?.endsWith('/chat/ChatView.tsx') && name === 'InputBar') return '\0input-proof.tsx';
    },
    load(id) {
      if (id === '\0startup.tsx') return fixture;
      if (id === '\0empty-chrome.tsx') return 'export default function Chrome() { return null; }';
      if (id === '\0input-proof.tsx') return `import React from 'react'; export default function Input() { return React.createElement('button', null, 'Fixture composer'); }`;
    },
    configureServer(vite) {
      vite.middlewares.use(async (req, res, next) => {
        if (req.url !== '/__startup') return next();
        res.setHeader('Content-Type', 'text/html');
        res.end(await vite.transformIndexHtml(req.url, '<html><head><meta name="viewport" content="width=device-width, initial-scale=1"></head><body><div id="root"></div><script type="module" src="/__startup.tsx"></script></body></html>'));
      });
    },
  }],
});

const session = id => ({ id, title: 'Test chat', projectPath: '/fixture', model: 'gpt-6-astra',
  requestedModel: 'gpt-6-astra', requestedReasoningEffort: 'high', createdAt: '2026-09-05',
  updatedAt: '2026-09-05', messageCount: 2, pinned: false, inputTokens: 0, outputTokens: 0 });
const bootstrap = id => ({ session: session(id), binding: { sessionId: id, streamSessionId: id,
  activeRequestId: null, activePlanId: null, realtimeSequence: 0 }, activePlan: null,
  messages: [
    { type: 'user', uuid: 'prompt', message: { content: [{ type: 'text', text: 'My saved opening prompt' }] } },
    { type: 'assistant', uuid: 'reply', message: { model: 'gpt-6-astra', content: [{ type: 'text', text: 'Saved assistant response' }] } },
  ],
});

let browser;
try {
  await server.listen();
  const address = server.httpServer.address();
  const origin = `http://127.0.0.1:${address.port}`;
  browser = await chromium.launch({ headless: true });
  for (const scenario of ['slow', 'retry', 'escape']) {
    const failure = scenario !== 'slow';
    const context = await browser.newContext({ serviceWorkers: 'block', ...(failure ? {
      viewport: { width: 390, height: 780 }, isMobile: true, hasTouch: true,
      userAgent: 'Mozilla/5.0 (Linux; Android 14) AppleWebKit/537.36 Chrome/131.0 Mobile Safari/537.36',
    } : {}) });
    const page = await context.newPage();
    const errors = [];
    const forbidden = [];
    const sockets = new Map();
    let releaseHistory;
    const historyGate = new Promise(resolve => { releaseHistory = resolve; });
    let failOnce = failure;
    page.on('pageerror', error => errors.push(error.message));
    await page.routeWebSocket(`${origin.replace('http:', 'ws:')}/api/realtime*`, ws => {
      ws.onMessage(message => {
        const value = JSON.parse(String(message));
        if (value.type === 'subscribe') sockets.set(value.sessionId, ws);
        if (value.type === 'ping') ws.send(JSON.stringify({ type: 'pong' }));
      });
    });
    await page.route('**/*', async route => {
      const url = new URL(route.request().url());
      if (url.origin !== origin) { forbidden.push(url.href); return route.abort(); }
      if (!url.pathname.startsWith('/api/')) return route.continue();
      let body;
      if (url.pathname === '/api/sessions') body = [session('test-session'), session('second-session')];
      else if (url.pathname.endsWith('/bootstrap')) {
        await historyGate;
        if (failOnce) { failOnce = false; return route.fulfill({ status: 503, json: { error: 'Fixture temporarily unavailable' } }); }
        body = bootstrap(url.pathname.split('/')[3]);
      } else if (url.pathname.endsWith('/inputs')) body = { inputs: [{
        type: 'input_delivery', accepted: true, sessionId: url.pathname.split('/')[3],
        requestId: 'accepted-guidance', message: 'Previously accepted guidance', delivery: 'sent',
      }] };
      else if (url.pathname === '/api/realtime/ticket') body = { ticket: 'fixture-only' };
      else if (url.pathname === '/api/terminal') body = { sessions: [] };
      else { forbidden.push(url.pathname); return route.fulfill({ status: 500, json: {} }); }
      await route.fulfill({ json: body });
    });
    await page.goto(`${origin}/__startup`);
    await expect.poll(() => page.evaluate(() => window.inspectStartup?.().readiness?.phase), { timeout: 30_000 }).toBe('history');
    // The previous four-second timeout must not reveal the blank conversation.
    await page.waitForTimeout(4_200);
    await expect(page.locator('.startup-overlay')).toBeVisible();
    await expect(page.getByText('My saved opening prompt', { exact: true })).toHaveCount(0);
    releaseHistory();
    if (failure) {
      await expect(page.getByRole('alert')).toBeVisible();
      await expect(page.locator('.startup-overlay')).toBeVisible();
      if (scenario === 'escape') {
        await page.getByRole('button', { name: 'Open workspace', exact: true }).click();
        await expect(page.locator('.startup-overlay')).toHaveCount(0);
        await expect(page.getByRole('button', { name: 'Fixture composer' })).toHaveCount(0);
      }
      await page.getByRole('button', { name: 'Try again', exact: true }).first().click();
    }
    await expect.poll(() => sockets.has('test-session'), { timeout: 20_000 }).toBe(true);
    const ws = sockets.get('test-session');
    ws.send(JSON.stringify({ type: 'snapshot', sessionId: 'test-session', throughSequence: 0, lines: [
      { type: 'streaming', active: true },
      { type: 'continuity', status: 'active', activeRequestId: 'live-request', isSessionWarm: true, requiresResume: false },
      { type: 'queue_updated', sessionId: 'test-session', requestId: 'queued-request', state: 'queued', message: 'Next queued prompt' },
      { type: 'ask_question', sessionId: 'test-session', runId: 'live-request', requestId: 'question-1', question: 'Which option should I use?', choices: ['First', 'Second'] },
    ] }));
    await expect.poll(() => page.evaluate(() => window.inspectStartup().tab.requestId)).toBe('live-request');
    if (scenario !== 'escape') await expect(page.locator('.startup-overlay')).toBeVisible();
    else await expect(page.getByRole('button', { name: 'Fixture composer' })).toHaveCount(0);
    assert.notEqual(await page.evaluate(() => window.inspectStartup().readiness.phase), 'ready');
    // Run state can change in journal replay after the snapshot. The catch-up
    // marker, not merely 'streaming', is the point at which it is safe to show.
    if (failure) ws.send(JSON.stringify({ type: 'event', replay: true, event: {
      sessionId: 'test-session', runId: 'live-request', sequence: 1,
      payload: { type: 'continuity', status: 'idle', isSessionWarm: true, requiresResume: false },
    } }));
    ws.send(JSON.stringify({ type: 'caughtUp', sessionId: 'test-session', sequence: failure ? 1 : 0 }));
    await expect(page.locator('.startup-overlay')).toHaveCount(0);
    await expect(page.getByText('My saved opening prompt', { exact: true })).toBeVisible();
    await expect(page.getByText('Saved assistant response', { exact: true })).toBeVisible();
    await expect(page.getByText('Previously accepted guidance', { exact: true }).first()).toBeVisible();
    await expect(page.getByText('Which option should I use?', { exact: true })).toBeVisible();
    assert.deepEqual(await page.evaluate(() => window.inspectStartup().queued), ['queued-request']);
    assert.equal(await page.evaluate(() => window.inspectStartup().tab.isStreaming), !failure);
    assert.equal(await page.evaluate(() => window.inspectStartup().readiness.phase), 'ready');
    await expect(page.getByRole('button', { name: 'Fixture composer' })).toBeVisible();
    await page.evaluate(() => window.openSecondChat());
    await expect.poll(() => sockets.has('second-session'), { timeout: 20_000 }).toBe(true);
    await expect(page.getByText('Loading chat and live state…', { exact: true })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Fixture composer' })).toHaveCount(0);
    const second = sockets.get('second-session');
    second.send(JSON.stringify({ type: 'snapshot', sessionId: 'second-session', throughSequence: 0, lines: [
      { type: 'streaming', active: true }, { type: 'continuity', status: 'active', activeRequestId: 'second-run', isSessionWarm: true, requiresResume: false },
    ] }));
    await page.waitForTimeout(100);
    await expect(page.getByRole('button', { name: 'Fixture composer' })).toHaveCount(0);
    second.send(JSON.stringify({ type: 'caughtUp', sessionId: 'second-session', sequence: 0 }));
    await expect(page.getByText('My saved opening prompt', { exact: true })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Fixture composer' })).toBeVisible();
    assert.deepEqual(errors, [], 'Browser rendering errors');
    assert.deepEqual(forbidden, [], 'Unexpected API or external requests');
    console.log(`PASS startup ${scenario}: history + full realtime catch-up ready before reveal`);
    await context.close();
  }
} finally {
  await browser?.close();
  await server.close();
}
