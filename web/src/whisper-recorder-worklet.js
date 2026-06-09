// Whisper 録音用 AudioWorklet プロセッサ。
//
// 以前は voice-whisper.ts 内でソースを blob URL 化して addModule() していたが、
// Hub の CSP（script-src 'self'）が blob: スクリプトを許可しておらず、AudioWorklet
// モジュールのロードがブロックされて録音が「マイクから音が取得できません」で失敗していた。
// CSP を緩めず blob: を許可しないため、同一オリジンの静的 JS として配信する。
// このファイルは esbuild で dist/ に出力され /whisper-recorder-worklet.js から配信される。
class AnyAiCliWhisperRecorder extends AudioWorkletProcessor {
  process(inputs) {
    const input = inputs[0];
    if (!input || input.length === 0 || input[0].length === 0) return true;
    const frames = input[0].length;
    const channels = input.length;
    const mono = new Float32Array(frames);
    for (let i = 0; i < frames; i++) {
      let sum = 0;
      for (let ch = 0; ch < channels; ch++) sum += input[ch][i] || 0;
      mono[i] = sum / channels;
    }
    this.port.postMessage(mono, [mono.buffer]);
    return true;
  }
}
registerProcessor('any-ai-cli-whisper-recorder', AnyAiCliWhisperRecorder);
