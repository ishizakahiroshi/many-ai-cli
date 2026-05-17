# [完了] 障害対応記録: 音声入力コードが複雑化しすぎて不安定になっていた

> 最終更新: 2026-05-18(月) 01:09:39

## 症状

音声入力が断続的に失敗する。具体的には以下が重なっていた。

- watchdog（no-result）が `audioend` 後 8000ms で誤発火し、STT サーバー応答を待ちきれずに abort → 「Could not recognize speech」トーストが出てテキストが入らない
- `auto-restart`（grace 秒）、`beginRetryTimer`、`forceCleanup` タイマー、`scheduleForceCleanup` タイマーが同時に走り、どのタイマーが勝つかで動作が変わる
- デバッグ用の `[VOICE-DBG]` ログと `inputEl.value` setter watcher が本番コードに残っており、根本原因の特定が難しい
- 状態変数が 10 個以上（`isRecording`, `isStarting`, `voiceIntent`, `userIntendedStop`, `restartTimer`, `isAutoRestarting`, `lastResultAt`, `silenceStopTimer`, `noResultWatchdog`, `forceCleanupTimer`, `beginRetryTimer`, ...）絡み合い、フラグの取りこぼしが随所で発生していた

## 根本原因（root cause）

`web/src/app.js` 音声入力 IIFE（書き直し前 7461〜8092 行）。

段階的なパッチ適用（第1次〜第3次修正、`recreate-1`〜`recreate-7`）で機能追加と workaround が積み重なり、コードが自己矛盾した状態になっていた。

- `forceCleanup` が複数の出口（watchdog 発火・abort エラー・btn.click 二度押し・タイムアウト）から呼ばれ、`onend` が来た後に二重呼び出しが起きることがあった
- `_recreateRecognition` がさまざまな場所から呼ばれ「どのインスタンスのイベントか」の判定（`_isCurrentRecognitionEvent`）が複雑化
- `auto-restart`（grace 秒）は設定 UI まで持っていたが、`DEFAULT_VOICE_GRACE_SEC = 0` のため事実上は無効で、コードだけが残っていた
- ウェイクワード機能は無効化済みにもかかわらず、それを前提とした `voiceIntent` フラグと `window._voiceIntentActive` が残っていた

## 修正内容

音声入力 IIFE を最小構成に全面書き直し。

**削除した機能・コード（約 320 行）**

| 削除したもの | 理由 |
|---|---|
| `[VOICE-DBG]` ログ全部 + `inputEl.value` setter watcher | デバッグ用コード |
| auto-restart（`restartTimer`, `isAutoRestarting`, `lastResultAt`, `silenceStopTimer`, `getVoiceGraceSec`） | 設定デフォルト 0 で実質無効。コードだけが残っていた |
| watchdog（`armNoResultWatchdog`, `clearNoResultWatchdog`, 2 種のタイムアウト定数） | STT サーバー応答を待てず誤発火していた。シンプルな `onerror`→`stopVoice` で代替 |
| `forceCleanup` / `scheduleForceCleanup` / `cancelForceCleanup` + `forceCleanupTimer` | `stopVoice()` 1 本に統合 |
| `beginRetryTimer` / `clearBeginRetryTimer` + retry ロジック | `start()` 失敗時は `onerror`→`stopVoice` で終了させる |
| `isStarting` | retry 廃止により不要 |
| `voiceIntent` + `window._voiceIntentActive` | ウェイクワード無効化済み。参照元なし |
| `_recognitionListeners` / `_onRecognition` / `_isCurrentRecognitionEvent` | インスタンスを毎回作り直すので複数インスタンス管理は不要 |

**新しい構造**

```js
// 状態変数: 3個のみ
let isRecording  = false;
let interimStart = 0;
let preVoiceText = '';

// 終了出口: stopVoice() 1 本
function stopVoice() {
  if (!isRecording) return;        // 二重呼び出しガード
  // ... UI リセット ...
  // 次回のためにインスタンス作り直し（Chrome stuck 対策）
  recognition = new SpeechRecognition();
  configureRecognition();
  attachHandlers();
}

function attachHandlers() {
  recognition.onstart  = () => { /* 録音開始 UI */ };
  recognition.onresult = (e) => { /* テキスト反映 */ };
  recognition.onend    = () => stopVoice();
  recognition.onerror  = (e) => { showVoiceError(e, btn); stopVoice(); };
  // ... 波形用: onsoundstart / onspeechstart / onspeechend / onaudioend ...
}
```

- `onend` / `onerror` どちらが先に来ても `stopVoice()` → `isRecording` ガードで二重実行を防ぐ
- 終了のたびに `new SpeechRecognition()` → Chrome の stuck を次回に持ち越さない

## 変更ファイル

| ファイル | 内容 |
|---|---|
| `web/src/app.js` | 音声入力 IIFE（7461〜8092 行）を最小構成に全面書き直し（約 320 行削減） |

## 検証

- <TODO: ビルド後、音声入力ボタンを押して発話 → テキストエリアに文字が入ることを確認>
- <TODO: 発話中に波形アニメーションが動くことを確認>
- <TODO: ✕（キャンセル）で preVoiceText に戻ることを確認>
- <TODO: ✓（確定）または Esc でバーが閉じることを確認>
- <TODO: 2 回目以降の音声入力が正常に動作することを確認（Chrome stuck が出ないこと）>
- <TODO: `no-speech` エラー（無音タイムアウト）でトーストが出ないことを確認>

## 備忘

- 以前の修正履歴（第1次〜第3次）は `docs/bugfix_voice-audioend-watchdog-cleared_2026-05-17.md` を参照。今回の書き直しでそれらの workaround は全て撤廃された
- `continuous=false`（現在の設定）では Chrome が発話終了後に一括で STT リクエストを送るため、`audioend` から `result` まで数秒かかることがある。workaround なしで待てる理由は「`onend` が来れば `stopVoice()` で終わる」だけでよく、STT の遅延に対して特別な処理が不要なため
- Grace 秒設定 UI（`voice-grace-select`）はコードから削除したが、HTML/CSS 側の要素は残っている。次回 UI 整理の際に合わせて削除する
