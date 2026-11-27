import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

const modulePath = process.env.VNEXT_VOICE_DIAG_MODULE;
assert.ok(modulePath, 'VNEXT_VOICE_DIAG_MODULE is required');

const { classifyDuplexHealth, emptyVoiceDuplexDiag } = await import(modulePath);

describe('voice duplex health classifier', () => {
  it('is quiet during warm-up', () => {
    const h = classifyDuplexHealth(emptyVoiceDuplexDiag({
      wsOpen: true,
      sessionUpdated: true,
      appendFrames: 2,
      liveSeconds: 2,
    }));
    assert.equal(h.code, 'connecting');
    assert.equal(h.ok, true);
  });

  it('detects missing uplink', () => {
    const h = classifyDuplexHealth(emptyVoiceDuplexDiag({
      wsOpen: true,
      sessionUpdated: true,
      appendFrames: 3,
      liveSeconds: 8,
    }));
    assert.equal(h.code, 'no_uplink');
    assert.equal(h.ok, false);
  });

  it('detects quiet mic', () => {
    const h = classifyDuplexHealth(emptyVoiceDuplexDiag({
      wsOpen: true,
      sessionUpdated: true,
      appendFrames: 200,
      micPeak: 0.001,
      speechStarts: 0,
      liveSeconds: 8,
    }));
    assert.equal(h.code, 'mic_quiet');
  });

  it('detects VAD miss when mic has energy', () => {
    const h = classifyDuplexHealth(emptyVoiceDuplexDiag({
      wsOpen: true,
      sessionUpdated: true,
      appendFrames: 200,
      micPeak: 0.05,
      speechStarts: 0,
      liveSeconds: 12,
    }));
    assert.equal(h.code, 'no_vad');
  });

  it('detects no reply audio after speech', () => {
    const h = classifyDuplexHealth(emptyVoiceDuplexDiag({
      wsOpen: true,
      sessionUpdated: true,
      appendFrames: 200,
      speechStarts: 1,
      speechStops: 1,
      audioDeltas: 0,
      liveSeconds: 10,
    }));
    assert.equal(h.code, 'no_reply_audio');
  });

  it('detects playback failure', () => {
    const h = classifyDuplexHealth(emptyVoiceDuplexDiag({
      wsOpen: true,
      sessionUpdated: true,
      appendFrames: 200,
      speechStarts: 1,
      speechStops: 1,
      audioDeltas: 10,
      playbackChunks: 0,
      liveSeconds: 10,
    }));
    assert.equal(h.code, 'no_playback');
  });

  it('reports ok when duplex signals present', () => {
    const h = classifyDuplexHealth(emptyVoiceDuplexDiag({
      wsOpen: true,
      sessionUpdated: true,
      appendFrames: 200,
      speechStarts: 1,
      speechStops: 1,
      audioDeltas: 20,
      playbackChunks: 15,
      micPeak: 0.1,
      liveSeconds: 12,
    }));
    assert.equal(h.code, 'ok');
    assert.equal(h.ok, true);
  });
});
