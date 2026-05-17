# [対応中] 障害対応記録: 音声入力が発話を認識しても SR:result が一度も来ない

> 最終更新: 2026-05-18(月) 00:55:15

## 症状

音声入力ボタンを押して発話すると、`SR:audiostart` / `SR:soundstart` / `SR:speechstart` は
届いて波形アニメーションも動くのに、テキストエリアに文字が一切入らない。

- `SR:result` イベントがログに一度も現れない（`interimResults=true` でも同様）
- ユーザーが ✕ を押して止めるまで何も起きない
- `SR:end` 時の `lastResultAt= 0` が証拠（result が届いていない）

**具体的なログパターン（第1次 / 2026-05-17）:**
```
SR:audiostart / SR:soundstart / SR:speechstart — すべて isCurrent=true で正常
SR:speechend / SR:soundend / SR:audioend — すべて isCurrent=true で正常
（この後 SR:result が来るはずが来ない）
inputEl.value SET "" ← at cancelBtn (app.js:7891)
forceCleanup → SR:end lastResultAt= 0
```

**再発ログパターン（2026-05-18）:**
```
_recreateRecognition 開始 / oldRecognition.abort() 完了
beginVoiceRecognition retryCount= 0
SR:start / SR:audiostart / SR:soundstart / SR:speechstart — isCurrent=true で正常
SR:speechend / SR:soundend → armNoResultWatchdog 8000ms
SR:audioend → voice-processing 開始 lastResultAt= 0
（8000ms 後）watchdog 発火! → _recreateRecognition → forceCleanup
→ トースト「Could not recognize speech (microphone may be contested)」
SR:error isCurrent= false error= aborted
SR:end isCurrent= false
```

- Google 音声検索（`google.co.jp`）では「音声テスト 音声テスト」が正常認識できることを確認 → Chrome Web Speech API 自体は正常
- → any-ai-cli コード固有の問題と確定

## 根本原因（root cause）

### 第1次原因（2026-05-17）

`web/src/app.js` の `audioend` ハンドラ（修正前 7968 行）で
`clearNoResultWatchdog()` を呼んでいた。

```
soundend ハンドラ: armNoResultWatchdog(8000ms)  ← サーバー応答待ち用の猶予を設定
    ↓ 直後に
audioend ハンドラ: clearNoResultWatchdog()        ← ★ここで 8000ms watchdog を消してしまう
```

Chrome は `audioend` 後もサーバー（Google STT）へ音声データを送信して応答を待つ。
この「サーバー応答待ち期間」が watchdog なしで保護されていなかった。

また UI 側にも「処理中」を示すフィードバックがなかったため、ユーザーは
"波形が止まった → 失敗した" と判断して ✕（cancelBtn）を押してしまい、
`recognition.abort()` が走って処理中の結果が捨てられていた。

### 第2次原因（2026-05-18 — 再発調査）

hw（ウェイクワード機能）の無効化に伴い、`stopPromise` が常に `null` になったことで、
`abort()` 直後に `start()` が 0ms ディレイで呼ばれるようになった。

```
// 以前（hw 有効時）
_recreateRecognition() → oldRecognition.abort()
stopPromise.then(() => beginVoiceRecognition())  // ← hw マイク解放後に start（数十〜数百ms 遅延）

// hw 無効化後（問題）
_recreateRecognition() → oldRecognition.abort()
beginVoiceRecognition()  // ← 0ms で即 start → Chrome のマイク解放前に競合
```

Chrome が前の `abort()` でマイクストリームを解放しきる前に `start()` が呼ばれると、
「波形は出るが `SR:result` が届かない」stuck 状態に入る。
hw 有効時は `stopPromise.then(...)` の非同期呼び出しが自然な遅延として機能していた。

## 修正内容

### 第1次修正（2026-05-17）

**① `audioend` ハンドラから `clearNoResultWatchdog()` を削除し、処理中クラスを付与**

before:
```js
_onRecognition('audioend', (e) => {
    if (!_isCurrentRecognitionEvent(e)) return;
    setVoiceAudioActive(false);
    voiceIntensityTarget = 0.03;
    clearNoResultWatchdog();  // ← watchdog を消していた
});
```

after:
```js
_onRecognition('audioend', (e) => {
    if (!_isCurrentRecognitionEvent(e)) return;
    setVoiceAudioActive(false);
    voiceIntensityTarget = 0.03;
    // clearNoResultWatchdog() を削除: soundend で張った 8000ms watchdog を継続
    console.log('[VOICE-DBG] SR:audioend → voice-processing 開始 lastResultAt=', lastResultAt);
    voiceBar.classList.add('voice-processing');
});
```

**② `result` / `end` / `forceCleanup` でクラスを除去**

```js
// result ハンドラ先頭
voiceBar.classList.remove('voice-processing');

// end ハンドラ（cancelForceCleanup 直後）
voiceBar.classList.remove('voice-processing');

// forceCleanup（hideVoiceBar の直前）
voiceBar.classList.remove('voice-processing');
```

**③ CSS: `voice-processing` 状態の視覚フィードバック**

