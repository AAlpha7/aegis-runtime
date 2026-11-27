/**
 * Global CEO voice engine — realtime duplex only.
 *
 * Product hierarchy (design B — one chat panel):
 *   - Top-bar Aegis avatar (OwnerShellChrome) = duplex voice toggle.
 *   - Text chat lives only in ChatHomeView (full Aegis tab or bottom sheet).
 *   - This component keeps confirm chips + live captions for the voice session.
 *   - Never silently demote a failed Realtime start into browser dictation.
 *
 * Realtime audio path: see voiceRealtime.ts.
 * High-risk tools still require confirm chips (float near chrome).
 */
import {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useRef,
  useState,
  type MutableRefObject,
} from 'react';
import type { Locale } from '../i18n/catalog';
import { t } from '../i18n/catalog';
import type { VNextVoiceSession } from '../api';
import { AegisAvatar } from './Avatar';
import { VNextIcon } from './icons';
import {
  dispatchCeoCommand,
  realtimeCeoTools,
  CEO_PERSONA_INSTRUCTIONS,
  type CeoHandlers,
} from './ceoCommands';
import { liveSpeakingLine } from './ceoVoicePanel';
import { voicePendingDecision } from './ceoIntent';
import type { VoiceHealth } from './voiceDiag';
import {
  startLiveRealtimeSession,
  type LiveRealtimeSession,
} from './voiceRealtime';
import type { PanelStreamItem } from './ceoVoicePanel';

export type { PanelStreamItem } from './ceoVoicePanel';

export type CeoVoicePhase = 'idle' | 'connecting' | 'live';

/** Snapshot for OwnerShellChrome summon control (avatar = voice entry). */
export type CeoVoiceChromeState = {
  phase: CeoVoicePhase;
  live: boolean;
  agentSpeaking: boolean;
  userSpeaking: boolean;
  fault: boolean;
  /** Empty when idle and healthy — chrome must not show idle noise. */
  statusLine: string;
  voiceLabel: string;
  voiceHint: string;
};

export type CeoVoiceAgentHandle = {
  /** Toggle full-duplex Realtime (same as tapping the chrome avatar). */
  toggleVoice: () => void;
  /** Surface a runtime HITL gate (SSE need_approval) during duplex. */
  notifyApproval: (card: { approvalId: string; toolName: string; summary: string }) => void;
  /** Mark a gate card settled after verbal approve/reject. */
  settleApproval: (approvalId: string, state: 'approved' | 'rejected') => void;
  /** Ask the live call to speak the next beat. False when the socket is closed. */
  continueTurn: (text: string) => boolean;
};

export type CeoVoiceAgentProps = {
  locale: Locale;
  busy?: boolean;
  enabled?: boolean;
  realtimeAvailable?: boolean;
  mintSession: () => Promise<VNextVoiceSession>;
  buildScreenContext: () => string;
  handlersRef: MutableRefObject<CeoHandlers>;
  /** Runtime HITL approve/reject (SSE need_approval cards). */
  onApprovalAction?: (approvalId: string, action: 'approve' | 'reject') => void | Promise<void>;
  onNotice?: (notice: string) => void;
  /** Publish phase/status for the shell avatar summon control. */
  onChromeState?: (state: CeoVoiceChromeState) => void;
  /** The single line shown while 安琪 is speaking. Empty when she is not. */
  onLiveLine?: (text: string) => void;
  /** A finished utterance, ready for the left-hand transcript. */
  onUtteranceSettled?: (role: 'owner' | 'aegis', text: string) => void;
  /** Spoken 批准 / 先不要 while a search gate is waiting. */
  onVoiceDecision?: (action: 'approve' | 'reject', approvalId: string) => void;
  /**
   * When true (default), hide the bottom-right floating orb — summon lives in
   * OwnerShellChrome. Legacy floating dock only if chromeMode={false}.
   */
  chromeMode?: boolean;
  aegisAvatarUrl?: string;
  className?: string;
};

type Phase = CeoVoicePhase;
type StreamItem = PanelStreamItem;

