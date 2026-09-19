import { useState, useRef, useCallback, useEffect, useMemo } from 'react';
import { detectPlanningMode, stripKeyword } from '../../utils/ultraplanKeyword';
import { useChat } from '../../hooks/useChat';
import { isAutonomousOrchestrationTab, useTabStore } from '../../stores/tabStore';
import { useChatStore } from '../../stores/chatStore';
import type { ModelId, PermissionMode } from '../../stores/tabStore';
import { api, apiFetch } from '../../lib/api';
import {
  MODEL_GROUPS,
  firstModelForProvider,
  modelLabel,
  modelProvider,
  normalizeReasoningEffort,
  reasoningEffortForSend,
  reasoningSelectionsForModel,
  type ProviderKind,
} from '../../lib/models';
import { applyCompactionEventToStores, clearCompactionBlock } from '../../lib/compactionEvents';
import { streamCompactSession } from '../../lib/sessionCompaction';
import { Icon } from '../ui/Icons';
import { useProjectFileStore } from '../../stores/projectFileStore';
import { reasoningChipLabel } from '../../lib/chatRouting';

const ACCEPTED_IMAGE_TYPES = ['image/png', 'image/jpeg', 'image/gif', 'image/webp'];

function providerLabel(provider: ProviderKind): string {
  return provider === 'codex' ? 'OpenAI Codex' : 'Claude';
}


type CompactCommand =
  | { kind: 'compact' }
  | { kind: 'compact-and'; prompt: string }
  | { kind: 'fork-and-compact'; prompt: string };

function parseCompactCommand(raw: string): CompactCommand | null {
  const trimmed = raw.trim();
  if (trimmed === '/compact') return { kind: 'compact' };
  if (trimmed.startsWith('/compact-and ')) {
    const prompt = trimmed.slice('/compact-and '.length).trim();
    return prompt ? { kind: 'compact-and', prompt } : null;
  }
  if (trimmed.startsWith('/fork-and-compact ')) {
    const prompt = trimmed.slice('/fork-and-compact '.length).trim();
    return prompt ? { kind: 'fork-and-compact', prompt } : null;
  }
  return null;
}


interface ImagePayload { data: string; mediaType: string; name: string }
interface AttachedImage extends ImagePayload { objectUrl: string }

interface Props {
  tabId: string;
  projectPath?: string;
  onViewChanges?: () => void;
}

