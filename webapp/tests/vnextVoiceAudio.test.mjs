import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

const audioPath = process.env.VNEXT_VOICE_AUDIO_MODULE;
const realtimePath = process.env.VNEXT_VOICE_REALTIME_MODULE;
assert.ok(audioPath, 'VNEXT_VOICE_AUDIO_MODULE is required');
assert.ok(realtimePath, 'VNEXT_VOICE_REALTIME_MODULE is required');

const {
  captureQuantumSamples,
  pcm16ToPlaybackFloat32,
  playbackBufferSampleRate,
  sessionFrameSamples,
  sessionPcmRateForContext,
} = await import(audioPath);
const { buildSessionUpdatePayload } = await import(realtimePath);

describe('duplex PCM rate follows the live AudioContext', () => {
  it('negotiates 48 kHz on a 48 kHz phone context', () => {
    assert.equal(sessionPcmRateForContext(48_000), 48_000);
    assert.equal(playbackBufferSampleRate(48_000), 48_000);
    const payload = buildSessionUpdatePayload({
      instructions: 'speak',
      tools: [],
      pcmRate: sessionPcmRateForContext(48_000),
    });
    assert.equal(payload.session.audio.input.format.rate, 48_000);
    assert.equal(payload.session.audio.output.format.rate, 48_000);
    assert.equal(payload.session.audio.input.format.type, 'audio/pcm');
  });

  it('negotiates 44.1 kHz when that is the hardware rate', () => {
    assert.equal(sessionPcmRateForContext(44_100), 44_100);
    const payload = buildSessionUpdatePayload({
      instructions: 'speak',
      tools: [],
      pcmRate: 44_100,
    });
    assert.equal(payload.session.audio.input.format.rate, 44_100);
    assert.equal(payload.session.audio.output.format.rate, 44_100);
  });

  it('keeps 24 kHz when the context actually runs at 24 kHz', () => {
    assert.equal(sessionPcmRateForContext(24_000), 24_000);
    assert.equal(playbackBufferSampleRate(24_000), 24_000);
  });

  it('sizes the capture quantum for the hardware rate', () => {
    assert.equal(captureQuantumSamples(48_000), 4096);
    assert.equal(captureQuantumSamples(44_100), 4096);
    assert.equal(captureQuantumSamples(24_000), 2048);
    assert.equal(sessionFrameSamples(4096, 48_000, 48_000), 4096);
  });
});

describe('duplex playback level and duration', () => {
  it('does not stretch or clip 48 kHz PCM played in a 48 kHz context', () => {
    const pcm = new Int16Array([32767, -32768, 0, 16000]);
    const played = pcm16ToPlaybackFloat32(pcm, 48_000, 48_000);
    assert.equal(played.length, pcm.length);
    assert.equal(playbackBufferSampleRate(48_000), 48_000);
    assert.ok(played[0] < 1);
    assert.equal(played[0], 32767 / 32768);
    assert.equal(played[1], -1);
    assert.equal(played[2], 0);
    let peak = 0;
    for (const sample of played) peak = Math.max(peak, Math.abs(sample));
    assert.ok(peak <= 1);
  });

  it('preserves duration when 24 kHz PCM must be played in a 48 kHz context', () => {
    const sessionRate = 24_000;
    const ctxRate = 48_000;
    const pcm = new Int16Array(sessionRate / 10);
    for (let i = 0; i < pcm.length; i += 1) {
      pcm[i] = Math.round(Math.sin((2 * Math.PI * 440 * i) / sessionRate) * 16000);
    }
    const played = pcm16ToPlaybackFloat32(pcm, sessionRate, ctxRate);
    const durationIn = pcm.length / sessionRate;
    const durationOut = played.length / playbackBufferSampleRate(ctxRate);
    assert.ok(Math.abs(durationIn - durationOut) < 0.002);
    let peak = 0;
    for (const sample of played) peak = Math.max(peak, Math.abs(sample));
    assert.ok(peak <= 16000 / 32768 + 1e-4);
    assert.ok(peak <= 1);
  });
});
