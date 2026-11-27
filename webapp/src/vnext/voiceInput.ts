import type { Locale } from '../i18n/catalog';
import { t, tf } from '../i18n/catalog';
import { resolveCeoShorthand } from './ceoIntent';

export type BrowserSpeechRecognitionResultEvent = {
  results: {
    length: number;
    [index: number]: {
      length: number;
      [index: number]: {
        transcript?: string;
      };
    };
  };
};

export type BrowserSpeechRecognition = {
  lang: string;
  interimResults: boolean;
  maxAlternatives: number;
  onstart: (() => void) | null;
  onerror: ((event: { error?: string }) => void) | null;
  onresult: ((event: BrowserSpeechRecognitionResultEvent) => void) | null;
  onend: (() => void) | null;
  start: () => void;
  stop?: () => void;
  abort?: () => void;
};

export type BrowserSpeechRecognitionConstructor = new () => BrowserSpeechRecognition;

export type BrowserSpeechRecognitionWindow = Window & typeof globalThis & {
  SpeechRecognition?: BrowserSpeechRecognitionConstructor;
  webkitSpeechRecognition?: BrowserSpeechRecognitionConstructor;
};

export function browserSpeechRecognitionConstructor(
  speechWindow: BrowserSpeechRecognitionWindow,
): BrowserSpeechRecognitionConstructor | null {
  return speechWindow.SpeechRecognition ?? speechWindow.webkitSpeechRecognition ?? null;
}

export function transcriptFromSpeechEvent(event: BrowserSpeechRecognitionResultEvent): string {
  const phrases: string[] = [];
  for (let resultIndex = 0; resultIndex < event.results.length; resultIndex += 1) {
    const result = event.results[resultIndex];
    if (!result || result.length === 0) continue;
    const transcript = result[0]?.transcript?.trim();
    if (transcript) phrases.push(transcript);
  }
  return phrases.join(' ').trim();
}

export function appendTranscript(current: string, transcript: string): string {
  if (!current.trim()) return transcript;
  return `${current.trimEnd()} ${transcript}`;
}

/**
 * CEO voice capture: browser STT fills the composer. When the transcript is a
 * known CEO shorthand (or very short), optional auto-send fires so one word of
 * speech can act without a second tap — aligned with the CEO Intent Contract.
 */
export function startBrowserVoiceInput({
  SpeechRecognition,
  language,
  locale = 'zh',
  setValue,
  setNotice,
  onTranscriptCaptured,
  onAutoSend,
}: {
  SpeechRecognition: BrowserSpeechRecognitionConstructor | null;
  language: string;
  /** Owner display language for notices (defaults zh for this product). */
  locale?: Locale;
  setValue: (updater: (current: string) => string) => void;
  setNotice: (notice: string) => void;
  onTranscriptCaptured?: () => void;
  /**
   * When set, a resolved CEO shorthand transcript is auto-submitted after fill
   * (speak → act). Free text still requires explicit Send.
   */
  onAutoSend?: (transcript: string) => void;
}) {
  if (!SpeechRecognition) {
    setNotice(t('voice.notice.unavailable', locale));
    return null;
  }

  let captured = false;
  let failed = false;
  let autoSent = false;
  const recognition = new SpeechRecognition();
  recognition.lang = language || (locale === 'en' ? 'en-US' : 'zh-CN');
  recognition.interimResults = false;
  recognition.maxAlternatives = 1;
  recognition.onstart = () => {
    setNotice(t('voice.notice.listening', locale));
  };
  recognition.onerror = (event) => {
    failed = true;
    const reason = event.error ? ` (${event.error})` : '';
    setNotice(tf('voice.notice.error', locale, { reason }));
  };
  recognition.onresult = (event) => {
    const transcript = transcriptFromSpeechEvent(event);
    if (!transcript) return;
    captured = true;
    setValue((current) => appendTranscript(current, transcript));
    onTranscriptCaptured?.();
    const shorthand = resolveCeoShorthand(transcript, locale);
    if (shorthand && onAutoSend) {
      autoSent = true;
      setNotice(t('voice.notice.capturedSending', locale));
      // Defer so React state for the composer settles before submit.
      queueMicrotask(() => onAutoSend(transcript));
      return;
    }
    setNotice(t('voice.notice.captured', locale));
  };
  recognition.onend = () => {
    if (!captured && !failed && !autoSent) {
      setNotice(t('voice.notice.empty', locale));
    }
  };

  try {
    recognition.start();
    return recognition;
  } catch {
    setNotice(t('voice.notice.startFailed', locale));
    return null;
  }
}

export function stopBrowserVoiceInput(recognition: BrowserSpeechRecognition | null | undefined) {
  if (!recognition) return;
  try {
    recognition.stop?.();
  } catch {
    try {
      recognition.abort?.();
    } catch {
      // already stopped
    }
  }
}
