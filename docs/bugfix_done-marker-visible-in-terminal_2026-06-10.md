# [完了] 障害対応記録: [ANY-AI-CLI-DONE] マーカーがターミナルに表示される

> 最終更新: 2026-06-10(水) 19:48:08

## 症状

`ANY_AI_CLI=1` 環境下で AI がタスク完了時に出力する `[ANY-AI-CLI-DONE]...[/ANY-AI-CLI-DONE]` ブロックが、any-ai-cli の xterm.js ターミナル画面にそのままテキストとして表示される。

再現手順:
1. any-ai-cli Hub 管理下のセッションで AI にコーディングタスクを依頼する
2. タスク完了時、AI の返答末尾に `[ANY-AI-CLI-DONE] ... [/ANY-AI-CLI-DONE]` が出力される
3. ターミナル画面にマーカー文字列とサマリーテキストが表示される

影響: Hub 管理下の全セッション。マーカーが非表示にならないため UI が汚れる。

## 根本原因（root cause）

`web/src/app/terminal.ts:916` の `hubMarkerBytePatterns` に `[ANY-AI-CLI-DONE]` / `[/ANY-AI-CLI-DONE]` が含まれていなかった。

`filterHubMarkersForDisplay()` はこの配列に登録されたパターンのみをスキップするため、DONE マーカーはフィルタされずそのまま xterm.js に書き込まれた。

また `[ANY-AI-CLI]` はタグのみを非表示にしてコンテンツ（承認プロンプト）を表示する設計だが、`[ANY-AI-CLI-DONE]` はタグ＋コンテンツ（完了サマリー）を丸ごと非表示にする必要があり、既存の単純なパターンスキップでは対応できなかった。

## 修正内容

`web/src/app/terminal.ts` に以下の変更を加えた。

**① DONE マーカー定数を追加**

```typescript
// before（hubMarkerBytePatterns の後）
export const hubMarkerEndBytes = hubMarkerBytePatterns[1];
export const eraseDisplayBelowBytes = new TextEncoder().encode('\x1b[J');

// after
export const hubMarkerEndBytes = hubMarkerBytePatterns[1];
export const hubDoneMarkerOpen = new TextEncoder().encode('[ANY-AI-CLI-DONE]');
export const hubDoneMarkerClose = new TextEncoder().encode('[/ANY-AI-CLI-DONE]');
export const eraseDisplayBelowBytes = new TextEncoder().encode('\x1b[J');
```

**② isPossibleMarkerPrefix を拡張**

チャンク境界で DONE open マーカーが分断された場合にも carry バッファで正しく処理されるよう、`hubDoneMarkerOpen` も部分一致チェック対象に追加。

```typescript
// before
export function isPossibleMarkerPrefix(bytes, offset) {
  const remaining = bytes.length - offset;
  return hubMarkerBytePatterns.some((pattern) => { ... });
}

// after
function isPossiblePrefix(bytes, offset, patterns) { ... }

export function isPossibleMarkerPrefix(bytes, offset) {
  return isPossiblePrefix(bytes, offset, hubMarkerBytePatterns) ||
    isPossiblePrefix(bytes, offset, [hubDoneMarkerOpen]);
}
```

**③ filterHubMarkersForDisplay に inDone フラグを追加**

`[ANY-AI-CLI-DONE]` 検出後は `inDone = true` にしてクローズタグまで全バイトをスキップ。クローズタグ検出後に `\x1b[J` を出力し `inDone = false` に戻す。`t.inDoneBlock` でチャンク間の状態を保持。

## 変更ファイル

| ファイル | 内容 |
|---|---|
| `web/src/app/terminal.ts` | `hubDoneMarkerOpen/Close` 定数追加、`isPossibleMarkerPrefix` 拡張、`filterHubMarkersForDisplay` に inDone ブロックスキップ追加 |

## 検証

- <TODO: make build 後、Hub 管理下セッションでタスクを実行し DONE マーカーが非表示になること>
- <TODO: `[ANY-AI-CLI]` 承認マーカーの動作に影響がないこと>
- <TODO: チャンク境界でマーカーが分断されるケース（大量出力時）でも正しく非表示になること>
