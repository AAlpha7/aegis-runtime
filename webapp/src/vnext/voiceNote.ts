/**
 * Voice notes for the chat-home composer.
 *
 * This is NOT dictation. The composer used to run browser speech-to-text
 * (`webkitSpeechRecognition`) straight into the text field — unstable on iOS,
 * and the owner never wanted a transcript typed for him. What he wanted is a
 * Telegram-style voice note: hold the mic, speak, send audio; the ASSISTANT
 * hears it. So the composer records with `MediaRecorder`, uploads the audio to
 * `POST /webapp/vnext/upload`, and the SERVER transcribes it (local whisper.cpp
 * — the same speech-to-text path Telegram voice notes take).
 *
 * Everything in this module is pure or dependency-injected so the recorder
 * state machine is unit-testable in plain Node — it touches no browser global
 * at all (no recorder API, no microphone API, no DOM). `App.tsx` supplies the
 * browser implementations of the injected primitives.
 */

/** Hard cap on one voice note. Longer than a Telegram note usually runs, and
 *  well inside the server's 10 MB voice ceiling for any sane codec. */
export const VOICE_NOTE_MAX_MS = 180_000;

/** How often the elapsed timer ticks while recording. */
export const VOICE_NOTE_TICK_MS = 1000;

/**
 * Container preference order for `MediaRecorder`.
 *
 * Chrome/Firefox on Android and desktop support the WebM/Opus entries. iOS
 * Safari (16.4+, the first version with `MediaRecorder` at all) supports ONLY
 * `audio/mp4` (AAC) — it is last in this list purely because Opus is smaller,
 * not because it is a fallback: on iOS it is the single working choice. The
 * server sniffs all of these by magic number, so a browser that ignores our
 * request and emits something else still uploads cleanly.
 */
export const VOICE_NOTE_MIME_CANDIDATES = [
  'audio/webm;codecs=opus',
  'audio/webm',
  'audio/ogg;codecs=opus',
  'audio/ogg',
  'audio/mp4',
  'audio/aac',
] as const;

/**
 * First container this browser can actually record. Returns `null` when none
 * of the candidates is supported — the caller then lets `MediaRecorder` choose
 * its own default rather than refusing to record.
 */
export function pickVoiceNoteMimeType(
  isTypeSupported: ((mime: string) => boolean) | undefined,
): string | null {
  if (typeof isTypeSupported !== 'function') return null;
  for (const candidate of VOICE_NOTE_MIME_CANDIDATES) {
    try {
      if (isTypeSupported(candidate)) return candidate;
    } catch {
      // A browser that throws on an unknown type is just saying "no".
    }
  }
  return null;
}

/**
 * Upload filename for a recorded blob. The server picks its ffmpeg demuxer
 * from the extension, so the name must match the container — a bare `blob`
 * (what a `Blob` carries by default) would make ffmpeg guess.
 */
export function voiceNoteFileName(mimeType: string | null | undefined): string {
  const base = (mimeType ?? '').split(';')[0]?.trim().toLowerCase() ?? '';
  if (base === 'audio/webm' || base === 'video/webm') return 'voice-note.webm';
  if (base === 'audio/ogg' || base === 'video/ogg') return 'voice-note.ogg';
  if (base === 'audio/mp4' || base === 'video/mp4' || base === 'audio/x-m4a') return 'voice-note.m4a';
  if (base === 'audio/aac') return 'voice-note.aac';
  if (base === 'audio/mpeg') return 'voice-note.mp3';
  if (base === 'audio/wav' || base === 'audio/x-wav') return 'voice-note.wav';
  return 'voice-note.webm';
}

