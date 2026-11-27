/**
 * CEO Voice Realtime audio helpers (pure).
 * Duplex PCM follows the live AudioContext rate (xAI native-rate path).
 * Forcing 24 kHz on a 48 kHz phone resamples every playback chunk.
 */

export const XAI_PCM_RATES = [8000, 16000, 22050, 24000, 32000, 44100, 48000] as const;
export type XaiPcmRate = (typeof XAI_PCM_RATES)[number];
export const VOICE_PCM_FALLBACK_RATE: XaiPcmRate = 24_000;

export function nearestXaiPcmRate(deviceRate: number): XaiPcmRate {
  if (!Number.isFinite(deviceRate) || deviceRate <= 0) return VOICE_PCM_FALLBACK_RATE;
  let best: XaiPcmRate = VOICE_PCM_FALLBACK_RATE;
  let bestDist = Infinity;
  for (const r of XAI_PCM_RATES) {
    const d = Math.abs(r - deviceRate);
    if (d < bestDist) {
      best = r;
      bestDist = d;
    }
  }
  return best;
}

export function buildRealtimeWsUrl(
  baseWsUrl: string,
  opts: { readonly model: string; readonly conversationId?: string | null },
): string {
  const base = baseWsUrl.trim() || 'wss://api.x.ai/v1/realtime';
  const params = new URLSearchParams({
    model: opts.model.trim() || 'grok-voice-think-fast-2.0',
  });
  if (opts.conversationId?.trim()) {
    params.set('conversation_id', opts.conversationId.trim());
  }
  const join = base.includes('?') ? '&' : '?';
  return `${base}${join}${params.toString()}`;
}

export function realtimeClientProtocols(clientSecret: string): string[] {
  const t = clientSecret.trim();
  if (!t) return [];
  return [`xai-client-secret.${t}`];
}

export function decodeBase64Pcm16(base64: string): Int16Array {
  if (!base64) return new Int16Array(0);
  const binary = atob(base64);
  const byteLen = binary.length - (binary.length % 2);
  if (byteLen <= 0) return new Int16Array(0);
  const bytes = new Uint8Array(byteLen);
  for (let i = 0; i < byteLen; i += 1) bytes[i] = binary.charCodeAt(i) & 0xff;
  const out = new Int16Array(byteLen / 2);
  const view = new DataView(bytes.buffer, bytes.byteOffset, byteLen);
  for (let i = 0; i < out.length; i += 1) {
    out[i] = view.getInt16(i * 2, true);
  }
  return out;
}

/**
 * Encode float32 mono samples as little-endian PCM16 base64 for xAI Realtime
 * `input_audio_buffer.append`. Matches Maka/Hercules byte path (byteOffset-safe).
 */
export function float32ToPcm16Base64(input: Float32Array): string {
  const pcm = new Int16Array(input.length);
  for (let i = 0; i < input.length; i += 1) {
    const s = Math.max(-1, Math.min(1, input[i] ?? 0));
    pcm[i] = s < 0 ? s * 0x8000 : s * 0x7fff;
  }
  const bytes = new Uint8Array(pcm.buffer, pcm.byteOffset, pcm.byteLength);
  let binary = '';
  for (let i = 0; i < bytes.length; i += 1) binary += String.fromCharCode(bytes[i] ?? 0);
  return btoa(binary);
}

/**
 * When the browser ignores `new AudioContext({ sampleRate: 24000 })` and runs at
 * 44.1/48 kHz, naive 24 kHz session PCM makes ASR hear chipmunk audio / silence
 * (Hercules voiceAudio note). Downsample before encode so uplink matches session rate.
 */
export function downsampleFloat32(
  input: Float32Array,
  fromRate: number,
  toRate: number,
): Float32Array {
  if (!Number.isFinite(fromRate) || !Number.isFinite(toRate) || fromRate <= 0 || toRate <= 0) {
    return input;
  }
  if (Math.abs(fromRate - toRate) < 1) return input;
  if (fromRate < toRate) return input; // upsample rare; leave as-is
  const ratio = fromRate / toRate;
  const outLen = Math.max(1, Math.floor(input.length / ratio));
  const out = new Float32Array(outLen);
  for (let i = 0; i < outLen; i += 1) {
    // Simple average over the source window — good enough for speech VAD.
    const start = Math.floor(i * ratio);
    const end = Math.min(input.length, Math.floor((i + 1) * ratio));
    let sum = 0;
    let n = 0;
    for (let j = start; j < end; j += 1) {
      sum += input[j] ?? 0;
      n += 1;
    }
    out[i] = n > 0 ? sum / n : 0;
  }
  return out;
}

const RATE_MATCH_HZ = 50;

/** True when resampling would only invent a few hertz of drift. */
export function pcmRatesMatch(a: number, b: number): boolean {
  return Number.isFinite(a) && Number.isFinite(b) && Math.abs(a - b) < RATE_MATCH_HZ;
}

/**
 * Session PCM rate follows the live AudioContext.
 * Phones ignore `new AudioContext({ sampleRate: 24000 })` and stay at
 * 44.1/48 kHz. Negotiating that hardware rate with xAI avoids a resample
 * on both the mic uplink and the speaker downlink.
 * 24 kHz remains the fallback when the context rate is unknown or already 24 kHz.
 */
