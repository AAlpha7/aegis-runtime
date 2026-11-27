import { useEffect, useRef, useState } from 'react';
import type { ContextualAssistantResponse } from '../api';
import { AssistantResponseView } from './AssistantResponseView';
import { assistantResultIsBusy } from './interactionState';
import { VNextIcon } from './icons';
import { assistantResultTitle, classNames } from './presentation';
import { RichText } from './RichText';
import { chatFullTimestampLabel, chatTimestampLabel, normalizeMessageTimestamp } from './time';
import { useUserTimezone } from './UserTimezoneContext';
import { useLocale, useT } from '../i18n/LocaleContext';
import { tf } from '../i18n/catalog';

export type InlineAssistantTurn = {
  id: string;
  role: 'owner' | 'aegis' | 'working' | 'queued';
  text?: string;
  timestamp?: number;
  result?: ContextualAssistantResponse;
};

export type InlineAssistantSuggestion = {
  id: string;
  label: string;
  prompt: string;
};

function resizeInlineAssistantInput(input: HTMLTextAreaElement | null) {
  if (!input) return;
  const computed = window.getComputedStyle(input);
  const maxHeight = Number.parseFloat(computed.maxHeight || '') || 128;
  input.style.height = 'auto';
  input.style.height = `${Math.min(input.scrollHeight, maxHeight)}px`;
}

function InlineTurnTime({ timestamp }: { timestamp?: number }) {
  const [expanded, setExpanded] = useState(false);
  const userTimezone = useUserTimezone();
  const { locale } = useLocale();
  const normalized = normalizeMessageTimestamp(timestamp);
  const label = expanded
    ? chatFullTimestampLabel(normalized, userTimezone)
    : chatTimestampLabel(normalized, userTimezone);
  const fullLabel = chatFullTimestampLabel(normalized, userTimezone);
  if (!label || !normalized) return null;
  return (
    <time
      className="vnext-inline-turn-time"
      dateTime={new Date(normalized).toISOString()}
      role="button"
      tabIndex={0}
      title={fullLabel}
      aria-label={
        expanded
          ? tf('inlineAssistant.aria.hideFullTime', locale, { time: fullLabel })
          : tf('inlineAssistant.aria.showFullTime', locale, { time: fullLabel })
      }
      onClick={() => setExpanded((current) => !current)}
      onKeyDown={(event) => {
        if (event.key !== 'Enter' && event.key !== ' ') return;
        event.preventDefault();
        setExpanded((current) => !current);
      }}
    >
      {label}
    </time>
  );
}

