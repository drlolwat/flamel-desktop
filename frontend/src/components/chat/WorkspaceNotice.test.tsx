import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import WorkspaceNotice from './WorkspaceNotice';

describe('separate workspace notice', () => {
  const workspace = { sourcePath: '/repo', projectPath: '/home/workspace', branch: 'flamel/session-example', baseSha: 'abcd1234', baseBranch: 'main', isolated: true };

  it('shows ownership and a review action, without a merge action', () => {
    const html = renderToStaticMarkup(<WorkspaceNotice sessionId="session" workspace={workspace} />);
    expect(html).toContain('Separate workspace');
    expect(html).toContain('flamel/session-example');
    expect(html).toContain('Review to bring back');
    expect(html).not.toContain('Merge now');
  });

  it('does not label the original folder as isolated', () => {
    expect(renderToStaticMarkup(<WorkspaceNotice sessionId="session" workspace={{ ...workspace, isolated: false }} />)).toBe('');
  });
});
