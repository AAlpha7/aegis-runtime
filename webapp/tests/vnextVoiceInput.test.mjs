import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

const modulePath = process.env.VNEXT_VOICE_INPUT_MODULE;
assert.ok(modulePath, 'VNEXT_VOICE_INPUT_MODULE is required');

const {
  appendTranscript,
  browserSpeechRecognitionConstructor,
  startBrowserVoiceInput,
  transcriptFromSpeechEvent,
} = await import(modulePath);

function speechEvent(...transcripts) {
  const results = transcripts.map((transcript) => ({
    length: 1,
    0: { transcript },
  }));
  results.length = transcripts.length;
  return { results };
}

describe('vNext voice input', () => {
  it('extracts and appends transcripts without auto-sending free text', () => {
    assert.equal(transcriptFromSpeechEvent(speechEvent(' commute ', '', 'tasks')), 'commute tasks');
    assert.equal(appendTranscript('', 'commute'), 'commute');
    assert.equal(appendTranscript('please check ', 'commute'), 'please check commute');
  });

  it('uses standard or webkit speech recognition constructors', () => {
    class StandardRecognition {}
    class WebkitRecognition {}
    assert.equal(browserSpeechRecognitionConstructor({ SpeechRecognition: StandardRecognition }), StandardRecognition);
    assert.equal(browserSpeechRecognitionConstructor({ webkitSpeechRecognition: WebkitRecognition }), WebkitRecognition);
    assert.equal(browserSpeechRecognitionConstructor({}), null);
  });

  it('captures voice into the current text field and never submits free text by itself', () => {
    const notices = [];
    let value = 'please check';
    let submitCalls = 0;
    let transcriptCapturedCalls = 0;
    let instance = null;
    class FakeRecognition {
      constructor() {
        this.lang = '';
        this.interimResults = true;
        this.maxAlternatives = 99;
        this.onstart = null;
        this.onerror = null;
        this.onresult = null;
        this.onend = null;
        instance = this;
      }
      start() {
        this.onstart?.();
      }
    }

    const recognition = startBrowserVoiceInput({
      SpeechRecognition: FakeRecognition,
      language: 'zh-CN',
      locale: 'en',
      setValue: (updater) => {
        value = updater(value);
      },
      setNotice: (notice) => notices.push(notice),
      onTranscriptCaptured: () => {
        transcriptCapturedCalls += 1;
      },
      onAutoSend: () => {
        submitCalls += 1;
      },
    });

    assert.equal(recognition, instance);
    assert.equal(instance.lang, 'zh-CN');
    assert.equal(instance.interimResults, false);
    assert.equal(instance.maxAlternatives, 1);
    assert.equal(notices[0], 'Listening…');

    // Free multi-word English is not a CEO shorthand key → no auto-send.
    instance.onresult?.(speechEvent('please review the draft carefully'));
    assert.equal(value, 'please check please review the draft carefully');
    assert.equal(transcriptCapturedCalls, 1);
    assert.equal(submitCalls, 0);
    assert.equal(notices.at(-1), 'Voice captured. Review or send.');
  });

  it('auto-sends known CEO shorthand transcripts when onAutoSend is provided', async () => {
    const notices = [];
    let autoSendText = '';
    let instance = null;
    class FakeRecognition {
      constructor() {
        this.lang = '';
        this.interimResults = false;
        this.maxAlternatives = 1;
        this.onstart = null;
        this.onerror = null;
        this.onresult = null;
        this.onend = null;
        instance = this;
      }
      start() {
        this.onstart?.();
      }
    }

    startBrowserVoiceInput({
      SpeechRecognition: FakeRecognition,
      language: 'zh-CN',
      locale: 'zh',
      setValue: () => {},
      setNotice: (notice) => notices.push(notice),
      onAutoSend: (text) => {
        autoSendText = text;
      },
    });

    instance.onresult?.(speechEvent('通勤'));
    await Promise.resolve();
    await Promise.resolve();
    assert.equal(autoSendText, '通勤');
    assert.equal(notices.at(-1), '已识别并发送。');
  });

  it('fails closed when speech recognition is unavailable or interrupted', () => {
    const notices = [];
    const noRecognition = startBrowserVoiceInput({
      SpeechRecognition: null,
      language: 'en-US',
      locale: 'en',
      setValue: () => {
        throw new Error('setValue must not run without recognition');
      },
      setNotice: (notice) => notices.push(notice),
    });
    assert.equal(noRecognition, null);
    assert.match(notices.at(-1), /not available/i);

    class ErrorRecognition {
      constructor() {
        this.lang = '';
        this.interimResults = false;
        this.maxAlternatives = 1;
        this.onstart = null;
        this.onerror = null;
        this.onresult = null;
        this.onend = null;
      }
      start() {
        throw new Error('microphone denied');
      }
    }
    const startFailure = startBrowserVoiceInput({
      SpeechRecognition: ErrorRecognition,
      language: 'en-US',
      locale: 'en',
      setValue: () => {
        throw new Error('setValue must not run on start failure');
      },
      setNotice: (notice) => notices.push(notice),
    });
    assert.equal(startFailure, null);
    assert.match(notices.at(-1), /could not start/i);
  });

  it('reports end-without-text distinctly from explicit speech errors', () => {
    const notices = [];
    let instance = null;
    class FakeRecognition {
      constructor() {
        this.lang = '';
        this.interimResults = false;
        this.maxAlternatives = 1;
        this.onstart = null;
        this.onerror = null;
        this.onresult = null;
        this.onend = null;
        instance = this;
      }
      start() {}
    }

    startBrowserVoiceInput({
      SpeechRecognition: FakeRecognition,
      language: 'en-US',
      locale: 'en',
      setValue: () => {
        throw new Error('setValue must not run without transcript');
      },
      setNotice: (notice) => notices.push(notice),
    });
    instance.onend?.();
    assert.match(notices.at(-1), /without text|没有听清/i);
  });

  it('uses Chinese notices when locale is zh', () => {
    const notices = [];
    let instance = null;
    class FakeRecognition {
      constructor() {
        this.lang = '';
        this.interimResults = false;
        this.maxAlternatives = 1;
        this.onstart = null;
        this.onerror = null;
        this.onresult = null;
        this.onend = null;
        instance = this;
      }
      start() {
        this.onstart?.();
      }
    }
    startBrowserVoiceInput({
      SpeechRecognition: FakeRecognition,
      language: 'zh-CN',
      locale: 'zh',
      setValue: () => {},
      setNotice: (notice) => notices.push(notice),
    });
    assert.equal(notices[0], '在听…');
    instance.onresult?.(speechEvent('随便说一句比较长的话用来测试自由文本'));
    assert.equal(notices.at(-1), '已识别，可检查后发送。');
  });
});