export function InlineAssistantPanel({
  label,
  text,
  placeholder,
  result,
  turns = [],
  suggestions = [],
  extraXLinks = [],
  busy,
  onChange,
  onSend,
  onClose,
  onRetry,
  retryLabel,
}: {
  label?: string;
  text: string;
  placeholder: string;
  result: ContextualAssistantResponse | null;
  turns?: InlineAssistantTurn[];
  suggestions?: InlineAssistantSuggestion[];
  /** Folded into the latest answer's X handoff so the dock does not repeat it. */
  extraXLinks?: string[];
  busy: boolean;
  onChange: (value: string) => void;
  onSend: (value: string) => void;
  onClose: () => void;
  onRetry?: () => void;
  retryLabel?: string;
}) {
  const panelRef = useRef<HTMLElement | null>(null);
  const inputRef = useRef<HTMLTextAreaElement | null>(null);
  const t = useT();
  const { locale } = useLocale();
  // DUAL-USE: `retryLabel` arrives as the English logic key ('Check result' /
  // 'Try again') so callers can compare it; localize only the displayed text.
  const retryDisplayLabel = retryLabel === 'Check result'
    ? t('inlineAssistant.retry.checkResult')
    : t('inlineAssistant.retry.tryAgain');
  const working = Boolean(result && assistantResultIsBusy(result.response_kind));
  const resolvedLabel = label ?? t('inlineAssistant.label');
  const lastAnswerIndex = turns.reduce((found, turn, index) => (
    turn.role === 'aegis' && turn.result ? index : found
  ), -1);
  const conversation = turns.length > 0 ? (
    <div className="vnext-inline-assistant-thread" aria-label={t('inlineAssistant.aria.conversation')}>
      {turns.map((turn, index) => (
        <div key={turn.id} className={classNames('vnext-inline-turn', turn.role)}>
          {(turn.role === 'owner' || turn.role === 'queued') && (
            <>
              <span className="vnext-inline-turn-label">{turn.role === 'queued' ? t('inlineAssistant.turn.queued') : t('inlineAssistant.turn.you')}</span>
              <InlineTurnTime timestamp={turn.timestamp} />
              <p>{turn.text}</p>
            </>
          )}
          {turn.role === 'working' && (
            <>
              <span className="vnext-inline-turn-label">{t('inlineAssistant.turn.aegis')}</span>
              <InlineTurnTime timestamp={turn.timestamp} />
              <p>{turn.text || t('inlineAssistant.turn.working')}</p>
            </>
          )}
          {turn.role === 'aegis' && turn.result && (
            <>
              <h3>
                <span>{assistantResultTitle(turn.result, locale)}</span>
                <InlineTurnTime timestamp={turn.timestamp} />
              </h3>
              <AssistantResponseView
                result={turn.result}
                extraXLinks={index === lastAnswerIndex ? extraXLinks : []}
              />
              {turn.result.suggested_intent && <p className="vnext-muted">{turn.result.suggested_intent.summary}</p>}
              {turn.result.draft && <RichText content={turn.result.draft} className="vnext-rich-text-draft" />}
            </>
          )}
        </div>
      ))}
      {result?.response_kind === 'error' && onRetry && (
        <div className="vnext-action-row">
          <button type="button" onClick={onRetry}>
            {busy
              ? (retryLabel === 'Check result' ? t('inlineAssistant.retry.queueCheck') : t('inlineAssistant.retry.queueRetry'))
              : retryDisplayLabel}
          </button>
        </div>
      )}
    </div>
  ) : result ? (
    <div className="vnext-inline-assistant-result">
      <h3>{assistantResultTitle(result, locale)}</h3>
      <AssistantResponseView result={result} extraXLinks={extraXLinks} />
      {result.suggested_intent && <p className="vnext-muted">{result.suggested_intent.summary}</p>}
      {result.draft && <RichText content={result.draft} className="vnext-rich-text-draft" />}
      {result.response_kind === 'error' && onRetry && (
        <div className="vnext-action-row">
          <button type="button" onClick={onRetry}>
            {busy
              ? (retryLabel === 'Check result' ? t('inlineAssistant.retry.queueCheck') : t('inlineAssistant.retry.queueRetry'))
              : retryDisplayLabel}
          </button>
        </div>
      )}
    </div>
  ) : null;

  useEffect(() => {
    window.setTimeout(() => {
      inputRef.current?.focus({ preventScroll: true });
      resizeInlineAssistantInput(inputRef.current);
    }, 0);
  }, []);

  useEffect(() => {
    resizeInlineAssistantInput(inputRef.current);
  }, [text]);

  return (
    <section
      ref={panelRef}
      className={classNames('vnext-inline-assistant', working && 'is-working')}
      aria-live="polite"
      aria-busy={working}
    >
      <div className="vnext-mini-heading-row">
        <h3>{resolvedLabel}</h3>
        <button type="button" className="vnext-quiet-button" onClick={onClose}>{t('inlineAssistant.collapse')}</button>
      </div>
      {conversation}
      {suggestions.length > 0 && turns.length === 0 && (
        <div className="vnext-inline-suggestions" aria-label={t('inlineAssistant.aria.suggestions')}>
          {suggestions.map((suggestion) => (
            <button
              key={suggestion.id}
              type="button"
              onClick={() => onSend(suggestion.prompt)}
            >
              {suggestion.label}
            </button>
          ))}
        </div>
      )}
      <form
        className="vnext-inline-assistant-form"
        onSubmit={(event) => {
          event.preventDefault();
          onSend(inputRef.current?.value ?? text);
        }}
      >
        <textarea
          ref={inputRef}
          aria-label={t('inlineAssistant.aria.ask')}
          value={text}
          placeholder={placeholder}
          rows={1}
          onChange={(event) => {
            resizeInlineAssistantInput(event.currentTarget);
            onChange(event.currentTarget.value);
          }}
          onKeyDown={(event) => {
            if (event.key !== 'Enter' || event.shiftKey || event.nativeEvent.isComposing) return;
            event.preventDefault();
            onSend(inputRef.current?.value ?? text);
          }}
        />
        <button
          type="submit"
          disabled={busy && !text.trim()}
          aria-label={busy ? t('inlineAssistant.aria.queueForAegis') : t('inlineAssistant.aria.sendToAegis')}
          title={busy ? t('inlineAssistant.aria.queueForAegis') : t('inlineAssistant.aria.sendToAegis')}
        >
          <VNextIcon name="send" />
          <span className="vnext-inline-send-label">{busy ? t('inlineAssistant.send.queue') : t('inlineAssistant.send.send')}</span>
        </button>
      </form>
    </section>
  );
}