export const CeoVoiceAgent = forwardRef<CeoVoiceAgentHandle, CeoVoiceAgentProps>(function CeoVoiceAgent({
  locale,
  // Soft hints from parent (status probe). Mint remains the source of truth —
  // do not gate connect on these (Maka/Hercules pattern).
  busy: _chatBusy = false,
  enabled = true,
  realtimeAvailable: _realtimeHint = false,
  mintSession,
  buildScreenContext,
  handlersRef,
  onApprovalAction: _onApprovalAction,
  onNotice,
  onChromeState,
  onLiveLine,
  onUtteranceSettled,
  onVoiceDecision,
  chromeMode = true,
  aegisAvatarUrl,
  className,
}, ref) {
  void _chatBusy;
  void _realtimeHint;
  void _onApprovalAction;
  const [phase, setPhase] = useState<Phase>('idle');
  const [stream, setStream] = useState<StreamItem[]>([]);
  const [elapsed, setElapsed] = useState(0);
  const [working, setWorking] = useState(false);
  const [agentSpeaking, setAgentSpeaking] = useState(false);
  const [userSpeaking, setUserSpeaking] = useState(false);
  const [health, setHealth] = useState<VoiceHealth | null>(null);
  /** Sticky duplex fault shown on the orb / chrome status. */
  const [duplexFault, setDuplexFault] = useState<string | null>(null);

  const buildContextRef = useRef(buildScreenContext);
  useEffect(() => {
    buildContextRef.current = buildScreenContext;
  });
  const onNoticeRef = useRef(onNotice);
  const onLiveLineRef = useRef(onLiveLine);
  const onUtteranceRef = useRef(onUtteranceSettled);
  const onVoiceDecisionRef = useRef(onVoiceDecision);
  const pendingApprovalRef = useRef<Array<{ approvalId: string; toolName: string; summary: string }>>([]);
  const settledUtteranceIds = useRef(new Set<string>());
  useEffect(() => {
    onNoticeRef.current = onNotice;
    onLiveLineRef.current = onLiveLine;
    onUtteranceRef.current = onUtteranceSettled;
    onVoiceDecisionRef.current = onVoiceDecision;
  });

  const liveRef = useRef<LiveRealtimeSession | null>(null);
  const elapsedTimerRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const lastFaultCodeRef = useRef<string | null>(null);
  const idSeq = useRef(0);
  const nextId = () => {
    idSeq.current += 1;
    return `v-${idSeq.current}`;
  };

  const pushStream = useCallback((item: StreamItem) => {
    setStream((current) => current.concat(item).slice(-40));
  }, []);

  const clearElapsedTimer = () => {
    if (elapsedTimerRef.current) {
      clearInterval(elapsedTimerRef.current);
      elapsedTimerRef.current = null;
    }
  };

  const stopAll = useCallback((notice?: string) => {
    liveRef.current?.stop();
    liveRef.current = null;
    clearElapsedTimer();
    setPhase('idle');
    setElapsed(0);
    setWorking(false);
    setAgentSpeaking(false);
    setUserSpeaking(false);
    setHealth(null);
    lastFaultCodeRef.current = null;
    if (notice) onNoticeRef.current?.(notice);
  }, []);

  const toggleVoiceRef = useRef<() => void>(() => {});

  useImperativeHandle(ref, () => ({
    toggleVoice: () => {
      toggleVoiceRef.current();
    },
    notifyApproval: (card) => {
      if (!pendingApprovalRef.current.some((row) => row.approvalId === card.approvalId)) {
        pendingApprovalRef.current = pendingApprovalRef.current.concat(card).slice(-6);
      }
      setStream((current) => {
        if (current.some((row) => row.type === 'approval' && row.approvalId === card.approvalId)) {
          return current;
        }
        return current.concat({
          id: nextId(),
          type: 'approval',
          approvalId: card.approvalId,
          toolName: card.toolName,
          summary: card.summary,
        }).slice(-40);
      });
    },
    settleApproval: (approvalId, state) => {
      pendingApprovalRef.current = pendingApprovalRef.current.filter((row) => row.approvalId !== approvalId);
      setStream((current) =>
        current.map((row) =>
          row.type === 'approval' && row.approvalId === approvalId
            ? { ...row, settled: state }
            : row,
        ),
      );
    },
    continueTurn: (text) => liveRef.current?.continueTurn(text) ?? false,
  }), []);

  const handleFunctionCall = useCallback(
    async (callId: string, name: string, argsJson: string) => {
      setWorking(true);
      const result = await dispatchCeoCommand(handlersRef.current, name, argsJson, { locale });
      if (result.needsConfirmation) {
        pushStream({
          id: nextId(),
          type: 'confirm',
          name,
          argsJson,
          detail: result.message,
        });
      } else {
        pushStream({
          id: nextId(),
          type: 'action',
          name,
          ok: result.ok,
          detail: result.message,
        });
      }
      liveRef.current?.sendFunctionOutput(callId, result);
      setWorking(false);
    },
    [handlersRef, locale, pushStream],
  );

  const startRealtime = useCallback(async () => {
    // Orb = duplex only. Mint is the source of truth (not cached status).
    // On failure: stay on the orb with a sticky fault — never open a text
    // panel and never silent-switch to browser dictation.
    stopAll();
    setDuplexFault(null);
    setPhase('connecting');
    onNoticeRef.current?.(t('voice.orb.connecting', locale));
    try {
      const session = await startLiveRealtimeSession(
        async () => {
          const minted = await mintSession();
          return {
            client_secret: minted.client_secret,
            model: minted.model,
            ws_url: minted.ws_url,
            max_session_seconds: minted.max_session_seconds,
          };
        },
        {
          buildInstructions: () =>
            (locale === 'en'
              ? 'You are Aegis, the owner\'s chief of staff. You are in a LIVE full-duplex voice call. '
                + 'Always speak a short audible reply after the owner talks. Never stay silent. '
                + 'Prefer spoken answers; use tools only when acting. Do not ask the owner to open chat.\n\n'
              : '你是 Aegis，私人秘书。你在全双工语音通话中。'
                + '对方说完后必须用简短口语回答，禁止只沉默或只出字不出声。'
                + '优先说话；需要办事时再用工具。不要让对方打开文字聊天窗。\n\n')
            + `${CEO_PERSONA_INSTRUCTIONS}\n\n`
            + `SCREEN CONTEXT:\n${buildContextRef.current() || '(empty)'}`,
          tools: realtimeCeoTools(),
          languageHint: locale === 'en' ? 'en' : 'zh',
          onPhase: (p) => {
            if (p === 'live') {
              setPhase('live');
              setDuplexFault(null);
              onNoticeRef.current?.(t('voice.orb.live', locale));
              clearElapsedTimer();
              elapsedTimerRef.current = setInterval(() => {
                setElapsed(liveRef.current?.getElapsedSeconds() ?? 0);
              }, 1000);
            } else if (p === 'idle') {
              setPhase('idle');
              clearElapsedTimer();
            } else {
              setPhase('connecting');
            }
          },
          onSpeaking: setAgentSpeaking,
          onListeningUser: setUserSpeaking,
          onDiag: (_diag, nextHealth) => {
            setHealth(nextHealth);
            if (!nextHealth.ok && nextHealth.code !== 'connecting') {
              if (lastFaultCodeRef.current !== nextHealth.code) {
                lastFaultCodeRef.current = nextHealth.code;
                const msg = locale === 'en' ? nextHealth.messageEn : nextHealth.messageZh;
                onNoticeRef.current?.(msg);
                setDuplexFault(msg);
              }
            } else if (nextHealth.ok) {
              lastFaultCodeRef.current = null;
              setDuplexFault(null);
            }
          },
          onTranscriptDelta: (delta) => {
            setStream((current) => {
              const last = current[current.length - 1];
              if (last?.type === 'assistant') {
                return current.slice(0, -1).concat({ ...last, text: last.text + delta });
              }
              return current.concat({ id: nextId(), type: 'assistant', text: delta });
            });
          },
          onUserTranscript: (text, final) => {
            const trimmed = text.trim();
            if (!trimmed) return;
            let settledId = '';
            setStream((current) => {
              const last = current[current.length - 1];
              if (last?.type === 'user') {
                if (final) settledId = last.id;
                return current.slice(0, -1).concat({ ...last, text: trimmed });
              }
              const id = nextId();
              if (final) settledId = id;
              return current.concat({ id, type: 'user', text: trimmed });
            });
            if (final && settledId && !settledUtteranceIds.current.has(settledId)) {
              settledUtteranceIds.current.add(settledId);
              const decision = voicePendingDecision(trimmed);
              const gate = pendingApprovalRef.current[pendingApprovalRef.current.length - 1];
              if (decision && gate) {
                pendingApprovalRef.current = pendingApprovalRef.current.filter(
                  (row) => row.approvalId !== gate.approvalId,
                );
                void onVoiceDecisionRef.current?.(decision, gate.approvalId);
              }
              onUtteranceRef.current?.('owner', trimmed);
            }
          },
          onError: (message) => {
            setDuplexFault(message);
            onNoticeRef.current?.(message);
          },
          onFunctionCall: (callId, name, argsJson) => {
            void handleFunctionCall(callId, name, argsJson);
          },
        },
      );
      liveRef.current = session;
    } catch (err) {
      const raw = err instanceof Error ? err.message : String(err ?? '');
      const errName = err instanceof Error ? err.name : '';
      const insecure = /insecure|SecurityError/i.test(raw);
      const micDenied = errName === 'NotAllowedError' || /not allowed|denied permission|Permission denied/i.test(raw);
      // Owner-facing detail only: never surface the browser's raw English
      // exception text (it showed up verbatim under the composer, 2026-09-08).
      const detail = insecure
        ? t('voice.orb.failCsp', locale)
        : micDenied
          ? t('voice.orb.failMicDenied', locale)
          : '';
      const banner = detail
        ? `${t('voice.orb.realtimeFailed', locale)} ${detail}`
        : t('voice.orb.realtimeFailed', locale);
      stopAll();
      // Header line stays short; the banner carries the detail once.
      setDuplexFault(t('voice.orb.realtimeFailedShort', locale));
      onNoticeRef.current?.(banner);
    }
  }, [handleFunctionCall, locale, mintSession, stopAll]);

  const toggleVoice = useCallback(() => {
    const liveNow = phase === 'live' || phase === 'connecting';
    if (!liveNow) {
      void startRealtime();
      return;
    }
    stopAll();
    setDuplexFault(null);
  }, [phase, startRealtime, stopAll]);
  toggleVoiceRef.current = toggleVoice;

  useEffect(() => () => stopAll(), [stopAll]);

  useEffect(() => {
    if (!enabled) stopAll();
  }, [enabled, stopAll]);

  const confirmPending = useCallback(
    async (item: Extract<StreamItem, { type: 'confirm' }>, yes: boolean) => {
      setStream((current) =>
        current.map((row) =>
          row.id === item.id && row.type === 'confirm'
            ? { ...row, settled: yes ? 'yes' : 'no' }
            : row,
        ),
      );
      if (!yes) return;
      let args: Record<string, unknown> = {};
      try {
        args = JSON.parse(item.argsJson) as Record<string, unknown>;
      } catch {
        args = {};
      }
      args.confirmed = true;
      const result = await dispatchCeoCommand(
        handlersRef.current,
        item.name,
        JSON.stringify(args),
        { locale },
      );
      pushStream({
        id: nextId(),
        type: 'action',
        name: item.name,
        ok: result.ok,
        detail: result.message,
      });
    },
    [handlersRef, locale, pushStream],
  );

  const live = phase === 'live' || phase === 'connecting';
  const orbLabel = live ? t('voice.orb.stop', locale) : t('voice.orb.start', locale);
  let statusLine = '';
  if (duplexFault && phase === 'idle') {
    statusLine = duplexFault;
  } else if (health && !health.ok && health.code !== 'connecting' && phase === 'live') {
    statusLine = locale === 'en' ? health.messageEn : health.messageZh;
  } else if (phase === 'connecting') {
    statusLine = t('voice.orb.connecting', locale);
  } else if (phase === 'live') {
    if (agentSpeaking) statusLine = t('voice.orb.speaking', locale);
    else if (userSpeaking) statusLine = t('voice.orb.hearingYou', locale);
    else statusLine = `${t('voice.orb.live', locale)}${elapsed > 0 ? ` · ${elapsed}s` : ''}`;
  }
  const faultActive = Boolean(
    duplexFault || (health && !health.ok && health.code !== 'connecting'),
  );

  useEffect(() => {
    if (!enabled) return;
    onChromeState?.({
      phase,
      live,
      agentSpeaking,
      userSpeaking,
      fault: faultActive,
      statusLine,
      voiceLabel: orbLabel,
      voiceHint: t('voice.orb.hint', locale),
    });
  }, [
    agentSpeaking,
    enabled,
    faultActive,
    live,
    locale,
    onChromeState,
    orbLabel,
    phase,
    statusLine,
    userSpeaking,
  ]);

  const assistantDraft = (() => {
    for (let i = stream.length - 1; i >= 0; i -= 1) {
      const item = stream[i];
      if (item?.type === 'assistant' && item.text.trim()) return item;
    }
    return null;
  })();
  const liveLine = liveSpeakingLine(agentSpeaking, assistantDraft?.text ?? '');

  useEffect(() => {
    onLiveLineRef.current?.(enabled ? liveLine : '');
  }, [enabled, liveLine]);

  useEffect(() => {
    if (!enabled || agentSpeaking || !assistantDraft?.text.trim()) return undefined;
    const item = assistantDraft;
    const timer = window.setTimeout(() => {
      if (settledUtteranceIds.current.has(item.id)) return;
      settledUtteranceIds.current.add(item.id);
      onUtteranceRef.current?.('aegis', item.text.trim());
    }, 400);
    return () => window.clearTimeout(timer);
  }, [agentSpeaking, assistantDraft, enabled]);

  useEffect(() => {
    const onCaption = (event: Event) => {
      const detail = (event as CustomEvent<{ speaking?: boolean; text?: string; role?: 'owner' | 'assistant' }>).detail;
      if (!detail || !enabled) return;
      const text = (detail.text || '').replace(/\s+/g, ' ').trim();
      const speaking = Boolean(detail.speaking);
      if (detail.role === 'owner') {
        if (!text) return;
        const id = nextId();
        setStream((current) => current.concat({ id, type: 'user', text }));
        if (!settledUtteranceIds.current.has(id)) {
          settledUtteranceIds.current.add(id);
          const decision = voicePendingDecision(text);
          const gate = pendingApprovalRef.current[pendingApprovalRef.current.length - 1];
          if (decision && gate) {
            pendingApprovalRef.current = pendingApprovalRef.current.filter(
              (row) => row.approvalId !== gate.approvalId,
            );
            void onVoiceDecisionRef.current?.(decision, gate.approvalId);
          }
          onUtteranceRef.current?.('owner', text);
        }
        return;
      }
      setPhase('live');
      setAgentSpeaking(speaking);
      setStream((current) => {
        const last = current[current.length - 1];
        if (last?.type === 'assistant') return current.slice(0, -1).concat({ ...last, text });
        return current.concat({ id: nextId(), type: 'assistant', text });
      });
    };
    window.addEventListener('aegis-voice-caption', onCaption);
    return () => window.removeEventListener('aegis-voice-caption', onCaption);
  }, [enabled]);

  if (!enabled) return null;

  const pendingConfirms = stream.filter(
    (item): item is Extract<StreamItem, { type: 'confirm' }> =>
      item.type === 'confirm' && !item.settled,
  );

  return (
    <div
      className={[
        'vnext-ceo-voice-agent',
        chromeMode && 'is-chrome',
        live && 'is-listening',
        agentSpeaking && 'is-agent-speaking',
        userSpeaking && 'is-user-speaking',
        faultActive && 'is-fault',
        working && 'is-working',
        className,
      ]
        .filter(Boolean)
        .join(' ')}
      data-testid="ceo-voice-agent"
      data-voice-phase={phase}
      data-voice-chrome={chromeMode ? 'on' : 'off'}
    >
      {pendingConfirms.length > 0 && (
        <div className="vnext-ceo-voice-confirm-float" role="region" aria-label={t('voice.panel.needsConfirm', locale)}>
          {pendingConfirms.map((item) => (
            <div key={item.id} className="vnext-ceo-voice-confirm vnext-ceo-review-card">
              <p className="vnext-kicker">{t('voice.panel.needsConfirm', locale)}</p>
              <p>{item.detail}</p>
              <div className="vnext-ceo-voice-confirm-actions">
                <button type="button" className="vnext-btn vnext-btn-primary" onClick={() => void confirmPending(item, true)}>
                  {t('voice.panel.approve', locale)}
                </button>
                <button type="button" className="vnext-btn vnext-btn-secondary" onClick={() => void confirmPending(item, false)}>
                  {t('voice.panel.reject', locale)}
                </button>
              </div>
            </div>
          ))}
        </div>
      )}

      {/* Legacy floating dock — only when chromeMode is off. */}
      {!chromeMode && (
        <div className="vnext-ceo-voice-dock">
          {statusLine && (
            <span className="vnext-ceo-voice-status" role="status">{statusLine}</span>
          )}
          <button
            type="button"
            className="vnext-ceo-voice-orb"
            aria-label={orbLabel}
            title={t('voice.orb.hint', locale)}
            aria-pressed={live}
            data-testid="ceo-voice-orb"
            onClick={toggleVoice}
          >
            <span className="vnext-ceo-voice-orb-ring" aria-hidden="true" />
            {aegisAvatarUrl ? (
              <AegisAvatar src={aegisAvatarUrl} className="vnext-ceo-voice-orb-avatar" />
            ) : (
              <span className="vnext-ceo-voice-orb-fallback" aria-hidden="true">
                <VNextIcon name="voice" />
              </span>
            )}
          </button>
        </div>
      )}
    </div>
  );
});