```css
@keyframes voice-processing-pulse {
  0%, 100% { border-color: rgba(96,165,250,0.25); }
  50%       { border-color: rgba(96,165,250,0.75); }
}
#voice-bar.voice-processing {
  animation: voice-processing-pulse 1.4s ease-in-out infinite;
}
#voice-bar.voice-processing .voice-waveform { opacity: 0.3; }
#voice-bar.voice-processing::after {
  content: '認識中...';
  position: absolute;
  left: 10px; right: 80px; top: 50%;
  transform: translateY(-50%);
  text-align: center; font-size: 12px;
  color: var(--fg-muted, #9ca3af);
  pointer-events: none;
}
```

### 第2次修正（2026-05-18）

**③ ボタンクリック時に 150ms ディレイを追加（hw 無効時のマイク競合回避）**

```js
// before
} else {
  beginVoiceRecognition();
}

// after
} else {
  // abort() 直後に start() するとマイク解放前に競合して result が届かなくなる。
  // 150ms 待って Chrome のマイク解放を完了させてから start する。
  setTimeout(() => {
    if (!voiceIntent) return;
    beginVoiceRecognition();
  }, 150);
}
```

**④ `audioend` 後の watchdog を 2000ms に更新（`soundend` からの残り待機を短縮）**（→ 第3次修正で撤回）

```js
// この変更は 2000ms が STT サーバー応答より短く、結果が届く前に watchdog が発火するため
// 第3次修正で削除された。
```

### 第3次修正（2026-05-18）

**⑤ `audioend` ハンドラから `armNoResultWatchdog(2000)` を削除**

第2次修正④で追加した `armNoResultWatchdog(2000)` が、`soundend` でセットした 8000ms を
2000ms に上書きしていた。STT サーバーの応答は `audioend` 後 2000ms 以上かかることがあり、
結果が届く前にウォッチドッグが発火して「Could not recognize speech」トーストが出ていた。

```js
// before（第2次修正後）
_onRecognition('audioend', (e) => {
  // ...
  armNoResultWatchdog(2000);  // ← soundend の 8000ms を 2000ms に上書き（原因）
  // ...
});

// after（第3次修正）
_onRecognition('audioend', (e) => {
  // ...
  // armNoResultWatchdog 呼び出しなし: soundend でセットした 8000ms をそのまま継続
  // ...
});
```

## 変更ファイル

| ファイル | 内容 |
|---|---|
| `web/src/app.js` | `audioend` ハンドラから `clearNoResultWatchdog()` を削除、`voice-processing` クラス付与/除去を各ハンドラに追加（第1次）; ボタンクリック時 150ms ディレイ追加（第2次）; `audioend` ハンドラの `armNoResultWatchdog(2000)` を削除（第3次） |
| `web/src/styles.css` | `#voice-bar.voice-processing` の CSS 追加（ボーダーパルス + 波形薄表示 + 「認識中...」テキスト） |

## 検証

### 第1次修正（2026-05-17）
- <TODO: 再ビルド後に音声入力ボタンを押して発話 → 「認識中...」表示が出ることを確認>
- <TODO: 発話完了後に ✓ または入力テキスト確認を行い、SR:result が届いてテキストが入ることを確認>
- <TODO: ✕ を押した場合に「認識中...」が消えてテキストエリアが元に戻ることを確認>
- <TODO: 8000ms 経過しても result が来ない場合（ネットワーク断等）に watchdog が発火してエラートーストが出ることを確認>

### 第2次修正（2026-05-18）
- 同一 PC の Chrome で Google 音声検索（`google.co.jp`）は「音声テスト 音声テスト」が正常認識 → Chrome Web Speech API 正常を確認済み ✅
- ビルド後に音声入力を試したが、`audioend` 後 2000ms でウォッチドッグが発火してトーストが出て再発 → 第3次修正へ

### 第3次修正（2026-05-18）
- <TODO: ビルド後、any-ai-cli で音声入力ボタンを押して「SR:result が届いてテキストが入る」ことを確認>

## 備忘

- `continuous=false`（現在の設定）では Chrome が発話終了後に一括で STT リクエストを送るため、
  `interimResults=true` でも `audioend` までに interim 結果が来ないことがある（短い発話では特に顕著）。
  `continuous=true` にすれば発話中に interim が逐次届くが、それは別の挙動変更を伴う。
- 今回の修正で watchdog が `audioend` 後も継続するため、ネットワーク断など
  サーバーが全く応答しない場合は 8000ms 後に watchdog が発火してエラートーストが自動表示される。
  以前は watchdog が消えていたためユーザーが手動で ✕ を押すまで何も起きなかった。
- コメントが残していた `soundend` の意図（「サーバ応答待ちなので長い猶予に更新」）と
  `audioend` の実装（watchdog を消す）が矛盾していたことが原因。コメントが正しく実装が誤り。
- （2026-05-18）hw を無効化する際に `stopPromise` 経由のディレイも同時に消滅した。
  「hw のマイク解放待ち」は hw 固有の処理ではなく「abort→start の間の遅延」として機能していたため、
  hw 無効後も同等のディレイが必要。150ms は実験値（hw の Promise 解決が通常数十〜数百ms だった実績から設定）。
- Chrome の Network タブに `speech.googleapis.com` のリクエストが表示されないのは仕様（Chrome の内部実装経由のため）。
  Web Speech API の正常性確認は Google 音声検索で代替する。