export function sessionPcmRateForContext(ctxSampleRate: number): XaiPcmRate {
  if (!Number.isFinite(ctxSampleRate) || ctxSampleRate <= 0) return VOICE_PCM_FALLBACK_RATE;
  if (pcmRatesMatch(ctxSampleRate, VOICE_PCM_FALLBACK_RATE)) return VOICE_PCM_FALLBACK_RATE;
  return nearestXaiPcmRate(ctxSampleRate);
}

const SCRIPT_PROCESSOR_SIZES = [256, 512, 1024, 2048, 4096, 8192, 16384] as const;

/**
 * ScriptProcessor quantum for ~85 ms.
 * 2048 frames is 85 ms only at 24 kHz. At the 48 kHz hardware rate that
 * same quantum is 43 ms, and the main-thread callback underruns into
 * crackle on the shared playback context.
 */
export function captureQuantumSamples(sampleRate: number): number {
  const rate = Number.isFinite(sampleRate) && sampleRate > 0 ? sampleRate : VOICE_PCM_FALLBACK_RATE;
  const target = rate * 0.085;
  let best: (typeof SCRIPT_PROCESSOR_SIZES)[number] = 2048;
  let bestDist = Infinity;
  for (const size of SCRIPT_PROCESSOR_SIZES) {
    const dist = Math.abs(size - target);
    if (dist < bestDist) {
      best = size;
      bestDist = dist;
    }
  }
  return best;
}

/** One capture quantum, counted in session-rate samples (silence frames use this). */
export function sessionFrameSamples(
  captureQuantum: number,
  captureRate: number,
  sessionRate: number,
): number {
  const quantum = Number.isFinite(captureQuantum) && captureQuantum > 0 ? captureQuantum : 1;
  if (pcmRatesMatch(captureRate, sessionRate)) return Math.max(1, Math.round(quantum));
  if (!(captureRate > 0) || !(sessionRate > 0)) return Math.max(1, Math.round(quantum));
  return Math.max(1, Math.round((quantum * sessionRate) / captureRate));
}

/**
 * Playback buffers must be declared at the live AudioContext rate.
 * A 24 kHz buffer inside a 48 kHz context is resampled per chunk
 * (crackle / clipping). Engines that ignore the buffer rate instead
 * play those samples at the wrong speed.
 */
export function playbackBufferSampleRate(ctxSampleRate: number): number {
  if (!Number.isFinite(ctxSampleRate) || ctxSampleRate <= 0) return VOICE_PCM_FALLBACK_RATE;
  return ctxSampleRate;
}

/** PCM16 → float in [-1, 1). Positive full scale stays below +1 so playback cannot clip. */
export function pcm16ToUnitFloat32(pcm: Int16Array): Float32Array {
  const out = new Float32Array(pcm.length);
  for (let i = 0; i < pcm.length; i += 1) {
    out[i] = (pcm[i] ?? 0) / 32768;
  }
  return out;
}

function resampleLinear(input: Float32Array, fromRate: number, toRate: number): Float32Array {
  if (pcmRatesMatch(fromRate, toRate) || input.length === 0) return input;
  if (!(fromRate > 0) || !(toRate > 0)) return input;
  const outLen = Math.max(1, Math.round((input.length * toRate) / fromRate));
  const out = new Float32Array(outLen);
  const scale = fromRate / toRate;
  const last = input.length - 1;
  for (let i = 0; i < outLen; i += 1) {
    const pos = i * scale;
    const i0 = Math.min(last, Math.floor(pos));
    const i1 = Math.min(last, i0 + 1);
    const frac = pos - i0;
    const a = input[i0] ?? 0;
    const b = input[i1] ?? 0;
    out[i] = a + (b - a) * frac;
  }
  return out;
}

/**
 * Assistant PCM at `sessionRate`, as float samples for a buffer declared
 * at the context rate. Duration is preserved (no chipmunk / slow-play).
 * Level is not boosted.
 */
export function pcm16ToPlaybackFloat32(
  pcm: Int16Array,
  sessionRate: number,
  ctxRate: number,
): Float32Array {
  const floats = pcm16ToUnitFloat32(pcm);
  const bufferRate = playbackBufferSampleRate(ctxRate);
  if (pcmRatesMatch(sessionRate, bufferRate)) return floats;
  return resampleLinear(floats, sessionRate, bufferRate);
}

/** Mic block at the capture rate, as samples the session PCM rate expects. */
export function float32ForSessionRate(
  input: Float32Array,
  captureRate: number,
  sessionRate: number,
): Float32Array {
  if (pcmRatesMatch(captureRate, sessionRate)) return input;
  if (captureRate > sessionRate) return downsampleFloat32(input, captureRate, sessionRate);
  return resampleLinear(input, captureRate, sessionRate);
}

/** xAI session.update audio formats. Input and output share one negotiated rate. */
export function realtimeSessionAudioFormats(pcmRate: number): {
  input: { format: { type: 'audio/pcm'; rate: XaiPcmRate } };
  output: { format: { type: 'audio/pcm'; rate: XaiPcmRate } };
} {
  const rate = nearestXaiPcmRate(pcmRate);
  return {
    input: { format: { type: 'audio/pcm', rate } },
    output: { format: { type: 'audio/pcm', rate } },
  };
}
