---
type: bugfix
status: draft
docsweep_state: watching
tags: []
owner: ishizakahiroshi
review_status: draft
related: []
last_reviewed: 2026-09-06
work_id: WK-20260906T124047346-c8b2654b
ai_provenance_version: 1
ai_author_agent: codex
ai_author_runtime: many-ai-cli
ai_author_provider: openai
ai_author_model_id: unknown
ai_author_model_display: unknown
ai_author_reasoning: unknown
ai_author_model_source: unavailable
ai_execution_refs: [AIX-20260906T124047346-36c36c53, AIX-20260906T124052394-2fb4096e]
---

# [様子見] 承認バーの開閉マーカーと左右設定を Hub の操作系にそろえる

## context配分

| C | 種別 | 内容 | 備考/注意点 | AI実行 | 実行モデル |
|---|---|---|---|---|---|
| C1 | done | 承認バーの開閉マーカーを状態同期し、左右設定の説明を実装へそろえる | `bun run check` まで。実画面は未確認 | AIX-20260906T124052394-2fb4096e | implementation: openai / unknown / unavailable |
| C2 | planned | Hub を起動した実画面で右上の `⊟/⊞`、左寄せ設定、リロード後の保持を確認する | ビルド・Hub 起動・ブラウザ操作はユーザー実施 |  |  |

実行順序: `C1 → C2`

本記録でいう「右側の開閉マーカー」は、承認待ち `#action-bar` の全文／コンパクト表示を切り替える `⊟/⊞` ボタンを指す。受領した画像ファイルはこの作業環境に存在しなかったため、既存の Hub 実装と設定名から対象を確定した。

## 症状

- 承認バー右上の `⊟/⊞` は、通常の再描画経路では更新されるが、options cache が無い経路で状態を切り替えると、表示グリフ・タイトル・支援技術向け状態が同時に更新されない。
- 開閉マーカーも左右へ寄せる対象なのに、設定画面の説明は `✕` の閉じるボタンだけを対象としており、利用者が「開閉・閉じるボタン」の設定で動くことを判断できない。
影響範囲は Web UI と設定表示で、承認入力そのもの・Hub の PTY 送信経路は変更しない。

## 根本原因

- `appendCollapseToggle` が初期描画時に `textContent` と `title` だけを設定し、クリック後の状態同期を個別に持っていなかった。cache が空の分岐は CSS class だけを切り替えていたため、マーカー DOM の状態が stale になり得た。
- `ui-side.ts` の `close` 状態は既に `body.close-left` と `.action-collapse-btn` の CSS へ接続済みだったが、設定ラベル・説明・⇄ ツールチップが `✕` だけを記載していた。

## 修正内容

- `approval.ts` に開閉マーカー同期関数を追加し、グリフ、タイトル、`aria-label`、`aria-expanded`、`aria-controls` を同じ状態から更新するようにした。ボタンを `type="button"` とし、クリック時の既定動作も抑止した。
- options cache が空の場合も、`collapsed` class とマーカーの表示・アクセシビリティ属性を同時に更新する。cache がある通常経路は再描画後にも同じ同期を通す。
- Settings の左右設定を「開閉・閉じるボタン」と明示し、`⊟/⊞/✕` が対象であることを ja/en/vi の説明へ反映した。既存の `uiSideClose` と `body.close-left` による左右移動は維持する。

## 変更ファイル

- `web/src/app/approval.ts` — 承認バー開閉マーカーの状態同期とクリック処理。
- `web/src/index.html` — 左右設定のラベル・説明文。
- `web/src/styles.css` — 開閉・閉じるボタンを扱う CSS コメントの明確化（実効 CSS は既存の `body.close-left` ルールを使用）。
- `web/src/i18n/ja.json` / `web/src/i18n/en.json` / `web/src/i18n/vi.json` — 左右設定と ⇄ ツールチップの翻訳。
- `docs/local/bugfix_action-bar-collapse-marker-side-settings_2026-09-06.md` — 本記録。

## 検証

- `bun run check`（`web/`）: 成功。
- ja/en/vi の JSON パース: 成功。3 ファイルは各 1673 キーで一致。
- `git diff --check`: 空白エラーなし（既存ファイルの CRLF 警告のみ）。
- 実画面: 未確認。`make build`、Hub の起動／再起動、ブラウザのリロードは実施していない。

## 備忘

- C2 では Settings → General の「操作系の左右: 開閉・閉じるボタン」を左へ変更し、承認バー表示中に `⊟` と `✕` の並びが左上へ移ること、右へ戻すと右上へ戻ることを確認する。
- `⊟` はコンパクト化、`⊞` は全文表示を表す。`aria-expanded="true"` は全文表示中、`false` はコンパクト表示中にする。
- ビルド成果物の更新、Hub の起動、ブラウザでの実画面確認は未実施。
