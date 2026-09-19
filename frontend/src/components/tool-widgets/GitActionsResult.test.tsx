import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { GitActionsResult, gitActionsView } from './GitActionsResult';
import { EngineeringToolWidget } from './EngineeringToolWidget';

const run = { id: 1758, run_number: 1758, name: 'Release Desktop', status: 'completed', conclusion: 'success', head_branch: 'main', head_sha: '5d04580082ef67d004d47b56e06c9598d86ddfb2', event: 'workflow_dispatch', html_url: '/lolwat/flamel/actions?run=run-uuid' };
const jobs = { total_count: 2, jobs: [{ id: 4210, name: 'Linux · AppImage', status: 'completed', conclusion: 'success', started_at: '2026-09-06T00:00:00Z', completed_at: '2026-09-06T00:02:03Z', html_url: 'https://git.flamel.dev/lolwat/flamel/actions?run=run-uuid' }, { id: 4211, name: 'Windows', status: 'in_progress', conclusion: null }] };
const output = `Run:\n${JSON.stringify(run)}\n\nJobs:\n${JSON.stringify(jobs)}`;
const render = (value: string, action = 'flamel.run.view') => renderToStaticMarkup(<GitActionsResult result={gitActionsView(value, action)!} action={action} />);

describe('Git Actions result cards', () => {
  it('renders the real Run/Jobs MCP wrapper as readable run metadata and job rows', () => {
    const html = render(output);
    expect(html).toContain('Release Desktop'); expect(html).toContain('Linux · AppImage');
    expect(html).toContain('Passed'); expect(html).toContain('Running'); expect(html).toContain('2m 3s');
    expect(html).toContain('5d045800'); expect(html).toContain('href="https://git.flamel.dev/lolwat/flamel/actions?run=run-uuid"');
    expect(html).not.toContain('total_count'); expect(html).not.toContain('head_sha');
    expect(html).toContain('min-width:0;font-size:11px'); expect(html).toContain('overflow-wrap:anywhere');
  });
  it('routes a full FlamelGit card through the readable body and retains raw disclosure', () => {
    const html = renderToStaticMarkup(<EngineeringToolWidget tool={{ id: 'git', name: 'mcp__flamel__git', input: { action: 'flamel.run.view' }, output, status: 'done' }} defaultExpanded />);
    expect(html).toContain('Run jobs'); expect(html).toContain('Inspect raw data');
  });
  it('supports object/array job wrappers and cancelled, failed, skipped jobs', () => {
    const result = gitActionsView(JSON.stringify({ run, jobs: { job_runs: [{ name: 'Build', conclusion: 'failure' }, { name: 'Docs', conclusion: 'skipped' }, { name: 'Test', conclusion: 'cancelled' }] } }), 'flamel_run_view');
    const html = renderToStaticMarkup(<GitActionsResult result={result!} action="flamel_run_view" />);
    expect(html).toContain('Failed'); expect(html).toContain('Skipped'); expect(html).toContain('Cancelled');
    expect(gitActionsView(`Run:\n${JSON.stringify(run)}\n\nJobs:\n[]`, 'flamel.run.view')?.jobs).toEqual([]);
  });
  it('rejects script/credential URLs while preserving safe labels and escaping markup', () => {
    const html = render(JSON.stringify({ run: { ...run, name: '<script>bad</script>', html_url: 'javascript:alert(1)' }, jobs: [{ name: 'Secrets', html_url: 'https://token:password@example.com', status: 'queued' }] }));
    expect(html).not.toContain('href='); expect(html).not.toContain('<script>'); expect(html).toContain('&lt;script&gt;');
  });
  it('never guesses a Forge host and supports display-title-only run objects', () => {
    const result = gitActionsView(JSON.stringify({ ...run, name: undefined, display_title: 'Actual display title' }), 'flamel.run.view')!;
    expect(renderToStaticMarkup(<GitActionsResult result={result} action="flamel.run.view" />)).not.toContain('href=');
    const html = renderToStaticMarkup(<GitActionsResult result={result} action="flamel.run.view" baseUrl="https://forge.example.org" />);
    expect(html).toContain('Actual display title'); expect(html).toContain('https://forge.example.org/lolwat/flamel/actions?run=run-uuid');
    expect(html).not.toContain('git.flamel.dev');
  });
  it('bounds rows and logs, strips ANSI, and leaves unrelated Git outputs alone', () => {
    const html = render(JSON.stringify({ runs: Array.from({ length: 75 }, (_, i) => ({ name: `Workflow ${i}`, status: 'queued' })) }), 'flamel.run.list');
    expect(html).toContain('first 60'); expect(html).not.toContain('Workflow 60');
    const log = gitActionsView(Array.from({ length: 120 }, (_, i) => `line${i}`).join('\n')+'\n\x1b[31mfailed\x1b[0m', 'flamel.job.logs')?.log;
    expect(log?.split('\n')).toHaveLength(100); expect(log).not.toContain('\x1b'); expect(log).toContain('failed');
    expect(gitActionsView(output, 'repo.status')).toBeUndefined();
    expect(gitActionsView('Run:\n{broken}\n\nJobs:\n[]', 'flamel.run.view')).toBeUndefined();
  });
});
