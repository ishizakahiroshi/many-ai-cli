# [完了] 障害対応記録: 音声入力が Chrome サイトデータ破損で stuck（onresult/onend が来ない）

> 最終更新: 2026-05-18(月) 02:47:46

## 症状

ある Windows 11 PC で `any-ai-cli serve` 起動後、Chrome から音声入力ボタンを押して発話すると、以下のログまでは進むが `onresult` / `onend` / `onerror` のいずれも発火せず、UI が「認識中…」表示のまま固まる。

```
[VOICE-DBG] init #1
[VOICE-DBG] btn.click current#1 isRecording=false
[VOICE-DBG] calling recognition.start() on #1
[VOICE-DBG] #1 onstart
[VOICE-DBG] #1 onaudiostart
[VOICE-DBG] #1 onsoundstart
[VOICE-DBG] #1 onspeechstart
[VOICE-DBG] #1 onspeechend
[VOICE-DBG] #1 onsoundend
[VOICE-DBG] #1 onaudioend
（ここで停止 — 数十秒待っても続きが来ない）
```

- `init #1` は一度しか出ておらず、`stopVoice() created next #N` も出ていない（インスタンス再生成は走っていない）
- 同じ exe を別の Windows 11 PC で動かすと正常動作するため、**コード側ではなく Chrome プロファイル / 環境側の問題**
- 同じ PC・同じ Chrome の **シークレットモード**で開くと正常動作 → プロファイル固有のサイトデータが原因と切り分けられた

## 根本原因（root cause）

Chrome プロファイルが `http://127.0.0.1:47777` に対して保持していたサイトデータ（マイク権限 / LocalStorage / Cookie / Cache のいずれか、または複数）が破損していたため、Chrome 側の Web Speech API が音声を Google STT へ送信した後の応答ハンドリングが stuck する状態になっていた。

- `onaudioend` までは Chrome のローカル処理（音声取得・送信前処理）で進行
- その後の **「Chrome → Google STT サーバーへの送信〜応答受信〜JS イベント発火」** のいずれかでハングする
- シークレットモードはサイトデータを共有しないため正常動作した
- 拡張機能を全 OFF にしても通常モードでは再現したため、拡張機能ではなくサイトデータ側

`web/src/app/voice.js` のコードロジックには問題なし（`bugfix_voice-recognition-rewrite_2026-05-18.md` の書き直しは正しい設計のまま）。

## 修正内容

コード変更なし。Chrome 側の操作のみで復旧:

1. Chrome の `chrome://settings/content/all?searchSubpage=127.0.0.1` を開く
2. 該当エントリ（`127.0.0.1`）のゴミ箱アイコンを押してサイトデータを全削除
3. ハブ URL を開き直し、マイク許可ダイアログで「許可」を再付与
4. 音声入力ボタン押下 → 通常通り `onresult` / `onend` が発火するようになった

切り分け順序（再発時の参考）:

| 順 | 操作 | 切り分け対象 |
|---|---|---|
| 1 | シークレットモードで再現確認 | プロファイル側 vs コード側 |
| 2 | 拡張機能を全 OFF にして通常モードで再現確認 | 拡張機能 vs サイトデータ |
| 3 | `127.0.0.1` のサイトデータ削除 → マイク権限再付与 | サイトデータ破損 |
| 4 | （上記で復旧しなければ）Chrome 全体のキャッシュ + Cookie を過去 1 時間で削除 | プロファイル全体 |

## 変更ファイル

| ファイル | 内容 |
|---|---|
| （なし） | コード変更なし。Chrome プロファイル側のサイトデータ削除のみで復旧 |

## 検証

- 該当 PC で `127.0.0.1` のサイトデータ削除 + マイク許可再付与後、音声入力ボタン押下 → 発話 → `onresult` 発火 → テキストエリアに文字が入ることを確認 ✅
- 別 Windows 11 PC では同じ exe・同じコードで終始正常動作（環境差を裏付け）✅

## 備忘

- **コードを疑う前に「シークレットモードで動くか」を最初に試す**。動けば 99% プロファイル側。`bugfix_voice-recognition-rewrite_2026-05-18.md` の書き直しが原因と誤判定して再リライトする罠を避けられる
- Chrome の Web Speech API は内部で Google STT サーバーへ HTTPS 送信するが、Network タブには現れない（Chrome 内部実装経由）。「通信していないように見える」のは正常
- 切り分け手順の詳細は [docs/local/reference_voice_stt_troubleshoot.md](local/reference_voice_stt_troubleshoot.md) に集約
- 同じ症状が他 PC でも出る可能性があるので、初回 setup 手順や FAQ にこの切り分け手順を入れることも検討（今回はリファレンス化のみ）
