import React, { useEffect, useMemo, useRef, useState } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import type { PendingQuestion } from '@flamel/shared';
import { api, errorMessageFromUnknown } from '../../lib/api';

interface Props {
  question: PendingQuestion;
  onResolved: () => void;
}

type Choice = { label: string; description?: string };

function normalizeChoice(choice: NonNullable<PendingQuestion['choices']>[number]): Choice {
  return typeof choice === 'string' ? { label: choice } : choice;
}

function isRecommended(label: string) {
  return /\brecommended\b/i.test(label);
}

export function AskQuestionBlock({ question, onResolved }: Props) {
  const choices = useMemo(
    () => (Array.isArray(question.choices) ? question.choices.map(normalizeChoice) : []),
    [question.choices],
  );
  const [freeText, setFreeText] = useState('');
  const [selectedChoices, setSelectedChoices] = useState<Set<string>>(new Set());
  const [submitting, setSubmitting] = useState<'answer' | 'cancel' | null>(null);
  const [error, setError] = useState<string | null>(null);
  const firstControlRef = useRef<HTMLElement | null>(null);
  const freeTextRef = useRef<HTMLInputElement>(null);
  const previousFocusRef = useRef<HTMLElement | null>(null);
  const showFreeText = question.allowFreeText !== false;
  const isMultiSelect = question.multiSelect === true;
  const questionLabelId = 'question-' + question.requestId;
  const statusId = 'question-status-' + question.requestId;

  useEffect(() => {
    previousFocusRef.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const frame = requestAnimationFrame(() => {
      (firstControlRef.current ?? freeTextRef.current)?.focus();
    });
    return () => cancelAnimationFrame(frame);
  }, [question.requestId]);

  function finish() {
    const previousFocus = previousFocusRef.current;
    onResolved();
    requestAnimationFrame(() => {
      if (previousFocus?.isConnected) previousFocus.focus();
    });
  }

  async function submit(answer: string) {
    const trimmed = answer.trim();
    if (!trimmed || submitting) return;
    setSubmitting('answer');
    setError(null);
    try {
      await api.chat.ask.answer(question.requestId, trimmed);
      finish();
    } catch (cause) {
      setSubmitting(null);
      setError(errorMessageFromUnknown(cause, 'Could not send the answer. Try again.'));
    }
  }

  async function cancel() {
    if (submitting) return;
    setSubmitting('cancel');
    setError(null);
    try {
      await api.chat.ask.cancel(question.requestId);
      finish();
    } catch (cause) {
      setSubmitting(null);
      setError(errorMessageFromUnknown(cause, 'Could not cancel the question. Try again.'));
    }
  }

  async function submitMulti() {
    const answer = [...selectedChoices, freeText.trim()].filter(Boolean).join('; ');
    await submit(answer);
  }

  function toggleChoice(choice: string) {
    setSelectedChoices((selected) => {
      const next = new Set(selected);
      if (next.has(choice)) next.delete(choice);
      else next.add(choice);
      return next;
    });
  }

  const dangerous = [
    /\brm\s+-rf\b/,
    /\bsudo\b/,
    /\bdrop\s+(table|database)\b/i,
    /\bchmod\s+777\b/,
    /\bdelete\s+all\b/i,
    /\bwipe\b/i,
  ].some((pattern) => pattern.test(question.question));
  const canSubmitMulti = selectedChoices.size > 0 || (showFreeText && freeText.trim().length > 0);

  return (
    <section
      aria-labelledby={questionLabelId}
      aria-describedby={error ? statusId : undefined}
      aria-busy={submitting !== null}
      style={{
        background: 'var(--surface)',
        border: dangerous ? '1px solid var(--error)' : '1px solid var(--border)',
        borderLeft: dangerous ? '3px solid var(--error)' : '3px solid var(--accent)',
        borderRadius: 'var(--radius-lg)',
        boxShadow: '0 10px 30px rgba(0,0,0,0.18)',
        overflow: 'hidden',
      }}
    >
      <header
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 10,
          padding: '9px 12px',
          borderBottom: '1px solid var(--border-soft)',
          background: 'var(--surface-2)',
        }}
      >
        <span
          aria-hidden="true"
          style={{
            width: 7,
            height: 7,
            borderRadius: '50%',
            background: 'var(--accent)',
          }}
        />
        <span
          style={{
            flex: 1,
            color: 'var(--text-muted)',
            fontSize: 11,
            fontWeight: 700,
          }}
        >
          Input required
        </span>
        <button
          type="button"
          onClick={cancel}
          disabled={submitting !== null}
          style={{
            padding: '4px 8px',
            border: '1px solid var(--border-soft)',
            borderRadius: 'var(--radius-sm)',
            background: 'transparent',
            color: 'var(--text-muted)',
            cursor: submitting ? 'not-allowed' : 'pointer',
            fontSize: 11,
          }}
        >
          {submitting === 'cancel' ? 'Cancelling…' : 'Cancel'}
        </button>
      </header>

      <div
        style={{
          display: 'grid',
          gap: 12,
          padding: '12px clamp(12px, 2vw, 16px)',
        }}
      >
        {dangerous && (
          <p role="note" style={{ margin: 0, color: 'var(--error)', fontSize: 11 }}>
            Potentially destructive operation
          </p>
        )}

        <p
          id={questionLabelId}
          style={{
            margin: 0,
            color: 'var(--text)',
            fontSize: 13,
            lineHeight: 1.55,
          }}
        >
          {question.question}
        </p>

        {choices.length > 0 && !isMultiSelect && (
          <div role="group" aria-label="Answer choices" style={{ display: 'grid', gap: 7 }}>
            {choices.map((choice, index) => {
              const descriptionId = choice.description ? question.requestId + '-choice-' + index : undefined;
              return (
                <button
                  key={choice.label + '-' + index}
                  ref={
                    index === 0
                      ? (node) => {
                          firstControlRef.current = node;
                        }
                      : undefined
                  }
                  type="button"
                  onClick={() => submit(choice.label)}
                  disabled={submitting !== null}
                  aria-describedby={descriptionId}
                  style={{
                    display: 'grid',
                    gridTemplateColumns: 'minmax(0, 1fr) auto',
                    gap: 6,
                    width: '100%',
                    padding: '9px 10px',
                    textAlign: 'left',
                    border: isRecommended(choice.label) ? '1px solid var(--accent)' : '1px solid var(--border-soft)',
                    borderRadius: 'var(--radius-md)',
                    background: isRecommended(choice.label) ? 'var(--accent-glow)' : 'var(--surface-2)',
                    color: 'var(--text)',
                    cursor: submitting ? 'not-allowed' : 'pointer',
                    opacity: submitting ? 0.6 : 1,
                  }}
                >
                  <span style={{ fontSize: 12, fontWeight: 650 }}>{choice.label}</span>
                  {isRecommended(choice.label) && (
                    <span
                      aria-hidden="true"
                      style={{
                        color: 'var(--accent)',
                        fontSize: 9,
                        fontWeight: 700,
                      }}
                    >
                      BEST FIT
                    </span>
                  )}
                  {choice.description && (
                    <span
                      id={descriptionId}
                      style={{
                        gridColumn: '1 / -1',
                        color: 'var(--text-muted)',
                        fontSize: 11,
                        lineHeight: 1.4,
                      }}
                    >
                      {choice.description}
                    </span>
                  )}
                </button>
              );
            })}
          </div>
        )}

        {choices.length > 0 && isMultiSelect && (
          <fieldset
            style={{
              display: 'grid',
              gap: 7,
              margin: 0,
              padding: 0,
              border: 0,
            }}
            disabled={submitting !== null}
          >
            <legend className="sr-only">Answer choices</legend>
            {choices.map((choice, index) => (
              <label
                key={choice.label + '-' + index}
                style={{
                  display: 'grid',
                  gridTemplateColumns: 'auto minmax(0, 1fr)',
                  gap: '2px 8px',
                  alignItems: 'start',
                  padding: '8px 10px',
                  border: selectedChoices.has(choice.label)
                    ? '1px solid var(--accent)'
                    : '1px solid var(--border-soft)',
                  borderRadius: 'var(--radius-md)',
                  background: selectedChoices.has(choice.label) ? 'var(--accent-glow)' : 'var(--surface-2)',
                  cursor: submitting ? 'not-allowed' : 'pointer',
                }}
              >
                <input
                  ref={
                    index === 0
                      ? (node) => {
                          firstControlRef.current = node;
                        }
                      : undefined
                  }
                  type="checkbox"
                  checked={selectedChoices.has(choice.label)}
                  onChange={() => toggleChoice(choice.label)}
                  style={{ marginTop: 2, accentColor: 'var(--accent)' }}
                />
                <span
                  style={{
                    color: 'var(--text)',
                    fontSize: 12,
                    fontWeight: 650,
                  }}
                >
                  {choice.label}
                </span>
                {choice.description && (
                  <span
                    style={{
                      gridColumn: 2,
                      color: 'var(--text-muted)',
                      fontSize: 11,
                    }}
                  >
                    {choice.description}
                  </span>
                )}
              </label>
            ))}
          </fieldset>
        )}

        {showFreeText && (
          <form
            onSubmit={(event) => {
              event.preventDefault();
              if (isMultiSelect) void submitMulti();
              else void submit(freeText);
            }}
            style={{ display: 'flex', flexWrap: 'wrap', gap: 7 }}
          >
            <label className="sr-only" htmlFor={question.requestId + '-answer'}>
              Custom answer
            </label>
            <input
              id={question.requestId + '-answer'}
              ref={(node) => {
                freeTextRef.current = node;
                if (choices.length === 0) firstControlRef.current = node;
              }}
              value={freeText}
              onChange={(event) => setFreeText(event.target.value)}
              disabled={submitting !== null}
              placeholder={choices.length ? 'Or type a custom answer…' : 'Type your answer…'}
              style={{
                flex: '1 1 220px',
                minWidth: 0,
                padding: '8px 10px',
                border: '1px solid var(--border)',
                borderRadius: 'var(--radius-md)',
                background: 'var(--surface-2)',
                color: 'var(--text)',
                fontSize: 12,
              }}
            />
            <button
              type="submit"
              disabled={submitting !== null || (isMultiSelect ? !canSubmitMulti : !freeText.trim())}
              style={{
                padding: '8px 13px',
                border: '1px solid transparent',
                borderRadius: 'var(--radius-md)',
                background: 'var(--accent)',
                color: '#fff',
                cursor: submitting ? 'not-allowed' : 'pointer',
                opacity: submitting !== null || (isMultiSelect ? !canSubmitMulti : !freeText.trim()) ? 0.45 : 1,
                fontSize: 12,
                fontWeight: 650,
              }}
            >
              {submitting === 'answer' ? 'Sending…' : isMultiSelect ? 'Submit choices' : 'Send'}
            </button>
          </form>
        )}

        {isMultiSelect && !showFreeText && (
          <button
            type="button"
            onClick={() => void submitMulti()}
            disabled={submitting !== null || !canSubmitMulti}
            style={{
              justifySelf: 'start',
              padding: '8px 13px',
              border: 0,
              borderRadius: 'var(--radius-md)',
              background: 'var(--accent)',
              color: '#fff',
              opacity: canSubmitMulti ? 1 : 0.45,
            }}
          >
            {submitting === 'answer' ? 'Sending…' : 'Submit choices'}
          </button>
        )}

        <p
          id={statusId}
          role={error ? 'alert' : 'status'}
          aria-live="polite"
          style={{
            display: error || submitting ? 'block' : 'none',
            margin: 0,
            color: error ? 'var(--error)' : 'var(--text-muted)',
            fontSize: 11,
          }}
        >
          {error ?? (submitting ? 'Saving your response…' : '')}
        </p>

        {question.previewMarkdown && (
          <div
            style={{
              borderTop: '1px solid var(--border-soft)',
              paddingTop: 10,
              color: 'var(--text-muted)',
              fontSize: 12,
              lineHeight: 1.65,
              overflow: 'auto',
              maxHeight: 260,
            }}
          >
            <div
              style={{
                marginBottom: 7,
                color: 'var(--text-dim)',
                fontSize: 9,
                fontWeight: 700,
                letterSpacing: '0.06em',
                textTransform: 'uppercase',
              }}
            >
              Preview
            </div>
            <ReactMarkdown remarkPlugins={[remarkGfm]}>{question.previewMarkdown}</ReactMarkdown>
          </div>
        )}
      </div>
    </section>
  );
}
