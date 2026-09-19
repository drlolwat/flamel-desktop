import React from 'react';
import { ToolWidgetFields, ToolWidgetSection, ToolWidgetTable } from './ToolWidgetPrimitives';

type RecordValue = Record<string, unknown>;
const record = (value: unknown): RecordValue | undefined => value !== null && typeof value === 'object' && !Array.isArray(value) ? value as RecordValue : undefined;
const text = (value: unknown): string => typeof value === 'string' ? value : typeof value === 'number' && Number.isFinite(value) ? String(value) : '';
const parse = (value: string): unknown => { try { return JSON.parse(value); } catch { return undefined; } };
const list = (value: unknown): RecordValue[] => {
  const wrapper = record(value);
  const items = Array.isArray(value) ? value : wrapper?.jobs ?? wrapper?.job_runs ?? wrapper?.workflow_runs ?? wrapper?.runs;
  return Array.isArray(items) ? items.flatMap(item => record(item) ? [record(item)!] : []) : [];
};

export interface GitActionsView {
  run?: RecordValue;
  jobs?: RecordValue[];
  runs?: RecordValue[];
  log?: string;
}

/** Read only the documented Git result envelopes; never infer fields from logs. */
export function gitActionsView(output: string, action: string): GitActionsView | undefined {
  if (/(?:job[._]logs|run[._]logs)$/.test(action)) {
    const parsed = record(parse(output));
    const content = parsed ? text(parsed.content ?? parsed.logs ?? parsed.log) : output;
    return content ? { log: content.replace(/\x1b\[[0-?]*[ -/]*[@-~]/g, '').split('\n').slice(-100).join('\n') } : undefined;
  }
  if (!/(?:run[._](?:view|list|rerun|cancel)|workflow[._]dispatch)$/.test(action)) return undefined;
  const combined = output.match(/^Run:\s*\n([\s\S]+?)\n\s*Jobs:\s*\n([\s\S]+)$/);
  if (combined) {
    const run = record(parse(combined[1]));
    const jobs = parse(combined[2]);
    if (run && (Array.isArray(jobs) || record(jobs))) return { run, jobs: list(jobs) };
    return undefined;
  }
  const parsed = parse(output);
  const wrapper = record(parsed);
  if (/(?:run[._]list)$/.test(action)) {
    const runs = list(parsed);
    return Array.isArray(parsed) || wrapper?.runs || wrapper?.workflow_runs ? { runs } : undefined;
  }
  if (wrapper?.run && record(wrapper.run)) return { run: record(wrapper.run), jobs: list(wrapper.jobs ?? wrapper.job_runs) };
  if (wrapper && (wrapper.name || wrapper.workflow_name || wrapper.display_title) && wrapper.status) return { run: wrapper, ...(wrapper.jobs ? { jobs: list(wrapper.jobs) } : {}) };
  return undefined;
}

function status(value: RecordValue) {
  const raw = text(value.conclusion) || text(value.status) || 'unknown';
  const states: Record<string, [string, string]> = {
    success: ['passed', 'Passed'], failure: ['failed', 'Failed'], failed: ['failed', 'Failed'],
    in_progress: ['running', 'Running'], running: ['running', 'Running'], queued: ['queued', 'Queued'],
    waiting: ['queued', 'Waiting'], pending: ['queued', 'Pending'], cancelled: ['skipped', 'Cancelled'],
    canceled: ['skipped', 'Cancelled'], skipped: ['skipped', 'Skipped'], completed: ['unknown', 'Completed'],
    timed_out: ['failed', 'Timed out'], action_required: ['queued', 'Needs attention'],
  };
  const [color, label] = states[raw] || ['unknown', raw.replaceAll('_', ' ')];
  return <span className="engineering-widget__check" data-status={color}><span aria-hidden="true" />{label}</span>;
}

function safeLink(value: unknown, baseUrl?: string): string | undefined {
  const candidate = text(value);
  if (!candidate) return undefined;
  try {
    if (candidate.startsWith('//')) return undefined;
    const url = new URL(candidate, baseUrl);
    return ['https:', 'http:'].includes(url.protocol) && !url.username && !url.password ? url.href : undefined;
  } catch { return undefined; }
}

function name(value: RecordValue, fallback: string, baseUrl?: string) {
  const label = text(value.name ?? value.workflow_name ?? value.display_title) || fallback;
  const href = safeLink(value.html_url, baseUrl);
  return href ? <a href={href} target="_blank" rel="noopener noreferrer" style={{ textDecoration: 'underline', textUnderlineOffset: 2 }}>{label}</a> : label;
}

function duration(value: RecordValue): string {
  const start = Date.parse(text(value.started_at ?? value.startedAt));
  const finish = Date.parse(text(value.completed_at ?? value.completedAt));
  if (!Number.isFinite(start) || !Number.isFinite(finish) || finish < start) return '—';
  const seconds = Math.round((finish - start) / 1000);
  return seconds < 60 ? `${seconds}s` : `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
}

export function GitActionsResult({ result, baseUrl }: { result: GitActionsView; action: string; baseUrl?: string }) {
  if (result.log !== undefined) return <ToolWidgetSection title="Job log" description="Last 100 lines · full output is kept in raw data"><pre style={{ margin: 0, maxHeight: 280, overflow: 'auto', whiteSpace: 'pre-wrap', overflowWrap: 'anywhere', fontSize: 11 }}>{result.log}</pre></ToolWidgetSection>;
  const run = result.run;
  const rows = result.runs ?? result.jobs;
  // Relative Forge URLs need their real origin, never the chat client's host.
  const linkBase = safeLink(baseUrl) || safeLink(run?.html_url)
    || rows?.map(row => safeLink(row.html_url)).find(Boolean);
  const runNumber = text(run?.run_number ?? run?.id);
  const runLabel = /^\d+$/.test(runNumber) ? `#${runNumber}` : 'Run details';
  const runLink = safeLink(run?.html_url, linkBase);
  return <div style={{ minWidth: 0, fontSize: 11 }}>
    {run && <ToolWidgetSection title={text(run.name ?? run.workflow_name ?? run.display_title) || 'Workflow run'} action={status(run)}>
      <ToolWidgetFields fields={[
        { label: 'Run', value: runLink ? <a href={runLink} target="_blank" rel="noopener noreferrer" style={{ textDecoration: 'underline', textUnderlineOffset: 2 }}>{runLabel}</a> : runLabel },
        { label: 'Branch', value: text(run.head_branch ?? run.ref_name) || '—' },
        { label: 'Commit', value: text(run.head_sha ?? run.commit_sha).slice(0, 8) || '—', mono: true },
        { label: 'Triggered by', value: text(run.event).replaceAll('_', ' ') || '—' },
      ]} />
    </ToolWidgetSection>}
    {rows && <ToolWidgetSection title={result.runs ? 'Workflow runs' : `Jobs · ${rows.length}`} description={rows.length > 60 ? 'Showing the first 60. Full result is kept in raw data.' : undefined}>
      {rows.length ? <ToolWidgetTable compact caption={result.runs ? 'Workflow runs' : 'Run jobs'} columns={[{ key: 'name', label: result.runs ? 'Workflow' : 'Job' }, { key: 'status', label: 'Status' }, { key: 'duration', label: 'Duration', align: 'right' }]} rows={rows.slice(0, 60).map((job, index) => ({
        id: text(job.id) || String(index), name: name(job, `Job ${index + 1}`, linkBase), status: status(job), duration: duration(job),
      }))} /> : <p style={{ margin: 0, color: 'var(--text-dim)', fontSize: 11 }}>No {result.runs ? 'runs' : 'jobs'} returned yet.</p>}
    </ToolWidgetSection>}
  </div>;
}
