import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

const modulePath = process.env.VNEXT_VOICE_NOTE_MODULE
  ?? new URL('../node_modules/.cache/aegis-runtime-vnext-voice-note-test/voiceNote.mjs', import.meta.url).href;

const {
  VOICE_NOTE_MAX_MS,
  VOICE_NOTE_MIME_CANDIDATES,
  IDLE_VOICE_NOTE,
  VOICE_MIC_IDLE_RELEASE_MS,
  createVoiceMicSession,
  formatVoiceDuration,
  formatVoiceElapsed,
  pickVoiceNoteMimeType,
  shouldAutoStopVoiceNote,
  startVoiceNoteRecording,
  voiceNoteAttachment,
  voiceNoteFileName,
  voiceNoteOccupiesComposer,
  voiceNoteReducer,
  voiceNoteTranscriptPreview,
} = await import(modulePath);

describe('voice note: container selection', () => {
  it('prefers Opus in WebM when the browser supports it', () => {
    const supported = new Set(['audio/webm;codecs=opus', 'audio/webm', 'audio/mp4']);
    assert.equal(pickVoiceNoteMimeType((mime) => supported.has(mime)), 'audio/webm;codecs=opus');
  });

  it('falls back to audio/mp4 on iOS Safari, which supports nothing else', () => {
    // Safari 16.4+ MediaRecorder: AAC in an MP4 container is the only option.
    assert.equal(pickVoiceNoteMimeType((mime) => mime === 'audio/mp4'), 'audio/mp4');
  });

  it('returns null when nothing is supported so the browser picks its own default', () => {
    assert.equal(pickVoiceNoteMimeType(() => false), null);
    assert.equal(pickVoiceNoteMimeType(undefined), null);
    // A browser that throws on an unknown type is just saying "no".
    assert.equal(pickVoiceNoteMimeType(() => { throw new Error('nope'); }), null);
  });

  it('names the upload after the container so ffmpeg picks the right demuxer', () => {
    assert.equal(voiceNoteFileName('audio/webm;codecs=opus'), 'voice-note.webm');
    assert.equal(voiceNoteFileName('audio/ogg;codecs=opus'), 'voice-note.ogg');
    assert.equal(voiceNoteFileName('audio/mp4'), 'voice-note.m4a');
    assert.equal(voiceNoteFileName('audio/mpeg'), 'voice-note.mp3');
    assert.equal(voiceNoteFileName('audio/wav'), 'voice-note.wav');
    // Unknown/absent: WebM is the safest guess, and the server sniffs anyway.
    assert.equal(voiceNoteFileName(undefined), 'voice-note.webm');
    assert.equal(voiceNoteFileName(''), 'voice-note.webm');
  });

  it('keeps every candidate an audio container', () => {
    for (const candidate of VOICE_NOTE_MIME_CANDIDATES) {
      assert.match(candidate, /^audio\//, `${candidate} must be an audio container`);
    }
  });
});

describe('voice note: elapsed + duration formatting', () => {
  it('formats M:SS and never renders NaN or a negative clock', () => {
    assert.equal(formatVoiceElapsed(0), '0:00');
    assert.equal(formatVoiceElapsed(7_400), '0:07');
    assert.equal(formatVoiceElapsed(65_000), '1:05');
    assert.equal(formatVoiceElapsed(179_999), '2:59');
    assert.equal(formatVoiceElapsed(-5), '0:00');
    assert.equal(formatVoiceElapsed(Number.NaN), '0:00');
  });

  it('formats a sent note duration and stays empty when unknown', () => {
    assert.equal(formatVoiceDuration(4.25), '0:04');
    assert.equal(formatVoiceDuration(90), '1:30');
    assert.equal(formatVoiceDuration(null), '');
    assert.equal(formatVoiceDuration(undefined), '');
    assert.equal(formatVoiceDuration(Number.NaN), '');
  });

  it('auto-stops at the 3-minute cap', () => {
    assert.equal(VOICE_NOTE_MAX_MS, 180_000);
    assert.equal(shouldAutoStopVoiceNote(179_999), false);
    assert.equal(shouldAutoStopVoiceNote(180_000), true);
    assert.equal(shouldAutoStopVoiceNote(5_000, 4_000), true);
  });

  it('collapses whitespace and ellipsizes a long transcript preview', () => {
    assert.equal(voiceNoteTranscriptPreview('  帮我  看看\n今天的邮件 '), '帮我 看看 今天的邮件');
    const long = 'a'.repeat(200);
    const preview = voiceNoteTranscriptPreview(long, 20);
    assert.equal(preview.length, 20);
    assert.ok(preview.endsWith('…'));
  });
});

describe('voice note: state machine', () => {
  it('walks idle → recording → uploading → attached', () => {
    let state = IDLE_VOICE_NOTE;
    state = voiceNoteReducer(state, { type: 'start' });
    assert.deepEqual(state, { status: 'recording', elapsedMs: 0 });
    state = voiceNoteReducer(state, { type: 'tick', elapsedMs: 3_200 });
    assert.deepEqual(state, { status: 'recording', elapsedMs: 3_200 });
    state = voiceNoteReducer(state, { type: 'stop' });
    assert.deepEqual(state, { status: 'uploading' });
    assert.equal(voiceNoteOccupiesComposer(state), true);
    state = voiceNoteReducer(state, { type: 'uploaded', transcript: '下午三点开会', durationSecs: 3.2 });
    assert.deepEqual(state, { status: 'attached', transcript: '下午三点开会', durationSecs: 3.2 });
    assert.deepEqual(voiceNoteAttachment(state), { transcript: '下午三点开会', durationSecs: 3.2 });
    assert.equal(voiceNoteOccupiesComposer(state), false);
  });

  it('clamps a tick to the hard cap and ignores ticks after the recording ended', () => {
    const recording = voiceNoteReducer(IDLE_VOICE_NOTE, { type: 'start' });
    const clamped = voiceNoteReducer(recording, { type: 'tick', elapsedMs: 999_999 });
    assert.deepEqual(clamped, { status: 'recording', elapsedMs: VOICE_NOTE_MAX_MS });
    const uploading = voiceNoteReducer(recording, { type: 'stop' });
    // A late timer tick must not resurrect the recording UI mid-upload.
    assert.deepEqual(voiceNoteReducer(uploading, { type: 'tick', elapsedMs: 10 }), uploading);
  });

  it('cancel returns to idle from any state, and a cancelled upload cannot land', () => {
    const uploading = voiceNoteReducer(
      voiceNoteReducer(IDLE_VOICE_NOTE, { type: 'start' }),
      { type: 'stop' },
    );
    const cancelled = voiceNoteReducer(uploading, { type: 'cancel' });
    assert.deepEqual(cancelled, { status: 'idle' });
    // The in-flight upload resolving afterwards is a no-op — no zombie note.
    assert.deepEqual(
      voiceNoteReducer(cancelled, { type: 'uploaded', transcript: 'late', durationSecs: 1 }),
      { status: 'idle' },
    );
    assert.equal(voiceNoteAttachment(cancelled), null);
  });

  it('parks on an error the composer can show, and a new recording clears it', () => {
    const recording = voiceNoteReducer(IDLE_VOICE_NOTE, { type: 'start' });
    const failed = voiceNoteReducer(recording, { type: 'failed', code: 'permission' });
    assert.deepEqual(failed, { status: 'error', code: 'permission' });
    assert.deepEqual(
      voiceNoteReducer(failed, { type: 'start' }),
      { status: 'recording', elapsedMs: 0 },
    );
    // `failed` only applies to an active attempt; it never overwrites a held note.
    const attached = voiceNoteReducer(
      voiceNoteReducer(recording, { type: 'stop' }),
      { type: 'uploaded', transcript: 'held', durationSecs: null },
    );
    assert.deepEqual(voiceNoteReducer(attached, { type: 'failed', code: 'upload' }), attached);
  });

  it('refuses to start a new recording while one is still uploading', () => {
    const uploading = voiceNoteReducer(
      voiceNoteReducer(IDLE_VOICE_NOTE, { type: 'start' }),
      { type: 'stop' },
    );
    assert.deepEqual(voiceNoteReducer(uploading, { type: 'start' }), uploading);
  });
});

// ---------------------------------------------------------------------------
// Recorder driver — exercised with fakes; no MediaRecorder, no DOM, no mic.
// ---------------------------------------------------------------------------

function fakeRecorderDeps(overrides = {}) {
  const state = {
    tracksStopped: 0,
    scheduled: null,
    unscheduled: [],
    recorder: null,
    files: [],
    nowMs: 1_000,
  };
  const stream = { getTracks: () => [{ stop: () => { state.tracksStopped += 1; } }] };
  const deps = {
    getUserMedia: async () => stream,
    createRecorder: (_stream, mimeType) => {
      const recorder = {
        mimeType: mimeType ?? 'audio/webm',
        started: false,
        stopped: false,
        ondataavailable: null,
        onstop: null,
        onerror: null,
        start() { this.started = true; },
        stop() {
          this.stopped = true;
          this.ondataavailable?.({ data: 'chunk' });
          this.onstop?.();
        },
      };
      state.recorder = recorder;
      return recorder;
    },
    isTypeSupported: (mime) => mime === 'audio/webm;codecs=opus',
    makeFile: (chunks, filename, mimeType) => {
      const file = { chunks, name: filename, type: mimeType };
      state.files.push(file);
      return file;
    },
    now: () => state.nowMs,
    schedule: (callback, everyMs) => {
      state.scheduled = { callback, everyMs };
      return 42;
    },
    unschedule: (handle) => { state.unscheduled.push(handle); },
    ...overrides,
  };
  return { deps, state };
}

describe('voice note: recorder driver', () => {
  it('records, then hands the assembled audio file to onComplete and releases the mic', async () => {
    const { deps, state } = fakeRecorderDeps();
    const completed = [];
    const handle = await startVoiceNoteRecording(deps, {
      onElapsed: () => {},
      onComplete: (file, elapsedMs) => completed.push({ file, elapsedMs }),
      onError: (code) => assert.fail(`unexpected error ${code}`),
    });
    assert.ok(handle);
    assert.equal(state.recorder.started, true);
    state.nowMs += 2_500;
    handle.stop();
    assert.equal(completed.length, 1);
    assert.equal(completed[0].file.name, 'voice-note.webm');
    assert.equal(completed[0].elapsedMs, 2_500);
    // The mic is released and the timer cleared exactly once.
    assert.equal(state.tracksStopped, 1);
    assert.deepEqual(state.unscheduled, [42]);
  });

  it('reports a denied microphone as a permission error and never starts', async () => {
    const { deps, state } = fakeRecorderDeps({
      getUserMedia: async () => { throw new Error('NotAllowedError'); },
    });
    const errors = [];
    const handle = await startVoiceNoteRecording(deps, {
      onElapsed: () => {},
      onComplete: () => assert.fail('must not complete'),
      onError: (code) => errors.push(code),
    });
    assert.equal(handle, null);
    assert.deepEqual(errors, ['permission']);
    assert.equal(state.recorder, null);
  });

  it('reports a browser without MediaRecorder as unsupported and frees the stream', async () => {
    const { deps, state } = fakeRecorderDeps({
      createRecorder: () => { throw new Error('MediaRecorder is not a constructor'); },
    });
    const errors = [];
    const handle = await startVoiceNoteRecording(deps, {
      onElapsed: () => {},
      onComplete: () => assert.fail('must not complete'),
      onError: (code) => errors.push(code),
    });
    assert.equal(handle, null);
    assert.deepEqual(errors, ['unsupported']);
    assert.equal(state.tracksStopped, 1, 'the mic must be released even when setup fails');
  });

  it('cancel discards the audio: no onComplete, no upload, mic released', async () => {
    const { deps, state } = fakeRecorderDeps();
    let completed = 0;
    const handle = await startVoiceNoteRecording(deps, {
      onElapsed: () => {},
      onComplete: () => { completed += 1; },
      onError: (code) => assert.fail(`unexpected error ${code}`),
    });
    handle.cancel();
    assert.equal(completed, 0, 'a cancelled note must never be uploaded');
    assert.equal(state.tracksStopped, 1);
    // A double-tap on Stop after Cancel is inert.
    handle.stop();
    assert.equal(completed, 0);
  });

  it('a double-tapped Stop uploads exactly once', async () => {
    const { deps } = fakeRecorderDeps();
    let completed = 0;
    const handle = await startVoiceNoteRecording(deps, {
      onElapsed: () => {},
      onComplete: () => { completed += 1; },
      onError: (code) => assert.fail(`unexpected error ${code}`),
    });
    handle.stop();
    handle.stop();
    assert.equal(completed, 1);
  });

  it('ticks the elapsed timer and auto-stops at the cap', async () => {
    const { deps, state } = fakeRecorderDeps();
    const elapsed = [];
    let completed = 0;
    const handle = await startVoiceNoteRecording(
      { ...deps, maxMs: 5_000 },
      {
        onElapsed: (ms) => elapsed.push(ms),
        onComplete: () => { completed += 1; },
        onError: (code) => assert.fail(`unexpected error ${code}`),
      },
    );
    assert.ok(handle);
    state.nowMs += 1_000;
    state.scheduled.callback();
    assert.deepEqual(elapsed, [1_000]);
    assert.equal(completed, 0);
    // Past the cap the recorder stops itself — the owner never has to.
    state.nowMs += 4_500;
    state.scheduled.callback();
    assert.equal(completed, 1);
  });

  it('treats an empty recording as a record error rather than a blank upload', async () => {
    const { deps, state } = fakeRecorderDeps({
      createRecorder: () => {
        const recorder = {
          mimeType: 'audio/webm',
          ondataavailable: null,
          onstop: null,
          onerror: null,
          start() {},
          stop() { this.onstop?.(); }, // no data event at all
        };
        state.recorder = recorder;
        return recorder;
      },
    });
    const errors = [];
    const handle = await startVoiceNoteRecording(deps, {
      onElapsed: () => {},
      onComplete: () => assert.fail('must not upload an empty recording'),
      onError: (code) => errors.push(code),
    });
    handle.stop();
    assert.deepEqual(errors, ['record']);
  });
});

// ---------------------------------------------------------------------------
// Microphone session (owner 2026-09-13: "每次都问权限")
// ---------------------------------------------------------------------------

function fakeMicSession(overrides = {}) {
  const state = { grants: 0, timers: [], cleared: [], streams: [] };
  const makeStream = () => {
    const track = { readyState: 'live', stop() { this.readyState = 'ended'; } };
    const stream = { tracks: [track], getTracks: () => [track] };
    state.streams.push(stream);
    return stream;
  };
  const session = createVoiceMicSession({
    getUserMedia: async () => { state.grants += 1; return makeStream(); },
    setTimer: (callback, afterMs) => { state.timers.push({ callback, afterMs }); return state.timers.length; },
    clearTimer: (handle) => { state.cleared.push(handle); },
    ...overrides,
  });
  return { session, state };
}

describe('voice note: microphone session', () => {
  it('reuses the granted stream for the next note instead of asking again', async () => {
    const { session, state } = fakeMicSession();
    const first = await session.acquire();
    session.release(first);
    const second = await session.acquire();
    assert.equal(second, first);
    assert.equal(state.grants, 1, 'one permission prompt for two notes');
    assert.equal(first.tracks[0].readyState, 'live', 'the mic stays open between notes');
    // Re-acquiring cancelled the idle release.
    assert.deepEqual(state.cleared, [1]);
  });

  it('stops the mic after the idle window and grants afresh afterwards', async () => {
    const { session, state } = fakeMicSession();
    const first = await session.acquire();
    session.release(first);
    assert.equal(state.timers.length, 1);
    assert.equal(state.timers[0].afterMs, VOICE_MIC_IDLE_RELEASE_MS);
    state.timers[0].callback();
    assert.equal(first.tracks[0].readyState, 'ended', 'idle release stops the tracks');
    assert.equal(session.isHeld(), false);
    const second = await session.acquire();
    assert.notEqual(second, first);
    assert.equal(state.grants, 2);
  });

  it('drops a stream whose tracks the OS ended and asks again', async () => {
    const { session, state } = fakeMicSession();
    const first = await session.acquire();
    session.release(first);
    first.tracks[0].readyState = 'ended';
    const second = await session.acquire();
    assert.notEqual(second, first);
    assert.equal(state.grants, 2);
  });

  it('dispose stops the held stream immediately (page hidden / unmount)', async () => {
    const { session, state } = fakeMicSession();
    const stream = await session.acquire();
    session.release(stream);
    session.dispose();
    assert.equal(stream.tracks[0].readyState, 'ended');
    assert.equal(session.isHeld(), false);
    assert.deepEqual(state.cleared, [1], 'the pending idle timer is cancelled');
  });

  it('a foreign stream handed to release is stopped, the held one untouched', async () => {
    const { session } = fakeMicSession();
    const held = await session.acquire();
    const foreign = { tracks: [{ readyState: 'live', stop() { this.readyState = 'ended'; } }], getTracks() { return this.tracks; } };
    session.release(foreign);
    assert.equal(foreign.tracks[0].readyState, 'ended');
    assert.equal(held.tracks[0].readyState, 'live');
    assert.equal(session.isHeld(), true);
  });

  it('the recorder driver hands the stream back through releaseStream instead of stopping it', async () => {
    const released = [];
    const { deps, state } = fakeRecorderDeps({ releaseStream: (stream) => { released.push(stream); } });
    const completed = [];
    const handle = await startVoiceNoteRecording(deps, {
      onElapsed: () => {},
      onComplete: (file) => { completed.push(file); },
      onError: (code) => { throw new Error(`unexpected error ${code}`); },
    });
    handle.stop();
    assert.equal(completed.length, 1);
    assert.equal(released.length, 1, 'stream handed back once');
    assert.equal(state.tracksStopped, 0, 'tracks are NOT stopped when a session owns the stream');
  });
});
