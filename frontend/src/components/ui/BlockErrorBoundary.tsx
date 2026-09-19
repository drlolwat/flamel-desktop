import React from 'react';

/**
 * Lightweight error boundary for individual chat blocks / UI components.
 * Catches render errors and shows an inline error pill instead of crashing the whole view.
 */
export class BlockErrorBoundary extends React.Component<
  { children: React.ReactNode; label?: string },
  { error: string | null }
> {
  constructor(props: { children: React.ReactNode; label?: string }) {
    super(props);
    this.state = { error: null };
  }

  static getDerivedStateFromError(err: unknown) {
    return { error: err instanceof Error ? err.message : String(err) };
  }

  render() {
    if (this.state.error) {
      const label = this.props.label ?? 'block';
      return (
        <div style={{
          padding: '6px 10px',
          margin: '4px 0',
          background: 'var(--surface)',
          border: '1px solid var(--border-soft)',
          borderLeft: '3px solid var(--error)',
          borderRadius: 'var(--radius-md)',
          fontSize: 11,
          color: 'var(--error)',
          fontFamily: 'var(--font-mono)',
        }}>
          {label} render error: {this.state.error}
        </div>
      );
    }
    return this.props.children;
  }
}
