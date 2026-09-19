import type { DiagnosticSeverity, WorkbenchDiagnostic } from '@flamel/shared';
import { useAppStore } from '../../stores/appStore';
import { useIdeStore } from '../../stores/ideStore';
import { Icon } from '../ui/Icons';

const SEVERITY_ORDER: Record<DiagnosticSeverity, number> = {
  error: 0,
  warning: 1,
  information: 2,
  hint: 3,
};

function severityColor(severity: DiagnosticSeverity): string {
  if (severity === 'error') return 'var(--error)';
  if (severity === 'warning') return 'var(--warning)';
  if (severity === 'information') return 'var(--info, #6aa9d8)';
  return 'var(--text-dim)';
}

function compactPath(path: string): string {
  return path.split('/').filter(Boolean).slice(-4).join('/');
}

export default function ProblemsPanel() {
  const diagnostics = useIdeStore(state => state.diagnostics);
  const requestReveal = useIdeStore(state => state.requestReveal);
  const setCenterView = useAppStore(state => state.setCenterView);
  const problems = Object.values(diagnostics)
    .flat()
    .sort((left, right) => (
      SEVERITY_ORDER[left.severity] - SEVERITY_ORDER[right.severity]
      || left.path.localeCompare(right.path)
      || left.startLine - right.startLine
      || left.startColumn - right.startColumn
    ));

  const counts = problems.reduce<Record<DiagnosticSeverity, number>>((result, problem) => {
    result[problem.severity] += 1;
    return result;
  }, { error: 0, warning: 0, information: 0, hint: 0 });

  function openProblem(problem: WorkbenchDiagnostic) {
    requestReveal(problem.path, problem.startLine, problem.startColumn);
    setCenterView('ide');
  }

  return (
    <div className="h-full flex flex-col overflow-hidden" style={{ background: 'var(--bg)' }}>
      <div className="flex items-center gap-3 shrink-0" style={{ minHeight: 37, padding: '0 12px', borderBottom: '1px solid var(--border-soft)', background: 'var(--surface)' }}>
        <span style={{ color: 'var(--text)', fontSize: 12, fontWeight: 650 }}>Problems</span>
        {counts.error > 0 && <span style={{ color: 'var(--error)', fontSize: 10 }}>{counts.error} errors</span>}
        {counts.warning > 0 && <span style={{ color: 'var(--warning)', fontSize: 10 }}>{counts.warning} warnings</span>}
        <span style={{ marginLeft: 'auto', color: 'var(--text-dim)', fontSize: 10 }}>{problems.length} total</span>
      </div>

      <div className="flex-1 overflow-y-auto">
        {problems.map(problem => (
          <button
            key={problem.id}
            type="button"
            onClick={() => openProblem(problem)}
            className="w-full text-left"
            style={{ display: 'grid', gridTemplateColumns: '18px minmax(0, 1fr) auto', gap: 8, alignItems: 'start', padding: '9px 12px', borderBottom: '1px solid var(--border-soft)', color: 'var(--text-muted)' }}
          >
            <Icon.Circle size={12} style={{ marginTop: 2, color: severityColor(problem.severity) }} />
            <span style={{ minWidth: 0 }}>
              <span style={{ display: 'block', color: 'var(--text)', fontSize: 11, lineHeight: 1.45 }}>{problem.message}</span>
              <span title={problem.path} style={{ display: 'block', marginTop: 3, color: 'var(--text-dim)', fontFamily: 'var(--font-mono)', fontSize: 9, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                {compactPath(problem.path)}{problem.source ? ` - ${problem.source}` : ''}{problem.code ? ` (${problem.code})` : ''}
              </span>
            </span>
            <code style={{ color: 'var(--text-dim)', fontSize: 9, whiteSpace: 'nowrap' }}>{problem.startLine}:{problem.startColumn}</code>
          </button>
        ))}
        {problems.length === 0 && (
          <div className="h-full flex items-center justify-center" style={{ minHeight: 180, color: 'var(--text-dim)', fontSize: 11 }}>
            No problems detected in open files.
          </div>
        )}
      </div>
    </div>
  );
}
