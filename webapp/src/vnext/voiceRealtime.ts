/**
 * xAI Grok Realtime session — audio path known to work on phones.
 *
 * Full duplex contract:
 *   - Mic uplinks while live; during assistant playback we uplink silence
 *     so speaker echo does not fire VAD and cut the reply after ~1–2s
 *   - Reply audio: WebAudio → MediaStreamDestination → <audio> (browser AEC)
 *   - speech_started only flushes local playback on *real* barge-in
 *   - Unexpected WS drop remints/reconnects (max 2) without killing the mic
 *   - Session PCM rate is the live AudioContext rate (native 48/44.1 kHz on
 *     phones). 24 kHz is only used when that is already the hardware rate.
 *
 * This module is side-effectful (mic, WS, AudioContext). Pure helpers stay in voiceAudio.ts.
 */

import {
  buildRealtimeWsUrl,
  captureQuantumSamples,
  decodeBase64Pcm16,
  float32ForSessionRate,
  float32ToPcm16Base64,
  pcm16ToPlaybackFloat32,
  pcmRatesMatch,
  playbackBufferSampleRate,
  realtimeClientProtocols,
  realtimeSessionAudioFormats,
  sessionFrameSamples,
  sessionPcmRateForContext,
  VOICE_PCM_FALLBACK_RATE,
} from './voiceAudio';
import {
  classifyDuplexHealth,
  emptyVoiceDuplexDiag,
  type VoiceDuplexDiag,
  type VoiceHealth,
} from './voiceDiag';

export const REALTIME_PCM_RATE = VOICE_PCM_FALLBACK_RATE; // 24000 — xAI / Maka default
const MAX_RECONNECT = 2;
const IDLE_HANGUP_MS = 120_000;
/** After assistant audio, keep uplink muted briefly so trailing echo is not speech. */
const POST_PLAYBACK_MUTE_MS = 350;
/** Mic peak (0..1) required to accept barge-in while we are still playing. */
const BARGE_IN_PEAK = 0.12;

export type RealtimeSessionHandlers = {
  onPhase?: (phase: 'connecting' | 'live' | 'idle') => void;
  onSpeaking?: (speaking: boolean) => void;
  onListeningUser?: (listening: boolean) => void;
  onTranscriptDelta?: (text: string) => void;
  /** Owner speech transcription (when the Realtime session provides it). */
  onUserTranscript?: (text: string, final: boolean) => void;
  onError?: (message: string) => void;
  onFunctionCall?: (callId: string, name: string, argsJson: string) => void | Promise<void>;
  /** Fired ~1/s with counters + classified health (for orb diagnostics). */
  onDiag?: (diag: VoiceDuplexDiag, health: VoiceHealth) => void;
  buildInstructions: () => string;
  tools: unknown[];
  languageHint?: string; // e.g. 'zh'
};

export type MintedRealtime = {
  client_secret: string;
  model: string;
  ws_url: string;
  max_session_seconds: number;
};

export type LiveRealtimeSession = {
  stop: (notice?: string) => void;
  /** Push updated instructions (screen context). */
  updateContext: () => void;
  /** Reply to a function call (function_call_output + response.create). */
  sendFunctionOutput: (callId: string, output: unknown) => void;
  /** Speak the next beat of this call. False when the socket is already closed. */
  continueTurn: (text: string) => boolean;
  getElapsedSeconds: () => number;
  isLive: () => boolean;
  getDiag: () => VoiceDuplexDiag;
  getHealth: () => VoiceHealth;
};

/**
 * Start a full-duplex Realtime session. Call from a user-gesture handler.
 * Throws if mic / mint / WS fails — caller should surface the error (do not
 * silently demote to one-shot STT when the owner asked for duplex).
 */
