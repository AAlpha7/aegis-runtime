/**
 * Pure duplex health classification — unit-testable without mic/WS.
 * Used by the orb to show WHY voice is stuck instead of "always listening".
 */

export type VoiceDuplexDiag = {
  wsOpen: boolean;
  sessionUpdated: boolean;
  /** PCM frames sent via input_audio_buffer.append */
  appendFrames: number;
  speechStarts: number;
  speechStops: number;
  /** response.output_audio.delta count */
  audioDeltas: number;
  /** Local scheduled playback chunks */
  playbackChunks: number;
  /** Peak abs sample seen on mic (0..1), last windows */
  micPeak: number;
  ctxSampleRate: number;
  sessionPcmRate: number;
  liveSeconds: number;
  lastServerError?: string;
};

export type VoiceHealth = {
  ok: boolean;
  code:
    | 'ok'
    | 'connecting'
    | 'no_ws'
    | 'no_session'
    | 'no_uplink'
    | 'mic_quiet'
    | 'no_vad'
    | 'no_reply_audio'
    | 'no_playback'
    | 'server_error';
  /** Owner-facing zh */
  messageZh: string;
  /** Owner-facing en */
  messageEn: string;
};

/** Classify after the session has been live long enough for signals to appear. */
export function classifyDuplexHealth(d: VoiceDuplexDiag): VoiceHealth {
  if (d.lastServerError) {
    return {
      ok: false,
      code: 'server_error',
      messageZh: `服务端：${d.lastServerError.slice(0, 80)}`,
      messageEn: `Server: ${d.lastServerError.slice(0, 80)}`,
    };
  }
  if (!d.wsOpen) {
    return {
      ok: false,
      code: 'no_ws',
      messageZh: '语音通道未连接',
      messageEn: 'Voice channel not connected',
    };
  }
  // Warm-up: don't alarm in the first few seconds.
  if (d.liveSeconds < 4) {
    return {
      ok: true,
      code: 'connecting',
      messageZh: '正在接通…',
      messageEn: 'Connecting…',
    };
  }
  if (!d.sessionUpdated) {
    return {
      ok: false,
      code: 'no_session',
      messageZh: '会话未就绪（session.update）',
      messageEn: 'Session not ready (session.update)',
    };
  }
  if (d.appendFrames < 15) {
    return {
      ok: false,
      code: 'no_uplink',
      messageZh: '麦克风上行无数据',
      messageEn: 'No microphone uplink frames',
    };
  }
  // Plenty of frames but essentially silence → wrong device / muted OS mic.
  if (d.liveSeconds >= 6 && d.micPeak < 0.008 && d.speechStarts === 0) {
    return {
      ok: false,
      code: 'mic_quiet',
      messageZh: '麦克风几乎无声（检查系统权限/设备）',
      messageEn: 'Mic nearly silent (check OS permission/device)',
    };
  }
  // Uplink OK but VAD never fires after owner should have spoken.
  if (d.liveSeconds >= 10 && d.speechStarts === 0 && d.micPeak >= 0.02) {
    return {
      ok: false,
      code: 'no_vad',
      messageZh: '有声音但服务端未检测到说话（采样/格式）',
      messageEn: 'Audio present but server VAD never fired (rate/format)',
    };
  }
  // User was detected; model never returned audio.
  if (d.speechStops >= 1 && d.liveSeconds >= 8 && d.audioDeltas === 0) {
    return {
      ok: false,
      code: 'no_reply_audio',
      messageZh: '已听到你，但没有回传语音',
      messageEn: 'Heard you, but no reply audio returned',
    };
  }
  // Model sent audio; we failed to schedule playback.
  if (d.audioDeltas >= 3 && d.playbackChunks === 0) {
    return {
      ok: false,
      code: 'no_playback',
      messageZh: '收到语音但本地无法播放',
      messageEn: 'Got audio deltas but local playback failed',
    };
  }
  return {
    ok: true,
    code: 'ok',
    messageZh: '双工正常',
    messageEn: 'Duplex healthy',
  };
}

export function emptyVoiceDuplexDiag(partial?: Partial<VoiceDuplexDiag>): VoiceDuplexDiag {
  return {
    wsOpen: false,
    sessionUpdated: false,
    appendFrames: 0,
    speechStarts: 0,
    speechStops: 0,
    audioDeltas: 0,
    playbackChunks: 0,
    micPeak: 0,
    ctxSampleRate: 0,
    sessionPcmRate: 24000,
    liveSeconds: 0,
    ...partial,
  };
}