export default function InputBar({ tabId, projectPath, onViewChanges }: Props) {
  const [value, setValue] = useState('');
  const [images, setImages] = useState<AttachedImage[]>([]);
  const [showModelPicker, setShowModelPicker] = useState(false);
  const [showReasoningPicker, setShowReasoningPicker] = useState(false);
  const [showRunnerPicker, setShowRunnerPicker] = useState(false);
  const [showQueuePanel, setShowQueuePanel] = useState(false);
  const [editingQueuedId, setEditingQueuedId] = useState<string | null>(null);
  const [queuedDraft, setQueuedDraft] = useState('');
  const [queuedEditError, setQueuedEditError] = useState<string | null>(null);
  const [savingQueuedMessage, setSavingQueuedMessage] = useState(false);
  const [showEnterChoice, setShowEnterChoice] = useState(false);
  const [runners, setRunners] = useState<{ id: string; name: string; online?: boolean }[]>([]);
  const [focused, setFocused] = useState(false);
  const [claudeConnected, setClaudeConnected] = useState<boolean | null>(null);
  const [codexConnected, setCodexConnected] = useState<boolean | null>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const { tabs, activeTabId, updateTab, addTab, setActiveTab } = useTabStore();
  const gitStatus = useProjectFileStore(s => projectPath ? s.projects[projectPath]?.gitStatus ?? null : null);
  const gitLoadedAt = useProjectFileStore(s => projectPath ? s.projects[projectPath]?.gitLoadedAt ?? null : null);
  const refreshGitStatus = useProjectFileStore(s => s.refreshGitStatus);
  const { getChat } = useChatStore();
  const {
    sendMessage,
    steerMessage,
    abort,
    loadHistory,
    queuedCount,
    queuedMessages,
    updateQueuedMessage,
    removeQueuedMessage,
    interruptWithMessage,
    isActive: isChatActive,
  } = useChat(tabId);

  const tab = tabs.find(t => t.id === tabId);
  const isOrchestrationOwned = isAutonomousOrchestrationTab(tab);
  const chat = getChat(tabId);

  async function readImageFile(file: File): Promise<AttachedImage | null> {
    if (!ACCEPTED_IMAGE_TYPES.includes(file.type)) return null;
    return new Promise(resolve => {
      const reader = new FileReader();
      reader.onload = e => {
        const result = e.target?.result as string;
        const base64 = result.split(',')[1];
        const objectUrl = URL.createObjectURL(file);
        resolve({ data: base64, mediaType: file.type, name: file.name, objectUrl });
      };
      reader.onerror = () => resolve(null);
      reader.readAsDataURL(file);
    });
  }

  const addImagePayloads = useCallback((payloads: ImagePayload[]) => {
    setImages(prev => [
      ...prev,
      ...payloads.map(payload => ({
        ...payload,
        objectUrl: `data:${payload.mediaType};base64,${payload.data}`,
      })),
    ]);
  }, []);

  const addImages = useCallback(async (files: FileList | File[]) => {
    const arr = Array.from(files);
    const results = await Promise.all(arr.map(readImageFile));
    const valid = results.filter(Boolean) as AttachedImage[];
    setImages(prev => [...prev, ...valid]);
  }, []);

  // Auth status for both providers
  useEffect(() => {
    const ac = new AbortController();
    apiFetch('/api/me/claude-auth', { credentials: 'include', signal: ac.signal })
      .then(r => r.ok ? r.json() : null)
      .then(d => { if (d) setClaudeConnected(d.connected); })
      .catch(() => {});
    apiFetch('/api/me/codex-auth', { credentials: 'include', signal: ac.signal })
      .then(r => r.ok ? r.json() : null)
      .then(d => { if (d) setCodexConnected(d.connected); })
      .catch(() => {});

    const onClaudeConnected = () => setClaudeConnected(true);
    const onCodexConnected = () => setCodexConnected(true);
    window.addEventListener('flamel:claude-connected', onClaudeConnected);
    window.addEventListener('flamel:codex-connected', onCodexConnected);
    return () => {
      ac.abort();
      window.removeEventListener('flamel:claude-connected', onClaudeConnected);
      window.removeEventListener('flamel:codex-connected', onCodexConnected);
    };
  }, []);

  // Fetch available runners
  useEffect(() => {
    api.runners.list()
      .then(list => setRunners(list.map((r: any) => ({ id: r.id, name: r.name, online: r.online }))))
      .catch(() => {});
  }, []);

  // Git status lives in projectFileStore so tool-result refreshes update the chip while streaming.
  useEffect(() => {
    if (!projectPath || tabId !== activeTabId) return;
    if (gitLoadedAt && Date.now() - gitLoadedAt < 30_000) return;
    refreshGitStatus(projectPath);
  }, [projectPath, gitLoadedAt, refreshGitStatus, tabId, activeTabId]);

  // Listen for input/send events — only the active tab's InputBar responds
  useEffect(() => {
    if (tabId !== activeTabId) return;

    function onSetInput(e: Event) {
      if (isOrchestrationOwned) return;
      const { message } = (e as CustomEvent).detail;
      setValue(message);
      setTimeout(() => { textareaRef.current?.focus(); autoResize(); }, 0);
    }

    async function onSendMessage(e: Event) {
      if (isOrchestrationOwned) return;
      const detail = (e as CustomEvent).detail || {};
      if (detail.tabId && detail.tabId !== tabId) return;
      const {
        message,
        projectPath: msgPath,
        model,
        reasoningEffort,
        ponytailEnabled,
        permissionMode,
        sessionId,
        runnerId,
        images,
        planningMode,
        compactFirst,
        loadHistoryFirst,
        recoveryStrategy,
      } = detail;
      const msg = (message as string).trim();
      if (!msg || !msgPath) return;
      const effectiveModel = model || tab?.model;
      const effectiveReasoningEffort = effectiveModel
        ? reasoningEffortForSend(effectiveModel, reasoningEffort, tab?.reasoningEffort)
        : undefined;
      const effectivePonytailEnabled = ponytailEnabled ?? tab?.ponytailEnabled ?? false;
      const effectivePermissionMode = permissionMode || tab?.permissionMode;
      if (!effectiveModel || !effectivePermissionMode) return;
      const request = {
        message: msg,
        workingDirectory: msgPath,
        options: {
          model: effectiveModel,
          reasoningEffort: effectiveReasoningEffort,
          ponytailEnabled: effectivePonytailEnabled,
          permissionMode: effectivePermissionMode,
          sessionId,
          runnerId,
          images,
          planningMode,
        },
      };
      if (loadHistoryFirst && sessionId) await loadHistory(sessionId);
      if (compactFirst && sessionId) {
        await compactAndSend(request, recoveryStrategy || 'manual_compact_and');
      } else {
        sendMessage(msg, msgPath, request.options);
      }
    }

    function onAttachImages(e: Event) {
      if (isOrchestrationOwned) return;
      const { files, tabId: target } = (e as CustomEvent).detail;
      if (target !== tabId) return;
      addImages(files as File[]);
    }

    window.addEventListener('flamel:set-input', onSetInput);
    window.addEventListener('flamel:send-message', onSendMessage);
    window.addEventListener('flamel:attach-images', onAttachImages);
    return () => {
      window.removeEventListener('flamel:set-input', onSetInput);
      window.removeEventListener('flamel:send-message', onSendMessage);
      window.removeEventListener('flamel:attach-images', onAttachImages);
    };
  }, [
    tabId,
    activeTabId,
    sendMessage,
    loadHistory,
    addImages,
    isOrchestrationOwned,
    tab?.model,
    tab?.reasoningEffort,
    tab?.ponytailEnabled,
    tab?.permissionMode,
  ]);

  if (!tab) return null;

  const isStreaming = tab.isStreaming || isChatActive;
  const isWaiting = chat.isWaiting;
  const hasPendingQuestion = !!chat.pendingQuestion;

  function autoResize() {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = 'auto';
    el.style.height = Math.min(el.scrollHeight, 160) + 'px';
  }

  function removeImage(i: number) {
    setImages(prev => {
      const objectUrl = prev[i]?.objectUrl;
      if (objectUrl?.startsWith('blob:')) URL.revokeObjectURL(objectUrl);
      return prev.filter((_, idx) => idx !== i);
    });
  }

  const ingestClipboardImages = useCallback(async (clipboardData: DataTransfer | null, preventDefault: () => void) => {
    if (!clipboardData) return false;
    const fileImages = Array.from(clipboardData.files).filter(f => ACCEPTED_IMAGE_TYPES.includes(f.type));
    const itemImages = Array.from(clipboardData.items)
      .filter(item => item.kind === 'file' && ACCEPTED_IMAGE_TYPES.includes(item.type))
      .map(item => item.getAsFile())
      .filter(Boolean) as File[];
    const files = fileImages.length > 0 ? fileImages : itemImages;
    if (files.length > 0) {
      preventDefault();
      await addImages(files);
      return true;
    }

    const types = Array.from(clipboardData.types);
    const hasImageHint = types.includes('Files') || types.some(type => type.startsWith('image/'));
    const hasPlainTextOnly = types.includes('text/plain') && !hasImageHint;
    if (hasPlainTextOnly || !window.electronAPI?.readClipboardImage) return false;
    preventDefault();
    const nativeImage = await window.electronAPI.readClipboardImage();
    if (!nativeImage) return false;
    addImagePayloads([nativeImage]);
    return true;
  }, [addImages, addImagePayloads]);

  const handlePaste = useCallback((e: React.ClipboardEvent) => {
    if (isOrchestrationOwned) return;
    void ingestClipboardImages(e.clipboardData, () => e.preventDefault());
  }, [ingestClipboardImages, isOrchestrationOwned]);

  useEffect(() => {
    if (tabId !== activeTabId || isOrchestrationOwned) return;
    const onPaste = (e: ClipboardEvent) => {
      const target = e.target as HTMLElement | null;
      if (target?.closest('input, textarea, [contenteditable="true"]')) return;
      void ingestClipboardImages(e.clipboardData, () => e.preventDefault());
    };
    window.addEventListener('paste', onPaste);
    return () => window.removeEventListener('paste', onPaste);
  }, [tabId, activeTabId, ingestClipboardImages, isOrchestrationOwned]);

  const planningMode = useMemo(() => detectPlanningMode(value), [value]);

  function clearDraft() {
    setValue('');
    setImages([]);
    setShowEnterChoice(false);
    if (textareaRef.current) textareaRef.current.style.height = 'auto';
  }

  function buildSendRequest(messageOverride?: string) {
    if (isOrchestrationOwned) return null;
    const rawMsg = (messageOverride ?? value).trim();
    if ((!rawMsg && images.length === 0) || !tab?.projectPath) return null;
    const mode = detectPlanningMode(rawMsg);
    const msg = mode ? stripKeyword(rawMsg) : rawMsg;
    const imgs = images.map(({ data, mediaType, name }) => ({ data, mediaType, name }));
    return {
      message: msg,
      workingDirectory: tab.projectPath,
      options: {
        model: tab.model,
        reasoningEffort: reasoningEffortForSend(tab.model, tab.reasoningEffort),
        permissionMode: tab.permissionMode,
        ponytailEnabled: tab.ponytailEnabled,
        sessionId: tab?.sessionId || chat.sessionId,
        images: imgs.length > 0 ? imgs : undefined,
        planningMode: mode ?? undefined,
        runnerId: tab.runnerId || undefined,
      },
    };
  }

  async function compactSessionForCommand(
    targetTabId: string,
    sessionId: string,
    queuedPrompt: boolean,
    recoveryStrategy: string,
  ) {
    updateTab(targetTabId, { isStreaming: true, currentTool: 'Collecting context...' });
    const completed = await streamCompactSession(sessionId, {
      queuedPrompt,
      recoveryStrategy,
      onEvent: event => applyCompactionEventToStores(targetTabId, event),
    });
    if (!completed) throw new Error('Compaction failed');
  }

  async function compactAndSend(
    request: NonNullable<ReturnType<typeof buildSendRequest>>,
    recoveryStrategy: string,
  ) {
    const sessionId = request.options.sessionId;
    if (!sessionId) {
      sendMessage(request.message, request.workingDirectory, request.options);
      return;
    }
    try {
      await compactSessionForCommand(tabId, sessionId, true, recoveryStrategy);
      sendMessage(request.message, request.workingDirectory, request.options);
    } catch (error) {
      clearCompactionBlock(tabId);
      updateTab(tabId, { isStreaming: false, currentTool: undefined });
      setValue(request.message);
      setTimeout(() => { textareaRef.current?.focus(); autoResize(); }, 0);
      console.warn('compact-and failed', error);
    }
  }

  async function forkAndCompactAndSend(request: NonNullable<ReturnType<typeof buildSendRequest>>) {
    const sourceSessionId = tab?.sessionId || chat.sessionId;
    if (!sourceSessionId || !tab?.projectPath) {
      sendMessage(request.message, request.workingDirectory, request.options);
      return;
    }
    try {
      updateTab(tabId, { isStreaming: true, currentTool: 'Forking session...' });
      const response = await apiFetch(`/api/sessions/${sourceSessionId}/fork`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ title: `${tab.title || 'Session'} fork` }),
      });
      if (!response.ok) throw new Error(`${response.status} ${await response.text()}`);
      const fork = await response.json();
      const forkSessionId = String(fork.id || '');
      if (!forkSessionId) throw new Error('Fork response did not include a session id');
      const forkTabId = addTab({
        sessionId: forkSessionId,
        projectPath: typeof fork.projectPath === 'string' ? fork.projectPath : request.workingDirectory,
        title: typeof fork.title === 'string' ? fork.title : `${tab.title || 'Session'} fork`,
        model: (typeof fork.model === 'string' ? fork.model : tab.model) as ModelId,
        reasoningEffort: tab.reasoningEffort,
        ponytailEnabled: tab.ponytailEnabled,
        permissionMode: tab.permissionMode,
        runnerId: tab.runnerId,
        runnerName: tab.runnerName,
      });
      useChatStore.getState().setSessionId(forkTabId, forkSessionId);
      updateTab(tabId, { isStreaming: false, currentTool: undefined });
      window.setTimeout(() => {
        window.dispatchEvent(new CustomEvent('flamel:send-message', {
          detail: {
            tabId: forkTabId,
            message: request.message,
            projectPath: typeof fork.projectPath === 'string' ? fork.projectPath : request.workingDirectory,
            model: typeof fork.model === 'string' ? fork.model : tab.model,
            reasoningEffort: tab.reasoningEffort,
            ponytailEnabled: tab.ponytailEnabled,
            permissionMode: tab.permissionMode,
            sessionId: forkSessionId,
            runnerId: tab.runnerId || undefined,
            images: request.options.images,
            planningMode: request.options.planningMode,
            compactFirst: true,
            loadHistoryFirst: true,
            recoveryStrategy: 'fork_and_compact',
          },
        }));
      }, 40);
    } catch (error) {
      updateTab(tabId, { isStreaming: false, currentTool: undefined });
      setValue(`/fork-and-compact ${request.message}`);
      setTimeout(() => { textareaRef.current?.focus(); autoResize(); }, 0);
      console.warn('fork-and-compact failed', error);
    }
  }

  async function handleSend(queueOnly = false) {
    if (isOrchestrationOwned) return;
    if ((!value.trim() && images.length === 0) || !tab?.projectPath || providerNotConnected) return;
    const rawMsg = value.trim();

    // /btw <question> — side question without disturbing the main session
    if (rawMsg.startsWith('/btw ')) {
      const question = rawMsg.slice(5).trim();
      if (question) {
        clearDraft();
        window.dispatchEvent(new CustomEvent('flamel:btw', {
          detail: {
            tabId,
            question,
            sessionId: tab?.sessionId || chat.sessionId,
            workingDirectory: tab.projectPath,
          },
        }));
      }
      return;
    }

    const compactCommand = parseCompactCommand(rawMsg);
    if (compactCommand) {
      if (isStreaming) return;
      if (compactCommand.kind === 'compact') {
        const sessionId = tab.sessionId || chat.sessionId;
        if (!sessionId) return;
        clearDraft();
        try {
          await compactSessionForCommand(tabId, sessionId, false, 'manual_compaction');
        } catch (error) {
          clearCompactionBlock(tabId);
          updateTab(tabId, { isStreaming: false, currentTool: undefined });
          setValue('/compact');
          setTimeout(() => { textareaRef.current?.focus(); autoResize(); }, 0);
          console.warn('compact failed', error);
        }
        return;
      }

      const request = buildSendRequest(compactCommand.prompt);
      if (!request) return;
      clearDraft();
      if (compactCommand.kind === 'compact-and') {
        await compactAndSend(request, 'manual_compact_and');
      } else {
        await forkAndCompactAndSend(request);
      }
      return;
    }

    const request = buildSendRequest();
    if (!request) return;
    clearDraft();
    try {
      if (isStreaming && !queueOnly) {
        await steerMessage(request.message, request.workingDirectory, request.options);
      } else {
        sendMessage(request.message, request.workingDirectory, request.options);
      }
    } catch {
      // The failed receipt explains why. Keep the text editable without erasing a newer draft.
      setValue(current => current || request.message);
    }
  }

  function handleInterruptSend() {
    if (isOrchestrationOwned || providerNotConnected) return;
    const request = buildSendRequest();
    if (!request) return;
    clearDraft();
    interruptWithMessage(request.message, request.workingDirectory, request.options);
  }

  function handleKeyDown(e: React.KeyboardEvent) {
    if (showEnterChoice) {
      if (e.key === 'q' || e.key === 'Q') {
        e.preventDefault();
        setShowEnterChoice(false);
        handleSend(true);
        return;
      }
      if (e.key === 'i' || e.key === 'I') {
        e.preventDefault();
        setShowEnterChoice(false);
        handleInterruptSend();
        return;
      }
    }
    if (e.key === 'Enter' && !e.shiftKey) {
      if (e.nativeEvent.isComposing) return;
      e.preventDefault();
      handleSend(e.ctrlKey || e.metaKey);
    }
    if (e.key === 'Escape') {
      if (showEnterChoice) {
        setShowEnterChoice(false);
        return;
      }
      if (editingQueuedId) {
        setEditingQueuedId(null);
        return;
      }
      if (showQueuePanel) {
        setShowQueuePanel(false);
        return;
      }
      if (isStreaming) abort();
    }
  }

  // Autofocus input when Claude is waiting for a reply
  useEffect(() => {
    if (isWaiting) textareaRef.current?.focus();
  }, [isWaiting]);

  // Focus textarea when the abort button fires (stop+redirect)
  useEffect(() => {
    const handler = (e: Event) => {
      const detail = (e as CustomEvent).detail;
      if (detail?.tabId !== tabId) return;
      textareaRef.current?.focus();
    };
    window.addEventListener('flamel:focus-input', handler);
    return () => window.removeEventListener('flamel:focus-input', handler);
  }, [tabId]);

  const lastBlock = chat.blocks[chat.blocks.length - 1];
  const wasInterrupted = !isStreaming && lastBlock?.stopReason === 'user_interrupted';
  const activeProvider = tab && tab.model !== 'auto'
    ? modelProvider(tab.model)
    : tab?.resolvedModel
      ? modelProvider(tab.resolvedModel)
      : 'codex';
  const providerNotConnected = tab?.model === 'auto'
    ? claudeConnected === false && codexConnected === false
    : (activeProvider === 'claude' && claudeConnected === false) ||
      (activeProvider === 'codex' && codexConnected === false);
  const alternateProvider: ProviderKind | null =
    activeProvider === 'claude' && codexConnected === true ? 'codex'
      : activeProvider === 'codex' && claudeConnected === true ? 'claude'
        : null;
  const alternateModel = alternateProvider ? firstModelForProvider(alternateProvider) : null;

  const placeholder = providerNotConnected
    ? tab?.model === 'auto'
      ? 'Connect Claude or Codex to use automatic routing.'
      : `Switch provider/model below or connect ${providerLabel(activeProvider)}.`
    : hasPendingQuestion ? 'Answer the question above...' : wasInterrupted ? 'Continue, redirect, or ask something else...' : isWaiting ? 'Reply...' : isStreaming ? 'Guide this run… Enter to send, Ctrl+Enter to queue' : 'Type a message...';
  const canSend = !isOrchestrationOwned && (value.trim() || images.length > 0) && !!tab?.projectPath && !providerNotConnected;
  const selectedQueuedMessage = queuedMessages.find(item => item.id === editingQueuedId) ?? null;
  const pendingInterrupts = queuedMessages.filter(item => item.interruptActive);
  const trimmedQueuedDraft = queuedDraft.trim();
  const canSaveQueuedDraft = !!trimmedQueuedDraft && !savingQueuedMessage;

  if (isOrchestrationOwned) return null;

  function openQueuedMessage(id: string) {
    const item = queuedMessages.find(q => q.id === id);
    if (!item) return;
    setEditingQueuedId(item.id);
    setQueuedDraft(item.message);
    setQueuedEditError(null);
    setShowQueuePanel(true);
    setShowEnterChoice(false);
  }

  async function saveQueuedMessage() {
    if (!editingQueuedId || !canSaveQueuedDraft) return;
    setSavingQueuedMessage(true);
    setQueuedEditError(null);
    const error = await updateQueuedMessage(editingQueuedId, trimmedQueuedDraft);
    setSavingQueuedMessage(false);
    if (error) {
      setQueuedEditError(error);
      return;
    }
    setEditingQueuedId(null);
  }

  function deleteQueuedMessage(id: string) {
    removeQueuedMessage(id);
    if (editingQueuedId === id) setEditingQueuedId(null);
  }

  const iconBtn = (style?: React.CSSProperties): React.CSSProperties => ({
    display: 'flex', alignItems: 'center', justifyContent: 'center',
    padding: 5, borderRadius: 'var(--radius-sm)',
    background: 'transparent', border: 'none', cursor: 'pointer',
    color: 'var(--text-dim)',
    transition: 'color var(--transition-fast), background var(--transition-fast)',
    ...style,
  });

  return (
    <div className="input-bar-shell">
      <input
        ref={fileInputRef}
        type="file"
        accept="image/png,image/jpeg,image/gif,image/webp"
        multiple
        style={{ display: 'none' }}
        onChange={e => { if (e.target.files) addImages(e.target.files); e.target.value = ''; }}
      />

      {pendingInterrupts.length > 0 && (
        <div
          className="pending-interrupts"
          role="status"
          aria-live="polite"
          aria-label={pendingInterrupts.length === 1 ? 'Pending interrupt' : `${pendingInterrupts.length} pending interrupts`}
        >
          {pendingInterrupts.map(item => {
            const failed = item.admissionState === 'failed';
            const preview = item.message || `${item.imageCount} attached image${item.imageCount === 1 ? '' : 's'}`;
            return (
              <button
                key={item.id}
                type="button"
                className={`pending-interrupt${failed ? ' is-failed' : ''}`}
                onClick={() => openQueuedMessage(item.id)}
                aria-label={`${failed ? 'Edit unsent interrupt' : 'Open pending interrupt'}: ${preview}`}
              >
                <span className="pending-interrupt-indicator" aria-hidden="true" />
                <span className="pending-interrupt-copy">
                  <span className="pending-interrupt-label">
                    You · {failed ? 'Not sent' : 'Interrupting'}
                  </span>
                  <span className="pending-interrupt-message">{preview}</span>
                </span>
                <span className="pending-interrupt-action">{failed ? 'Edit' : 'View'}</span>
              </button>
            );
          })}
        </div>
      )}

      {providerNotConnected && (
        <div style={{
          display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 10,
          padding: '8px 10px', marginBottom: 6,
          borderRadius: 'var(--radius-md)',
          background: 'var(--surface-2)', border: '1px solid var(--border-soft)',
        }}>
          <div style={{ minWidth: 0 }}>
            <p style={{ fontSize: 12, color: 'var(--text)', fontWeight: 500, marginBottom: 2 }}>
              {providerLabel(activeProvider)} account not connected
            </p>
            <p style={{ fontSize: 11, color: 'var(--text-muted)' }}>
              {alternateProvider
                ? `${providerLabel(alternateProvider)} is connected. Switch models below or use the shortcut here.`
                : `Connect ${providerLabel(activeProvider)} or pick a different provider model below.`}
            </p>
          </div>
          <div style={{ display: 'flex', gap: 6, flexShrink: 0 }}>
            {alternateProvider && alternateModel && (
              <button
                onClick={() => updateTab(tabId, {
                  model: alternateModel,
                  ...(alternateProvider === 'codex' ? { reasoningEffort: normalizeReasoningEffort(alternateModel, tab.reasoningEffort) } : {}),
                  isSessionWarm: false,
                  requiresResume: !!tab.sessionId,
                  sessionWarmSince: undefined,
                  currentTool: undefined,
                })}
                style={{
                  padding: '5px 10px', borderRadius: 'var(--radius-sm)',
                  background: 'var(--accent-glow)', color: 'var(--accent)',
                  border: '1px solid var(--accent)', fontSize: 11, cursor: 'pointer',
                  fontFamily: 'var(--font-mono)',
                }}
              >
                Use {modelLabel(alternateModel)}
              </button>
            )}
            <button
              onClick={() => window.dispatchEvent(new CustomEvent('flamel:open-settings', { detail: { tab: activeProvider === 'codex' ? 'openai' : 'claude' } }))}
              style={{
                padding: '5px 10px', borderRadius: 'var(--radius-sm)',
                background: 'var(--surface-3)', color: 'var(--text)',
                border: '1px solid var(--border-soft)', fontSize: 11, cursor: 'pointer',
              }}
            >
              Connect
            </button>
          </div>
        </div>
      )}

      {/* Image thumbnails */}
      {images.length > 0 && (
        <div className="flex flex-wrap gap-2" style={{ marginBottom: 6 }}>
          {images.map((img, i) => (
            <div key={i} className="relative group" style={{ display: 'inline-block' }}>
              <img
                src={img.objectUrl}
                alt={img.name}
                style={{ height: 52, maxWidth: 90, objectFit: 'cover', borderRadius: 'var(--radius-md)', border: '1px solid var(--border-soft)', display: 'block' }}
              />
              <button
                onClick={() => removeImage(i)}
                style={{
                  position: 'absolute', top: -4, right: -4,
                  width: 15, height: 15, borderRadius: '50%',
                  background: 'var(--error)', color: '#fff',
                  fontSize: 9, border: 'none', cursor: 'pointer',
                  display: 'flex', alignItems: 'center', justifyContent: 'center',
                }}
              >
                <Icon.Close size={8} />
              </button>
            </div>
          ))}
        </div>
      )}

      {/* UltraPlan / UltraReview mode badge */}
      {planningMode && (
        <div style={{
          display: 'flex', alignItems: 'center', gap: 6,
          padding: '3px 8px 3px 6px',
          marginBottom: 4,
          borderRadius: 'var(--radius-sm)',
          background: planningMode === 'ultraplan' ? 'var(--accent-glow)' : 'var(--warning-glow)',
          border: `1px solid ${planningMode === 'ultraplan' ? 'var(--accent-border)' : 'rgba(240,196,92,0.3)'}`,
          width: 'fit-content',
        }}>
          <span style={{ fontSize: 11 }}>{planningMode === 'ultraplan' ? '📋' : '🔍'}</span>
          <span style={{
            fontSize: 11, fontWeight: 600,
            color: planningMode === 'ultraplan' ? 'var(--accent)' : 'var(--warning)',
            letterSpacing: '0.04em',
          }}>
            {planningMode === 'ultraplan' ? 'ultraplan mode' : 'ultrareview mode'}
          </span>
          <span style={{ fontSize: 10, color: 'var(--text-dim)' }}>
            — {planningMode === 'ultraplan' ? 'will plan before coding' : 'will review before changes'}
          </span>
        </div>
      )}

      {/* Input row: attach | textarea | send/stop */}
      <div style={{
        display: 'flex', alignItems: 'flex-end', gap: 6,
        borderBottom: `1px solid ${hasPendingQuestion ? 'var(--border-soft)' : isWaiting ? 'var(--accent)' : focused ? 'var(--border-focus)' : 'var(--border-soft)'}`,
        opacity: hasPendingQuestion ? 0.4 : 1,
        pointerEvents: hasPendingQuestion ? 'none' : 'auto',
        transition: 'border-color var(--transition-base)',
        paddingBottom: 6,
      }}>
        {/* Attach */}
        <button
          onClick={() => fileInputRef.current?.click()}
          title={isStreaming ? "Attach image (sent as a queued message)" : "Attach image"}
          style={{
            ...iconBtn({ cursor: 'pointer' }),
            marginBottom: 1, flexShrink: 0,
          }}
          onMouseEnter={e => { (e.currentTarget as HTMLElement).style.color = 'var(--text-muted)'; (e.currentTarget as HTMLElement).style.background = 'var(--surface-3)'; }}
          onMouseLeave={e => { (e.currentTarget as HTMLElement).style.color = 'var(--text-dim)'; (e.currentTarget as HTMLElement).style.background = 'transparent'; }}
        >
          <Icon.Attach size={14} />
        </button>

        {/* Textarea */}
        <textarea
          ref={textareaRef}
          value={value}
          onChange={e => { setValue(e.target.value); autoResize(); }}
          onKeyDown={handleKeyDown}
          onPaste={handlePaste}
          onFocus={() => setFocused(true)}
          onBlur={() => setFocused(false)}
          placeholder={placeholder}
          rows={1}
          className="w-full resize-none outline-none"
          style={{
            background: 'transparent',
            border: 'none',
            color: 'var(--text)',
            minHeight: 24,
            maxHeight: 160,
            padding: 0,
            fontFamily: 'var(--font-sans)',
            fontSize: 13,
            lineHeight: 1.55,
          }}
        />

        {/* Send / Stop */}
        {isStreaming ? (
          <div style={{ display: 'flex', gap: 4, flexShrink: 0 }}>
            {/* Guidance is non-destructive; Queue and Stop stay explicit. */}
            {canSend && (
              <>
              <button
                onClick={() => handleSend(true)}
                title="Queue for next turn (Ctrl+Enter)"
                style={{ border: '1px solid var(--border-soft)', borderRadius: 3, background: 'transparent', color: 'var(--text-muted)', padding: '2px 7px', fontSize: 12, cursor: 'pointer' }}
              >Queue</button>
              <button
                onClick={() => handleSend()}
                title="Send guidance (Enter)"
                aria-label="Send guidance"
                style={{
                  width: 26, height: 26, borderRadius: 'var(--radius-md)',
                  background: 'transparent',
                  border: '1px solid var(--border-soft)',
                  color: 'var(--text-dim)',
                  display: 'flex', alignItems: 'center', justifyContent: 'center',
                  cursor: 'pointer',
                  opacity: 0.7,
                  transition: 'opacity var(--transition-base)',
                }}
                onMouseEnter={e => (e.currentTarget.style.opacity = '1')}
                onMouseLeave={e => (e.currentTarget.style.opacity = '0.7')}
              >
                <Icon.Send size={11} />
              </button>
              <button onClick={() => setShowEnterChoice(!showEnterChoice)} title="More message options" aria-label="More message options" style={{ ...iconBtn(), width: 24 }}>…</button>
              </>
            )}
            <button
              onClick={abort}
              title="Stop (Esc)"
              style={{
                width: 26, height: 26, borderRadius: 'var(--radius-md)',
                background: 'var(--error-glow)',
                border: '1px solid var(--error)',
                color: 'var(--error)',
                display: 'flex', alignItems: 'center', justifyContent: 'center',
                cursor: 'pointer',
              }}
            >
              <Icon.Stop size={10} />
            </button>
          </div>
        ) : (
          <button
            onClick={() => handleSend()}
            disabled={!canSend}
            title="Send (Enter)"
            style={{
              width: 26, height: 26, borderRadius: 'var(--radius-md)', flexShrink: 0,
              background: canSend ? 'var(--accent)' : 'transparent',
              border: `1px solid ${canSend ? 'var(--accent)' : 'var(--border-soft)'}`,
              color: canSend ? '#fff' : 'var(--text-dim)',
              display: 'flex', alignItems: 'center', justifyContent: 'center',
              cursor: canSend ? 'pointer' : 'not-allowed',
              opacity: canSend ? 1 : 0.4,
              transition: 'all var(--transition-base)',
            }}
          >
            <Icon.Send size={11} />
          </button>
        )}
      </div>

      {showEnterChoice && (
        <div style={{
          display: 'flex', alignItems: 'center', justifyContent: 'flex-end', gap: 6,
          padding: '6px 0 0',
        }}>
          <button
            onClick={() => setShowEnterChoice(false)}
            title="Cancel"
            style={{ ...iconBtn(), width: 24, height: 24 }}
          >
            <Icon.Close size={10} />
          </button>
          <button
            onClick={() => handleSend(true)}
            title="Queue (Q)"
            style={{
              padding: '4px 9px', borderRadius: 'var(--radius-sm)',
              border: '1px solid var(--border-soft)', background: 'var(--surface-2)',
              color: 'var(--text-muted)', fontSize: 11, cursor: 'pointer',
            }}
          >
            Queue (Q)
          </button>
          <button
            onClick={handleInterruptSend}
            title="Stop the current run and send this message (I)"
            style={{
              padding: '4px 9px', borderRadius: 'var(--radius-sm)',
              border: '1px solid var(--error)', background: 'var(--error-glow)',
              color: 'var(--error)', fontSize: 11, cursor: 'pointer',
            }}
          >
            Stop and send (I)
          </button>
        </div>
      )}

      {showQueuePanel && queuedMessages.length > 0 && (
        <div style={{
          marginTop: 6,
          border: '1px solid var(--border-soft)',
          borderRadius: 'var(--radius-md)',
          background: 'var(--surface-2)',
          overflow: 'hidden',
        }}>
          <div style={{ maxHeight: 172, overflowY: 'auto' }}>
            {queuedMessages.map((item, index) => (
              <div
                key={item.id}
                role="button"
                tabIndex={0}
                onClick={() => openQueuedMessage(item.id)}
                onKeyDown={e => {
                  if (e.key === 'Enter' || e.key === ' ') {
                    e.preventDefault();
                    openQueuedMessage(item.id);
                  }
                }}
                style={{
                  display: 'flex', alignItems: 'center', gap: 8,
                  padding: '7px 8px', borderBottom: index === queuedMessages.length - 1 ? 'none' : '1px solid var(--border-soft)',
                  background: editingQueuedId === item.id ? 'var(--accent-glow)' : 'transparent',
                  cursor: 'pointer', minHeight: 34,
                }}
              >
                <span style={{
                  color: 'var(--text-dim)', fontSize: 10, fontFamily: 'var(--font-mono)',
                  width: 16, flexShrink: 0, textAlign: 'right',
                }}>
                  {index + 1}
                </span>
                <span style={{
                  color: 'var(--text)', fontSize: 12, lineHeight: 1.35,
                  whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis', flex: 1,
                }}>
                  {item.message || `${item.imageCount} image${item.imageCount === 1 ? '' : 's'}`}
                </span>
                {item.imageCount > 0 && (
                  <span style={{ color: 'var(--text-dim)', fontSize: 10, fontFamily: 'var(--font-mono)', flexShrink: 0 }}>
                    {item.imageCount} img
                  </span>
                )}
                {item.admissionState === 'failed' && (
                  <span
                    title="The server did not accept this message. It has been kept so you can edit or remove it."
                    style={{ color: 'var(--error)', fontSize: 10, flexShrink: 0 }}
                  >
                    Not sent
                  </span>
                )}
                <button
                  onClick={e => { e.stopPropagation(); deleteQueuedMessage(item.id); }}
                  disabled={item.serverOwned}
                  title={item.serverOwned ? "Server-backed messages can be edited but not removed" : "Remove"}
                  style={{ ...iconBtn({ width: 22, height: 22, padding: 0, flexShrink: 0 }) }}
                >
                  {item.serverOwned ? <Icon.Shield size={11} /> : <Icon.Trash size={11} />}
                </button>
              </div>
            ))}
          </div>

          {selectedQueuedMessage && (
            <div style={{
              borderTop: '1px solid var(--border-soft)',
              padding: 8,
              background: 'var(--surface)',
            }}>
              <textarea
                value={queuedDraft}
                onChange={e => setQueuedDraft(e.target.value)}
                onKeyDown={e => {
                  if (e.key === 'Escape') setEditingQueuedId(null);
                  if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) saveQueuedMessage();
                }}
                rows={3}
                disabled={savingQueuedMessage}
                style={{
                  width: '100%', resize: 'vertical', minHeight: 64, maxHeight: 160,
                  borderRadius: 'var(--radius-sm)', border: '1px solid var(--border-soft)',
                  background: 'var(--surface-2)', color: 'var(--text)', padding: 8,
                  fontSize: 12, lineHeight: 1.45, outline: 'none',
                }}
              />
              {queuedEditError && (
                <div role="alert" style={{ color: 'var(--error)', fontSize: 11, lineHeight: 1.4, marginTop: 6 }}>
                  {queuedEditError}
                </div>
              )}
              <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 8, marginTop: 7 }}>
                <button
                  onClick={() => deleteQueuedMessage(selectedQueuedMessage.id)}
                  disabled={selectedQueuedMessage.serverOwned}
                  title={selectedQueuedMessage.serverOwned ? 'Server-backed messages cannot be removed' : 'Remove'}
                  style={{
                    ...iconBtn({ gap: 4, padding: '4px 7px', color: 'var(--error)' }),
                    opacity: selectedQueuedMessage.serverOwned ? 0.45 : 1,
                    cursor: selectedQueuedMessage.serverOwned ? 'not-allowed' : 'pointer',
                  }}
                >
                  <Icon.Trash size={11} />
                  <span style={{ fontSize: 11 }}>Remove</span>
                </button>
                <div style={{ display: 'flex', gap: 6 }}>
                  <button
                    onClick={() => setEditingQueuedId(null)}
                    style={{
                      padding: '4px 9px', borderRadius: 'var(--radius-sm)',
                      border: '1px solid var(--border-soft)', background: 'transparent',
                      color: 'var(--text-dim)', fontSize: 11, cursor: 'pointer',
                    }}
                  >
                    Cancel
                  </button>
                  <button
                    onClick={saveQueuedMessage}
                    disabled={!canSaveQueuedDraft}
                    style={{
                      padding: '4px 9px', borderRadius: 'var(--radius-sm)',
                      border: `1px solid ${canSaveQueuedDraft ? 'var(--accent)' : 'var(--border-soft)'}`,
                      background: canSaveQueuedDraft ? 'var(--accent)' : 'transparent',
                      color: canSaveQueuedDraft ? '#fff' : 'var(--text-dim)',
                      fontSize: 11, cursor: canSaveQueuedDraft ? 'pointer' : 'not-allowed',
                      opacity: canSaveQueuedDraft ? 1 : 0.5,
                    }}
                  >
                    {savingQueuedMessage ? 'Saving…' : 'Save'}
                  </button>
                </div>
              </div>
            </div>
          )}
        </div>
      )}

      {/* Bottom toolbar */}
      <div className="composer-toolbar">
        {/* Git status */}
        {gitStatus?.isRepo && (
          <button
            className="composer-status-chip"
            onClick={onViewChanges}
            title="View changes"
            style={{
              ...iconBtn(),
              gap: 4, padding: '2px 6px', minWidth: 0,
              fontSize: 10, fontFamily: 'var(--font-mono)',
              color: 'var(--text-dim)',
            }}
            onMouseEnter={e => { (e.currentTarget as HTMLElement).style.color = 'var(--text-muted)'; (e.currentTarget as HTMLElement).style.background = 'var(--surface-3)'; }}
            onMouseLeave={e => { (e.currentTarget as HTMLElement).style.color = 'var(--text-dim)'; (e.currentTarget as HTMLElement).style.background = 'transparent'; }}
          >
            <Icon.Branch size={10} style={{ flexShrink: 0 }} />
            <span className="composer-chip-label">{gitStatus.branch}</span>
            {(gitStatus.added > 0 || gitStatus.removed > 0) && (
              <>
                <span style={{ color: 'var(--success)' }}>+{gitStatus.added}</span>
                <span style={{ color: 'var(--error)' }}>-{gitStatus.removed}</span>
              </>
            )}
          </button>
        )}

        <div className="composer-toolbar-spacer" />

        {/* Queue count badge */}
        {queuedCount > 0 && (
          <button
            onClick={() => { setShowQueuePanel(v => !v); setShowEnterChoice(false); }}
            title="View queued messages"
            style={{
              display: 'flex', alignItems: 'center', gap: 4,
              fontSize: 10, fontFamily: 'var(--font-mono)',
              color: showQueuePanel ? 'var(--accent)' : 'var(--text-dim)',
              padding: '2px 5px',
              background: showQueuePanel ? 'var(--accent-glow)' : 'var(--surface-2)',
              borderRadius: 'var(--radius-sm)',
              border: '1px solid var(--border-soft)',
              cursor: 'pointer',
            }}
          >
            <Icon.Plan size={10} />
            {queuedCount} queued
          </button>
        )}

        {/* Model picker */}
        <div style={{ position: 'relative' }}>
          <button
            className={`composer-control-button composer-model-chip${showModelPicker ? ' is-open' : ''}`}
            onClick={() => {
              setShowModelPicker(s => !s);
              setShowReasoningPicker(false);
              setShowRunnerPicker(false);
            }}
            title="Select model"
          >
            <span className="composer-chip-label">{modelLabel(tab.model)}</span>
            <Icon.ChevronDown size={8} style={{ opacity: 0.55, flexShrink: 0 }} />
          </button>

          {showModelPicker && (
            <div className="composer-menu" style={{
              position: 'absolute', bottom: 'calc(100% + 4px)', right: 0,
              background: 'var(--surface)', border: '1px solid var(--border)',
              borderRadius: 'var(--radius-lg)', minWidth: 160,
              boxShadow: 'var(--shadow-lg)', zIndex: 50, padding: '4px 0',
            }}>
              {MODEL_GROUPS.map((group, gi) => (
                <div key={group.provider}>
                  {gi > 0 && <div style={{ height: 1, background: 'var(--border-soft)', margin: '4px 0' }} />}
                  <div style={{
                    padding: '4px 12px 3px',
                    fontSize: 9, fontWeight: 600, letterSpacing: '0.08em',
                    textTransform: 'uppercase', color: 'var(--text-dim)',
                    fontFamily: 'var(--font-mono)',
                  }}>
                    {group.label}
                  </div>
                  {group.models.map(def => {
                    const active = tab.model === def.id;
                    return (
                      <button
                        key={def.id}
                        onClick={() => {
                          updateTab(tabId, {
                            model: def.id,
                            ...(def.provider === 'codex' ? {
                              reasoningEffort: tab.reasoningEffort === 'auto'
                                ? 'auto'
                                : normalizeReasoningEffort(def.id, tab.reasoningEffort),
                            } : {}),
                            isSessionWarm: false,
                            requiresResume: !!tab.sessionId,
                            sessionWarmSince: undefined,
                            currentTool: undefined,
                          });
                          setShowModelPicker(false);
                        }}
                        style={{
                          width: '100%', textAlign: 'left',
                          padding: '5px 12px', fontSize: 11,
                          color: active ? 'var(--accent)' : 'var(--text)',
                          background: active ? 'var(--accent-glow)' : 'transparent',
                          fontFamily: 'var(--font-mono)',
                          display: 'flex', alignItems: 'center', justifyContent: 'space-between',
                          cursor: 'pointer', gap: 8, border: 'none',
                          transition: 'background var(--transition-fast)',
                        }}
                        onMouseEnter={e => { if (!active) (e.currentTarget as HTMLElement).style.background = 'var(--surface-2)'; }}
                        onMouseLeave={e => { if (!active) (e.currentTarget as HTMLElement).style.background = 'transparent'; }}
                      >
                        <span>{def.label}</span>
                        {active && <Icon.Check size={11} style={{ color: 'var(--accent)' }} />}
                      </button>
                    );
                  })}
                </div>
              ))}
            </div>
          )}
        </div>

        {tab.model !== 'auto' && activeProvider === 'codex' && (
          <div style={{ position: 'relative' }}>
            <button
              className={`composer-control-button composer-reasoning-chip${showReasoningPicker ? ' is-open' : ''}`}
              onClick={() => {
                setShowReasoningPicker(value => !value);
                setShowModelPicker(false);
                setShowRunnerPicker(false);
              }}
              title={tab.reasoningEffort === 'auto'
                ? `Automatic reasoning${tab.resolvedReasoningEffort ? ` · current run: ${tab.resolvedReasoningEffort}` : ''}${tab.routingReason ? ` · ${tab.routingReason}` : ''}`
                : `Manual reasoning: ${tab.reasoningEffort}${tab.resolvedReasoningEffort ? ` · current run: ${tab.resolvedReasoningEffort}` : ''}`}
            >
              <Icon.Prompts size={10} style={{ flexShrink: 0 }} />
              <span className="composer-chip-label">
                {reasoningChipLabel(tab.model, tab.reasoningEffort, tab.resolvedReasoningEffort)}
              </span>
              <Icon.ChevronDown size={8} style={{ opacity: 0.55, flexShrink: 0 }} />
            </button>

            {showReasoningPicker && (
              <div className="composer-menu composer-reasoning-menu">
                <div className="composer-menu-heading">Reasoning</div>
                {reasoningSelectionsForModel(tab.model).map(effort => {
                  const active = tab.reasoningEffort === effort.id;
                  return (
                    <button
                      key={effort.id}
                      className={`composer-menu-option${active ? ' is-active' : ''}`}
                      onClick={() => {
                        updateTab(tabId, {
                          reasoningEffort: effort.id,
                          isSessionWarm: false,
                          requiresResume: !!tab.sessionId,
                          sessionWarmSince: undefined,
                          currentTool: undefined,
                        });
                        setShowReasoningPicker(false);
                      }}
                    >
                      <span>
                        <strong>{effort.label}</strong>
                        <small>{effort.title}</small>
                      </span>
                      {active && <Icon.Check size={11} />}
                    </button>
                  );
                })}
              </div>
            )}
          </div>
        )}

        <button
          type="button"
          className={`composer-control-button composer-ponytail-toggle${tab.ponytailEnabled ? ' is-active' : ''}`}
          aria-pressed={tab.ponytailEnabled}
          onClick={() => updateTab(tabId, {
            ponytailEnabled: !tab.ponytailEnabled,
            isSessionWarm: false,
            requiresResume: !!tab.sessionId,
            sessionWarmSince: undefined,
            currentTool: undefined,
          })}
          title={tab.ponytailEnabled ? 'Turn Ponytail off' : 'Turn Ponytail on'}
        >
          <Icon.Ponytail size={11} />
          <span>pony</span>
        </button>

        {/* Permission mode */}
        <div className="composer-control-group" title="Tool approvals">
          <Icon.Shield className="composer-control-icon" size={10} />
          {(['bypassPermissions', 'default'] as PermissionMode[]).map((mode) => {
            const active = tab.permissionMode === mode;
            const label = mode === 'bypassPermissions' ? 'allow' : 'ask';
            return (
              <button
                key={mode}
                className={`composer-segment-button composer-permission-button${active ? ' is-active' : ''}`}
                data-mode={mode}
                onClick={() => updateTab(tabId, { permissionMode: mode })}
                title={mode === 'bypassPermissions' ? 'Auto-approve all tools' : 'Ask before each tool'}
              >{label}</button>
            );
          })}
        </div>

        {/* Runner picker */}
        {runners.length > 0 && (
          <div style={{ position: 'relative' }}>
            <button
              className={`composer-control-button composer-runner-chip${showRunnerPicker ? ' is-open' : ''}${tab.runnerId ? ' has-selection' : ''}`}
              onClick={() => {
                setShowRunnerPicker(s => !s);
                setShowModelPicker(false);
                setShowReasoningPicker(false);
              }}
              title="Select execution host"
            >
              <Icon.Server size={9} style={{ flexShrink: 0 }} />
              <span className="composer-chip-label">{tab.runnerId === 'auto' ? 'auto' : tab.runnerName || 'local'}</span>
              <Icon.ChevronDown size={8} style={{ opacity: 0.55, flexShrink: 0 }} />
            </button>

            {showRunnerPicker && (
              <div className="composer-menu" style={{
                position: 'absolute', bottom: 'calc(100% + 4px)', right: 0,
                background: 'var(--surface)', border: '1px solid var(--border)',
                borderRadius: 'var(--radius-lg)', minWidth: 160,
                boxShadow: 'var(--shadow-lg)', zIndex: 50, padding: '4px 0',
              }}>
                {/* Local option */}
                <button
                  onClick={() => { updateTab(tabId, { runnerId: undefined, runnerName: undefined }); setShowRunnerPicker(false); }}
                  style={{
                    width: '100%', textAlign: 'left',
                    padding: '6px 12px', fontSize: 11,
                    color: !tab.runnerId ? 'var(--accent)' : 'var(--text)',
                    background: !tab.runnerId ? 'var(--accent-glow)' : 'transparent',
                    fontFamily: 'var(--font-mono)',
                    display: 'flex', alignItems: 'center', justifyContent: 'space-between',
                    cursor: 'pointer', gap: 8, border: 'none',
                    transition: 'background var(--transition-fast)',
                  }}
                  onMouseEnter={e => { if (tab.runnerId) (e.currentTarget as HTMLElement).style.background = 'var(--surface-2)'; }}
                  onMouseLeave={e => { if (tab.runnerId) (e.currentTarget as HTMLElement).style.background = 'transparent'; }}
                >
                  <span>local</span>
                  {!tab.runnerId && <Icon.Check size={11} style={{ color: 'var(--accent)' }} />}
                </button>

                {/* Auto option */}
                <button
                  onClick={() => { updateTab(tabId, { runnerId: 'auto', runnerName: 'auto' }); setShowRunnerPicker(false); }}
                  style={{
                    width: '100%', textAlign: 'left',
                    padding: '6px 12px', fontSize: 11,
                    color: tab.runnerId === 'auto' ? 'var(--accent)' : 'var(--text)',
                    background: tab.runnerId === 'auto' ? 'var(--accent-glow)' : 'transparent',
                    fontFamily: 'var(--font-mono)',
                    display: 'flex', alignItems: 'center', justifyContent: 'space-between',
                    cursor: 'pointer', gap: 8, border: 'none',
                    transition: 'background var(--transition-fast)',
                  }}
                  onMouseEnter={e => { if (tab.runnerId !== 'auto') (e.currentTarget as HTMLElement).style.background = 'var(--surface-2)'; }}
                  onMouseLeave={e => { if (tab.runnerId !== 'auto') (e.currentTarget as HTMLElement).style.background = 'transparent'; }}
                >
                  <span>auto</span>
                  {tab.runnerId === 'auto' && <Icon.Check size={11} style={{ color: 'var(--accent)' }} />}
                </button>

                <div style={{ height: 1, background: 'var(--border-soft)', margin: '2px 0' }} />

                {/* Runner options */}
                {runners.map(r => {
                  const active = tab.runnerId === r.id;
                  return (
                    <button
                      key={r.id}
                      onClick={() => { updateTab(tabId, { runnerId: r.id, runnerName: r.name }); setShowRunnerPicker(false); }}
                      style={{
                        width: '100%', textAlign: 'left',
                        padding: '6px 12px', fontSize: 11,
                        color: active ? 'var(--accent)' : 'var(--text)',
                        background: active ? 'var(--accent-glow)' : 'transparent',
                        fontFamily: 'var(--font-mono)',
                        display: 'flex', alignItems: 'center', justifyContent: 'space-between',
                        cursor: 'pointer', gap: 8, border: 'none',
                        transition: 'background var(--transition-fast)',
                        opacity: r.online ? 1 : 0.5,
                      }}
                      onMouseEnter={e => { if (!active) (e.currentTarget as HTMLElement).style.background = 'var(--surface-2)'; }}
                      onMouseLeave={e => { if (!active) (e.currentTarget as HTMLElement).style.background = 'transparent'; }}
                    >
                      <span style={{ display: 'flex', alignItems: 'center', gap: 5 }}>
                        <span style={{
                          width: 5, height: 5, borderRadius: '50%',
                          background: r.online ? 'var(--success, #22c55e)' : 'var(--text-dim)',
                          flexShrink: 0,
                        }} />
                        {r.name}
                      </span>
                      {active && <Icon.Check size={11} style={{ color: 'var(--accent)' }} />}
                    </button>
                  );
                })}
              </div>
            )}
          </div>
        )}

        {/* Background — visible while streaming */}
        {isStreaming && (
          <button
            className="composer-icon-button composer-background-button"
            onClick={() => {
              const newId = addTab();
              setActiveTab(newId);
            }}
            title="Background this agent and open a new session"
            aria-label="Background this agent and open a new session"
          >
            <Icon.Plus size={10} />
          </button>
        )}
      </div>
    </div>
  );
}