export async function startLiveRealtimeSession(
  mint: () => Promise<MintedRealtime>,
  handlers: RealtimeSessionHandlers,
): Promise<LiveRealtimeSession> {
  let stopped = false;
  let userStop = false;
  let startedAt = Date.now();
  let lastActivity = Date.now();
  let lastInstructions = '';
  let generation = 0;
  let reconnects = 0;
  let maxSeconds = 600;
  let activeResponses = 0;
  let muteUplinkUntil = 0;
  /** Negotiated with xAI. Starts at the 24 kHz server default until the context exists. */
  let sessionPcmRate = REALTIME_PCM_RATE;
  /** True after session.updated applied our rate. The server default is 24 kHz until then. */
  let outputRateReady = false;

  let ws: WebSocket | null = null;
  let audioCtx: AudioContext | null = null;
  let sinkNode: MediaStreamAudioDestinationNode | null = null;
  let sinkEl: HTMLAudioElement | null = null;
  let processor: ScriptProcessorNode | null = null;
  let micSource: MediaStreamAudioSourceNode | null = null;
  let micStream: MediaStream | null = null;
  let muteGain: GainNode | null = null;
  const scheduled: AudioBufferSourceNode[] = [];
  let nextPlay = 0;
  const timers: ReturnType<typeof setInterval>[] = [];

  const diag = emptyVoiceDuplexDiag({
    sessionPcmRate: REALTIME_PCM_RATE,
  });

  const isPlayingAssistant = (): boolean => {
    if (!audioCtx) return activeResponses > 0;
    if (activeResponses > 0) return true;
    if (scheduled.length > 0) return true;
    if (nextPlay > audioCtx.currentTime + 0.05) return true;
    if (performance.now() < muteUplinkUntil) return true;
    return false;
  };

  const snapshotDiag = (): VoiceDuplexDiag => ({
    ...diag,
    wsOpen: Boolean(ws && ws.readyState === WebSocket.OPEN),
    liveSeconds: Math.floor((Date.now() - startedAt) / 1000),
    ctxSampleRate: audioCtx?.sampleRate ?? diag.ctxSampleRate,
  });

  const emitDiag = () => {
    const snap = snapshotDiag();
    handlers.onDiag?.(snap, classifyDuplexHealth(snap));
  };

  const flushPlayback = () => {
    for (const source of scheduled) {
      try {
        source.stop();
      } catch {
        /* */
      }
    }
    scheduled.length = 0;
    nextPlay = 0;
    handlers.onSpeaking?.(false);
  };

  const playDelta = (base64: string) => {
    if (!audioCtx || !base64) return;
    const pcm = decodeBase64Pcm16(base64);
    if (pcm.length === 0) return;
    try {
      const bufferRate = playbackBufferSampleRate(audioCtx.sampleRate);
      const floats = pcm16ToPlaybackFloat32(pcm, sessionPcmRate, bufferRate);
      const buffer = audioCtx.createBuffer(1, floats.length, bufferRate);
      buffer.getChannelData(0).set(floats);
      const source = audioCtx.createBufferSource();
      source.buffer = buffer;
      source.connect(sinkNode ?? audioCtx.destination);
      const startAt = Math.max(audioCtx.currentTime, nextPlay);
      source.start(startAt);
      nextPlay = startAt + buffer.duration;
      scheduled.push(source);
      diag.playbackChunks += 1;
      handlers.onSpeaking?.(true);
      void sinkEl?.play().catch(() => undefined);
      if (audioCtx.state === 'suspended') {
        void audioCtx.resume().catch(() => undefined);
      }
      source.onended = () => {
        const idx = scheduled.indexOf(source);
        if (idx >= 0) scheduled.splice(idx, 1);
        if (scheduled.length === 0 && audioCtx && nextPlay <= audioCtx.currentTime + 0.05) {
          handlers.onSpeaking?.(false);
          muteUplinkUntil = performance.now() + POST_PLAYBACK_MUTE_MS;
        }
      };
    } catch (err) {
      diag.lastServerError = err instanceof Error ? `playback: ${err.message}` : 'playback failed';
      emitDiag();
    }
  };

  const sendSessionUpdate = () => {
    if (!ws || ws.readyState !== WebSocket.OPEN) return;
    // Never thrash session.update mid-reply (server can cancel the turn → "1–2s cut").
    if (isPlayingAssistant()) return;
    const instructions = handlers.buildInstructions();
    lastInstructions = instructions;
    const tools = Array.isArray(handlers.tools) ? handlers.tools : [];
    ws.send(JSON.stringify(buildSessionUpdatePayload({
      instructions,
      tools,
      languageHint: handlers.languageHint,
      pcmRate: sessionPcmRate,
    })));
  };

  const tearDownSocket = () => {
    const socket = ws;
    ws = null;
    if (socket && socket.readyState !== WebSocket.CLOSED) {
      try {
        socket.close();
      } catch {
        /* */
      }
    }
  };

  const stop = (notice?: string) => {
    if (stopped) return;
    stopped = true;
    userStop = true;
    generation += 1;
    for (const timer of timers) clearInterval(timer);
    timers.length = 0;
    try {
      processor?.disconnect();
    } catch {
      /* */
    }
    try {
      micSource?.disconnect();
    } catch {
      /* */
    }
    try {
      muteGain?.disconnect();
    } catch {
      /* */
    }
    processor = null;
    micSource = null;
    muteGain = null;
    micStream?.getTracks().forEach((track) => track.stop());
    micStream = null;
    flushPlayback();
    tearDownSocket();
    if (sinkEl) {
      try {
        sinkEl.pause();
        sinkEl.srcObject = null;
        sinkEl.remove();
      } catch {
        /* */
      }
      sinkEl = null;
    }
    sinkNode = null;
    void audioCtx?.close().catch(() => undefined);
    audioCtx = null;
    handlers.onPhase?.('idle');
    handlers.onSpeaking?.(false);
    handlers.onListeningUser?.(false);
    if (notice) handlers.onError?.(notice);
  };

  const bindSocketHandlers = (socket: WebSocket, gen: number) => {
    socket.onmessage = (event) => {
      if (ws !== socket || stopped || gen !== generation) return;
      lastActivity = Date.now();
      let msg: Record<string, unknown>;
      try {
        msg = JSON.parse(String(event.data)) as Record<string, unknown>;
      } catch {
        return;
      }
      const type = typeof msg.type === 'string' ? msg.type : '';
      switch (type) {
        case 'session.updated':
          // Ack of our session.update — input/output rate is now the negotiated one.
          outputRateReady = true;
          diag.sessionUpdated = true;
          emitDiag();
          break;
        case 'session.created':
          // Created still uses the server default (24 kHz) until session.updated.
          diag.sessionUpdated = true;
          emitDiag();
          break;
        case 'response.created':
          activeResponses += 1;
          break;
        case 'response.done':
          activeResponses = Math.max(0, activeResponses - 1);
          if (activeResponses === 0) {
            muteUplinkUntil = performance.now() + POST_PLAYBACK_MUTE_MS;
          }
          break;
        case 'response.output_audio.delta':
        case 'response.audio.delta': {
          const delta = typeof msg.delta === 'string' ? msg.delta : '';
          if (delta) {
            diag.audioDeltas += 1;
            playDelta(delta);
          }
          break;
        }
        case 'input_audio_buffer.speech_started': {
          // Self-echo on speakerphone used to fire VAD mid-reply and flush TTS
          // after ~1–2s ("assistant closes"). Only barge-in when not playing,
          // or when mic peak is clearly above residual echo.
          diag.speechStarts += 1;
          const playing = isPlayingAssistant();
          const loud = diag.micPeak >= BARGE_IN_PEAK;
          if (playing && !loud) {
            emitDiag();
            break;
          }
          flushPlayback();
          handlers.onListeningUser?.(true);
          emitDiag();
          break;
        }
        case 'input_audio_buffer.speech_stopped':
          diag.speechStops += 1;
          handlers.onListeningUser?.(false);
          emitDiag();
          break;
        case 'response.output_audio_transcript.delta':
        case 'response.audio_transcript.delta':
        case 'response.output_text.delta': {
          const delta = typeof msg.delta === 'string' ? msg.delta : '';
          if (delta) handlers.onTranscriptDelta?.(delta);
          break;
        }
        // Owner-side captions (xAI / OpenAI Realtime variants).
        case 'conversation.item.input_audio_transcription.delta': {
          const delta = typeof msg.delta === 'string'
            ? msg.delta
            : (typeof msg.transcript === 'string' ? msg.transcript : '');
          if (delta) handlers.onUserTranscript?.(delta, false);
          break;
        }
        case 'conversation.item.input_audio_transcription.completed':
        case 'conversation.item.input_audio_transcription.done': {
          const transcript = typeof msg.transcript === 'string'
            ? msg.transcript
            : (typeof msg.text === 'string' ? msg.text : '');
          if (transcript) handlers.onUserTranscript?.(transcript, true);
          break;
        }
        case 'response.function_call_arguments.done': {
          const callId = typeof msg.call_id === 'string' ? msg.call_id : '';
          const name = typeof msg.name === 'string' ? msg.name : '';
          const args = typeof msg.arguments === 'string' ? msg.arguments : '{}';
          if (callId && name) void handlers.onFunctionCall?.(callId, name, args);
          break;
        }
        case 'error': {
          const errObj = msg.error;
          const text = errObj && typeof errObj === 'object' && 'message' in errObj
            ? String((errObj as { message?: string }).message || 'Realtime error')
            : 'Realtime error';
          diag.lastServerError = text;
          emitDiag();
          // Recoverable: keep the session open (Maka/Hercules do not hang up on error events).
          handlers.onError?.(text);
          break;
        }
        default:
          break;
      }
    };
  };

  const openSocket = async (gen: number): Promise<void> => {
    outputRateReady = false;
    const minted = await mint();
    if (stopped || gen !== generation) throw new Error('Voice session cancelled');
    maxSeconds = Math.max(60, minted.max_session_seconds || 600);
    const wsUrl = buildRealtimeWsUrl(minted.ws_url, {
      model: minted.model || 'grok-voice-think-fast-2.0',
    });
    const protocols = realtimeClientProtocols(minted.client_secret);
    if (protocols.length === 0) {
      throw new Error('Voice mint returned an empty client token');
    }
    const socket = new WebSocket(wsUrl, protocols);
    ws = socket;
    bindSocketHandlers(socket, gen);
    let opened = false;

    await new Promise<void>((resolve, reject) => {
      const failTimer = window.setTimeout(() => {
        reject(new Error('Realtime WebSocket open timeout'));
      }, 15_000);

      socket.onopen = () => {
        window.clearTimeout(failTimer);
        if (stopped || gen !== generation) {
          reject(new Error('Voice session cancelled'));
          return;
        }
        opened = true;
        if (audioCtx?.state === 'suspended') {
          void audioCtx.resume().catch(() => undefined);
        }
        activeResponses = 0;
        lastActivity = Date.now();
        sendSessionUpdate();
        handlers.onPhase?.('live');
        emitDiag();
        resolve();
      };

      socket.onerror = () => {
        // Browsers hide the reason; onclose carries the outcome.
        if (!opened) {
          window.clearTimeout(failTimer);
        }
      };

      socket.onclose = () => {
        window.clearTimeout(failTimer);
        if (gen !== generation) return;
        if (!opened) {
          reject(new Error('Realtime WebSocket closed before open'));
          return;
        }
        if (ws !== socket && ws !== null) return;
        if (stopped || userStop) return;
        // Unexpected drop after live: remint + reconnect without killing mic.
        if (reconnects < MAX_RECONNECT) {
          reconnects += 1;
          handlers.onPhase?.('connecting');
          void openSocket(generation)
            .then(() => {
              reconnects = 0;
            })
            .catch((err) => {
              stop(err instanceof Error ? err.message : 'Voice connection lost');
            });
          return;
        }
        stop('Voice connection closed');
      };
    });
  };

  handlers.onPhase?.('connecting');

  // ── 1) Native-rate AudioContext (user gesture) ──────────────────
  // Do not pin 24 kHz. Phones keep the hardware clock (usually 48 kHz);
  // a 24 kHz pin is ignored or resampled, and that resample is the crackle.
  const AudioCtx =
    window.AudioContext
    || (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
  if (!AudioCtx) {
    throw new Error('Web Audio not available');
  }
  audioCtx = new AudioCtx();
  const ctxRate = audioCtx.sampleRate;
  sessionPcmRate = sessionPcmRateForContext(ctxRate);
  diag.ctxSampleRate = ctxRate;
  diag.sessionPcmRate = sessionPcmRate;
  const resumePromise = audioCtx.state === 'suspended'
    ? audioCtx.resume().catch(() => undefined)
    : Promise.resolve();

  // ── 2) Mic at the same rate ─────────────────────────────────────
  try {
    micStream = await navigator.mediaDevices.getUserMedia({
      audio: {
        channelCount: 1,
        sampleRate: { ideal: ctxRate },
        echoCancellation: true,
        noiseSuppression: true,
        autoGainControl: true,
      },
      video: false,
    });
  } catch (err) {
    void audioCtx.close().catch(() => undefined);
    audioCtx = null;
    throw err;
  }
  await resumePromise;
  if (stopped || !audioCtx) {
    micStream.getTracks().forEach((track) => track.stop());
    void audioCtx?.close().catch(() => undefined);
    audioCtx = null;
    throw new Error('Voice session cancelled');
  }

  // ── 3) AEC: element-routed playback (Maka critical path) ────────
  sinkNode = audioCtx.createMediaStreamDestination();
  sinkEl = document.createElement('audio');
  sinkEl.autoplay = true;
  sinkEl.setAttribute('playsinline', 'true');
  sinkEl.setAttribute('webkit-playsinline', 'true');
  sinkEl.muted = false;
  sinkEl.volume = 1;
  sinkEl.srcObject = sinkNode.stream;
  sinkEl.style.cssText = 'position:fixed;width:0;height:0;opacity:0;pointer-events:none;left:-9999px';
  document.body.appendChild(sinkEl);
  try {
    await sinkEl.play();
  } catch {
    /* first delta may resume */
  }

  // ── 4) Continuous mic uplink (created once; survives reconnect) ─
  if (audioCtx && micStream && !processor) {
    micSource = audioCtx.createMediaStreamSource(micStream);
    const quantum = captureQuantumSamples(ctxRate);
    processor = audioCtx.createScriptProcessor(quantum, 1, 1);
    const captureRate = ctxRate;
    // One ScriptProcessor quantum of silence at the *session* PCM rate.
    const silenceSamples = sessionFrameSamples(quantum, captureRate, sessionPcmRate);
    const silenceFrame = float32ToPcm16Base64(new Float32Array(silenceSamples));
    processor.onaudioprocess = (event) => {
      const raw = event.inputBuffer.getChannelData(0);
      // The processor must stay connected, but its output is not a monitor.
      event.outputBuffer.getChannelData(0).fill(0);
      let peak = 0;
      for (let i = 0; i < raw.length; i += 8) {
        const a = Math.abs(raw[i] ?? 0);
        if (a > peak) peak = a;
      }
      diag.micPeak = Math.max(peak, diag.micPeak * 0.98);

      const live = ws;
      if (!live || live.readyState !== WebSocket.OPEN || stopped) return;
      // Until session.updated, xAI still expects the 24 kHz default.
      // 48 kHz frames decoded as 24 kHz are the slow, broken speech.
      if (!outputRateReady && !pcmRatesMatch(sessionPcmRate, REALTIME_PCM_RATE)) return;

      // While assistant is talking, uplink silence so speaker echo cannot
      // trigger server VAD and cut the reply short.
      if (isPlayingAssistant()) {
        if (silenceFrame) {
          live.send(JSON.stringify({ type: 'input_audio_buffer.append', audio: silenceFrame }));
          diag.appendFrames += 1;
        }
        return;
      }

      const samples = float32ForSessionRate(raw, captureRate, sessionPcmRate);
      const audio = float32ToPcm16Base64(samples);
      if (!audio) return;
      live.send(JSON.stringify({ type: 'input_audio_buffer.append', audio }));
      diag.appendFrames += 1;
    };
    micSource.connect(processor);
    muteGain = audioCtx.createGain();
    muteGain.gain.value = 0;
    processor.connect(muteGain);
    muteGain.connect(audioCtx.destination);
  }

  // ── 5) Mint + WebSocket ─────────────────────────────────────────
  startedAt = Date.now();
  lastActivity = Date.now();
  try {
    await openSocket(generation);
  } catch (err) {
    stop();
    throw err;
  }

  timers.push(
    setInterval(() => {
      if (stopped) return;
      emitDiag();
      if (Date.now() - startedAt >= maxSeconds * 1000) {
        stop('Voice session time limit reached');
      } else if (Date.now() - lastActivity >= IDLE_HANGUP_MS) {
        stop('Voice idle disconnect');
      }
    }, 1000),
  );
  timers.push(
    setInterval(() => {
      if (stopped) return;
      if (isPlayingAssistant()) return;
      const next = handlers.buildInstructions();
      if (next !== lastInstructions) sendSessionUpdate();
    }, 5000),
  );

  return {
    stop,
    updateContext: sendSessionUpdate,
    sendFunctionOutput: (callId, output) => {
      if (!ws || ws.readyState !== WebSocket.OPEN) return;
      ws.send(
        JSON.stringify({
          type: 'conversation.item.create',
          item: {
            type: 'function_call_output',
            call_id: callId,
            output: JSON.stringify(output),
          },
        }),
      );
      ws.send(JSON.stringify({ type: 'response.create' }));
    },
    continueTurn: (text) => {
      const spoken = text.trim();
      if (!spoken || !ws || ws.readyState !== WebSocket.OPEN) return false;
      ws.send(
        JSON.stringify({
          type: 'conversation.item.create',
          item: {
            type: 'message',
            role: 'user',
            content: [{ type: 'input_text', text: spoken }],
          },
        }),
      );
      ws.send(JSON.stringify({ type: 'response.create' }));
      return true;
    },
    getElapsedSeconds: () => Math.floor((Date.now() - startedAt) / 1000),
    isLive: () => !stopped && ws?.readyState === WebSocket.OPEN,
    getDiag: () => snapshotDiag(),
    getHealth: () => classifyDuplexHealth(snapshotDiag()),
  };
}

/** Pure: session.update payload shape for tests (Maka parity). */
export function buildSessionUpdatePayload(input: {
  instructions: string;
  tools: unknown[];
  languageHint?: string;
  /** Live context rate, already snapped to an xAI PCM rate. */
  pcmRate?: number;
}) {
  const languageHint = input.languageHint?.trim();
  const audio = realtimeSessionAudioFormats(input.pcmRate ?? REALTIME_PCM_RATE);
  return {
    type: 'session.update' as const,
    session: {
      instructions: input.instructions,
      turn_detection: { type: 'server_vad' as const },
      voice: 'eve',
      audio: {
        input: {
          ...audio.input,
          ...(languageHint ? { transcription: { language_hint: languageHint } } : {}),
        },
        output: audio.output,
      },
      ...(input.tools.length > 0
        ? { tools: input.tools, tool_choice: 'auto' as const }
        : {}),
    },
  };
}