/** `M:SS` elapsed label for the recording pill. Never negative, never `NaN`. */
export function formatVoiceElapsed(elapsedMs: number): string {
  const safe = Number.isFinite(elapsedMs) && elapsedMs > 0 ? elapsedMs : 0;
  const totalSeconds = Math.floor(safe / 1000);
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${String(seconds).padStart(2, '0')}`;
}

/** Duration label for a sent voice note (`0:07`), or `''` when unknown. */
export function formatVoiceDuration(durationSecs: number | null | undefined): string {
  if (typeof durationSecs !== 'number' || !Number.isFinite(durationSecs) || durationSecs < 0) return '';
  return formatVoiceElapsed(Math.round(durationSecs) * 1000);
}

/** Has this recording hit the hard cap and must stop itself? */
export function shouldAutoStopVoiceNote(elapsedMs: number, maxMs: number = VOICE_NOTE_MAX_MS): boolean {
  return Number.isFinite(elapsedMs) && elapsedMs >= maxMs;
}

/** One-line preview of a transcript for the composer pill. */
export function voiceNoteTranscriptPreview(transcript: string, maxChars = 80): string {
  const collapsed = transcript.replace(/\s+/g, ' ').trim();
  if (collapsed.length <= maxChars) return collapsed;
  return `${collapsed.slice(0, maxChars - 1)}…`;
}

// ---------------------------------------------------------------------------
// State machine
// ---------------------------------------------------------------------------

/** Why a voice note failed. Mapped to a localized string by the caller — this
 *  module never holds user-facing copy. */
export type VoiceNoteErrorCode =
  | 'unsupported'
  | 'permission'
  | 'record'
  | 'upload'
  | 'empty';

export type VoiceNoteState =
  | { status: 'idle' }
  | { status: 'recording'; elapsedMs: number }
  | { status: 'uploading' }
  | { status: 'attached'; transcript: string; durationSecs: number | null }
  | { status: 'error'; code: VoiceNoteErrorCode };

export type VoiceNoteEvent =
  | { type: 'start' }
  | { type: 'tick'; elapsedMs: number }
  | { type: 'stop' }
  | { type: 'uploaded'; transcript: string; durationSecs: number | null }
  | { type: 'failed'; code: VoiceNoteErrorCode }
  | { type: 'cancel' }
  | { type: 'clear' };

export const IDLE_VOICE_NOTE: VoiceNoteState = { status: 'idle' };

/**
 * Pure transition table: idle → recording → uploading → attached, with
 * `cancel`/`clear` returning to idle and `failed` parking on an error the
 * composer can show. Unknown transitions are no-ops (a late `tick` after
 * `stop`, a duplicate `uploaded` from a retried request) so a race can never
 * resurrect a cancelled recording.
 */
export function voiceNoteReducer(state: VoiceNoteState, event: VoiceNoteEvent): VoiceNoteState {
  switch (event.type) {
    case 'start':
      // Starting a new note always replaces whatever was held before —
      // except mid-upload, which must settle first (its result would
      // otherwise land on the new recording).
      return state.status === 'uploading' ? state : { status: 'recording', elapsedMs: 0 };
    case 'tick':
      if (state.status !== 'recording') return state;
      return { status: 'recording', elapsedMs: Math.max(0, Math.min(event.elapsedMs, VOICE_NOTE_MAX_MS)) };
    case 'stop':
      return state.status === 'recording' ? { status: 'uploading' } : state;
    case 'uploaded':
      if (state.status !== 'uploading') return state;
      return {
        status: 'attached',
        transcript: event.transcript,
        durationSecs: event.durationSecs,
      };
    case 'failed':
      if (state.status !== 'recording' && state.status !== 'uploading') return state;
      return { status: 'error', code: event.code };
    case 'cancel':
    case 'clear':
      return IDLE_VOICE_NOTE;
    default:
      return state;
  }
}

/** Is the composer holding a voice note ready to send? */
export function voiceNoteAttachment(
  state: VoiceNoteState,
): { transcript: string; durationSecs: number | null } | null {
  return state.status === 'attached'
    ? { transcript: state.transcript, durationSecs: state.durationSecs }
    : null;
}

/** Does this state block the composer's normal text row (recording UI shown)? */
export function voiceNoteOccupiesComposer(state: VoiceNoteState): boolean {
  return state.status === 'recording' || state.status === 'uploading';
}

// ---------------------------------------------------------------------------
// Recorder driver (dependency-injected — no browser globals in this module)
// ---------------------------------------------------------------------------

/** The slice of `MediaRecorder` we use. */
export interface VoiceNoteRecorderLike {
  start: (timesliceMs?: number) => void;
  stop: () => void;
  ondataavailable: ((event: { data: unknown }) => void) | null;
  onstop: (() => void) | null;
  onerror: ((event: unknown) => void) | null;
  mimeType?: string;
}

/** The slice of `MediaStream` we use (we only ever stop its tracks). */
export interface VoiceNoteStreamLike {
  getTracks: () => Array<{ stop: () => void }>;
}

// ---------------------------------------------------------------------------
// Microphone session
// ---------------------------------------------------------------------------

/**
 * How long a granted microphone stream stays open after a recording ends.
 *
 * iOS home-screen web apps ask for microphone permission on EVERY
 * `getUserMedia` call, so releasing the stream after each note meant a
 * permission sheet before every recording (owner, 2026-09-13: "每次都问权限").
 * Keeping the granted stream alive for a few minutes lets consecutive notes
 * reuse it — no prompt, no second `getUserMedia`. The page drops the stream
 * as soon as it is hidden, so the mic indicator never outlives the
 * conversation.
 */
export const VOICE_MIC_IDLE_RELEASE_MS = 5 * 60_000;

export interface VoiceMicTrackLike {
  stop: () => void;
  readyState?: string;
}

export interface VoiceMicStreamLike {
  getTracks: () => VoiceMicTrackLike[];
}

export interface VoiceMicSessionDeps {
  /** `navigator.mediaDevices.getUserMedia({ audio: true })`. */
  getUserMedia: () => Promise<VoiceMicStreamLike>;
  /** `setTimeout`-shaped. */
  setTimer: (callback: () => void, afterMs: number) => number;
  /** `clearTimeout`-shaped. */
  clearTimer: (handle: number) => void;
  idleMs?: number;
}

export interface VoiceMicSession {
  /** A live stream: the held one when its tracks are still live, else a fresh grant. */
  acquire: () => Promise<VoiceMicStreamLike>;
  /** A recording finished with this stream: keep it warm, stop it after `idleMs`. */
  release: (stream: VoiceMicStreamLike) => void;
  /** Stop the held stream now (page hidden, unmount). */
  dispose: () => void;
  isHeld: () => boolean;
}

function stopMicStream(stream: VoiceMicStreamLike): void {
  try {
    for (const track of stream.getTracks()) track.stop();
  } catch {
    // Best-effort release; never let it mask a result.
  }
}

function micStreamIsLive(stream: VoiceMicStreamLike): boolean {
  try {
    const tracks = stream.getTracks();
    return tracks.length > 0
      && tracks.every((track) => track.readyState === undefined || track.readyState === 'live');
  } catch {
    return false;
  }
}

export function createVoiceMicSession(deps: VoiceMicSessionDeps): VoiceMicSession {
  const idleMs = deps.idleMs ?? VOICE_MIC_IDLE_RELEASE_MS;
  let held: VoiceMicStreamLike | null = null;
  let timer: number | null = null;
  const clearIdleTimer = () => {
    if (timer !== null) {
      deps.clearTimer(timer);
      timer = null;
    }
  };
  const dropHeld = () => {
    clearIdleTimer();
    if (held) {
      stopMicStream(held);
      held = null;
    }
  };
  return {
    acquire: async () => {
      clearIdleTimer();
      if (held && micStreamIsLive(held)) return held;
      // The OS ended the old tracks (backgrounded app, unplugged headset):
      // a stale stream would record silence.
      dropHeld();
      const stream = await deps.getUserMedia();
      held = stream;
      return stream;
    },
    release: (stream) => {
      if (stream !== held) {
        stopMicStream(stream);
        return;
      }
      clearIdleTimer();
      timer = deps.setTimer(() => {
        timer = null;
        dropHeld();
      }, idleMs);
    },
    dispose: dropHeld,
    isHeld: () => held !== null,
  };
}

export interface VoiceNoteRecorderDeps {
  /** `navigator.mediaDevices.getUserMedia({ audio: true })` (or a mic session's `acquire`). */
  getUserMedia: () => Promise<VoiceNoteStreamLike>;
  /**
   * Hand the stream back when the recording ends instead of stopping its
   * tracks — a `VoiceMicSession` keeps it warm for the next note. Absent →
   * the tracks are stopped (mic released) as before.
   */
  releaseStream?: (stream: VoiceNoteStreamLike) => void;
  /** `new MediaRecorder(stream, mimeType ? { mimeType } : undefined)`. */
  createRecorder: (stream: VoiceNoteStreamLike, mimeType: string | null) => VoiceNoteRecorderLike;
  /** `MediaRecorder.isTypeSupported`, when the browser has one. */
  isTypeSupported?: (mime: string) => boolean;
  /** Assemble the recorded chunks into the upload payload. */
  makeFile: (chunks: unknown[], filename: string, mimeType: string) => File;
  now: () => number;
  /** `setInterval`-shaped. */
  schedule: (callback: () => void, everyMs: number) => number;
  /** `clearInterval`-shaped. */
  unschedule: (handle: number) => void;
  maxMs?: number;
  tickMs?: number;
}

export interface VoiceNoteRecorderCallbacks {
  onElapsed: (elapsedMs: number) => void;
  /** Recording finished with audio to upload. */
  onComplete: (file: File, elapsedMs: number) => void;
  onError: (code: VoiceNoteErrorCode) => void;
}

export interface VoiceNoteRecorderHandle {
  /** Stop and hand the audio to `onComplete`. */
  stop: () => void;
  /** Stop and discard — `onComplete`/`onError` never fire afterwards. */
  cancel: () => void;
}

/**
 * Start recording a voice note.
 *
 * Resolves once the recorder is running (mic permission granted); rejects
 * never — every failure is reported through `onError` with a code so the
 * composer shows one consistent error pill. The returned handle is inert after
 * the first `stop`/`cancel`, so a double-tap on Stop cannot upload twice.
 */
export async function startVoiceNoteRecording(
  deps: VoiceNoteRecorderDeps,
  callbacks: VoiceNoteRecorderCallbacks,
): Promise<VoiceNoteRecorderHandle | null> {
  const maxMs = deps.maxMs ?? VOICE_NOTE_MAX_MS;
  const tickMs = deps.tickMs ?? VOICE_NOTE_TICK_MS;
  let stream: VoiceNoteStreamLike;
  try {
    stream = await deps.getUserMedia();
  } catch {
    // Chrome/Safari both surface a denied prompt as a rejected promise; we do
    // not distinguish "denied" from "no microphone" because the owner's next
    // step (check the site's mic permission) is the same either way.
    callbacks.onError('permission');
    return null;
  }

  const stopTracks = () => {
    try {
      if (deps.releaseStream) {
        deps.releaseStream(stream);
        return;
      }
      for (const track of stream.getTracks()) track.stop();
    } catch {
      // Releasing the mic is best-effort; never let it mask a result.
    }
  };

  const mimeType = pickVoiceNoteMimeType(deps.isTypeSupported);
  let recorder: VoiceNoteRecorderLike;
  try {
    recorder = deps.createRecorder(stream, mimeType);
  } catch {
    stopTracks();
    callbacks.onError('unsupported');
    return null;
  }

  const chunks: unknown[] = [];
  const startedAt = deps.now();
  let settled = false;
  let cancelled = false;
  let timer: number | null = null;

  const clearTimer = () => {
    if (timer !== null) {
      deps.unschedule(timer);
      timer = null;
    }
  };

  const finish = (outcome: () => void) => {
    if (settled) return;
    settled = true;
    clearTimer();
    stopTracks();
    outcome();
  };

  recorder.ondataavailable = (event) => {
    const data = (event as { data?: unknown } | undefined)?.data;
    if (data) chunks.push(data);
  };

  recorder.onerror = () => {
    finish(() => callbacks.onError('record'));
  };

  recorder.onstop = () => {
    if (cancelled) {
      finish(() => {});
      return;
    }
    const elapsedMs = deps.now() - startedAt;
    if (chunks.length === 0) {
      finish(() => callbacks.onError('record'));
      return;
    }
    const effectiveMime = recorder.mimeType || mimeType || 'audio/webm';
    const filename = voiceNoteFileName(effectiveMime);
    let file: File;
    try {
      file = deps.makeFile(chunks, filename, effectiveMime);
    } catch {
      finish(() => callbacks.onError('record'));
      return;
    }
    finish(() => callbacks.onComplete(file, elapsedMs));
  };

  try {
    recorder.start();
  } catch {
    finish(() => callbacks.onError('record'));
    return null;
  }

  const stopRecorder = () => {
    try {
      recorder.stop();
    } catch {
      // Already stopped (or never started): fall through to the same result
      // path so a failed stop still releases the mic.
      recorder.onstop?.();
    }
  };

  timer = deps.schedule(() => {
    if (settled) {
      clearTimer();
      return;
    }
    const elapsedMs = deps.now() - startedAt;
    callbacks.onElapsed(elapsedMs);
    if (shouldAutoStopVoiceNote(elapsedMs, maxMs)) {
      clearTimer();
      stopRecorder();
    }
  }, tickMs);

  return {
    stop: () => {
      if (settled) return;
      clearTimer();
      stopRecorder();
    },
    cancel: () => {
      if (settled) return;
      cancelled = true;
      clearTimer();
      stopRecorder();
    },
  };
}
