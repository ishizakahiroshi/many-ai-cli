---
type: reference
status: draft
tags: [ui, sidebar]
owner: user
related: [README.md, REVIEW.md, PROGRESS.md]
last_reviewed: 2026-10-20
---

# #P-20261020-004 根拠（実コードで確認した事実）

確認日: 2026-10-20。確認した SHA: `754f5f5782b1bb7762c3b730e73ce93f97113a8d`。行番号はこの SHA のもので、着手前に実物を開いて確かめる。ここにある事実と実コードが食い違ったら、README の「止まる条件」3 に当たる。

## カード

| 事実 | 出典 |
|---|---|
| カードの組み立ては `renderSessionList` の `entries.forEach(...)`。1 段目は `card-title-row`（状態 pill・`#id`・`cardProviderModelHtml(s)`・題・`cardMemoBadgeHtml(s)`）、2 段目は `card-meta-row`（`card-status-slot` / `card-done-slot` / `card-meta-extra` / `card-ctx-slot` / 🔕 / 子トグル / ブランチ） | `web/src/app/session-list.ts` 1273〜1366 行付近 |
| 権限モードのチップは `permissionModeChipHtml`。値は**起動時の 1 点**で、あとから CLI 側で変えても追従しない。空（フラグなし起動）のときはチップごと出さない。文言のキーは `card_permission_mode` / `card_permission_mode_tooltip` / `permission_mode_*` | `web/src/app/session-list.ts` 102〜113 行、1319〜1327 行。`web/src/i18n/ja.json` 465〜472 行 |
| 権限モードの値は `s.permission_mode` | `web/src/types/proto.ts` 424・564 行 |
| ブランチのバッジ `branchBadge` は**常に出る**（git 外は "(no git)"）。クリックで Git タブを開く。クリックを無視する対象の判定にも `.card-branch` が入っている | `web/src/app/session-list.ts` 1341〜1348 行、963〜1002 行、769 行付近。`web/src/app/session-list-tabs.ts` 262 行 |
| 1M は `cardCtxHtml` の中の `card-ctx-1m`。フッターは `token-statusbar.ts` の `tsb-1m` が「ctx %」の隣に出している | `web/src/app/session-list.ts` 713〜720 行。`web/src/app/token-statusbar.ts` 501 行 |
| 完了サマリー `cardDoneHtml` は待機側の状態でだけ出る。見ていないセッションの完了に気づける唯一の手がかり。部分更新が `.card-done-slot` を探す | `web/src/app/session-list.ts` 698〜711 行、749 行付近 |
| カード自身が `c.dataset.tooltip`（題と最初のメッセージ）を持つ。全体の `data-tooltip` は遅延なしで表示される（コンパクトで残すとポップと二重に出る） | `web/src/app/session-list.ts` 1292 行。`web/src/app/settings.ts` 281〜323 行 |
| カードの CSS | `web/src/styles.css`: `.card` 1318 行、`.card-title-row` 1366 行、`.card-done-slot` 1449 行、`.card-branch` 1475 行、`.card-permission-chip` 1485・1511・1625 行、`.card-meta-row` 1605 行 |

## ヘッダーとフッター

| 事実 | 出典 |
|---|---|
| ヘッダーは `#session-info-chip`（タブバー `#unified-tab-bar` の左端）。`renderSessionInfoChip` が「#番号・状態・プロバイダ・モデル・effort・🧠・エージェント名・契約バッジ」を描く | `web/src/app/settings.ts` 2976〜3039 行。`web/src/index.html` 1748 行。`web/src/styles/tabs.css` 208〜240 行 |
| `renderSessionInfoChip` の呼び出しは 3 か所 | `web/src/app/session-list.ts` 222・304・1563 行 |
| 契約バッジは「カード側には出さない（常時表示は情報過密）」と書いて、ヘッダーだけに出している先例 | `web/src/app/settings.ts` 3027〜3034 行 |
| ブランチ名はセッション帯にも出ている（開いているプロジェクトの全セッション分）。省略は `abbreviateBranchName` | `web/src/app/session-strip.ts` の `sessionItemHtml`。`web/src/app/session-strip-branch.ts` |
| セッションのブランチ値の更新経路 | `web/src/app/ws-client.ts` 632 行付近 |
| 🔒 は「表示モード固定」でタブに付くマークで、ヘッダーのチップではない | `web/src/styles/tabs.css` 368 行付近 |

## 設定と検査

| 事実 | 出典 |
|---|---|
| 表示の設定は「文字サイズ」の行の並び。起動時の反映は即時関数 | `web/src/index.html` 145〜152 行。`web/src/app/settings.ts` 1280〜1300 行 |
| サーバー同期する設定の対応表。ここに載せなければ同期しない（端末ごと） | `web/src/app/user-prefs.ts` 239 行付近（`_USER_PREFS_PATH_TO_LS`） |
| 端末ごとの保存の先例（帯の高さ） | `web/src/app/session-strip.ts` の `STORAGE_STRIP_HEIGHT_KEY` |
| 0.4 秒ポップの先例 | `web/src/app/project-group-path-popover.ts`（`SHOW_DELAY_MS = 400`） |
| PR の `Validate` は frontend の型検査（`bun run check`）・UI 部品検査（`scripts/check-ui-components.mjs`）・モジュール初期化検査（`scripts/check-web-module-init.mjs`）・ビルド・fixtures（`bun run test`）を走らせる。fixtures は `web/package.json` の `test` に列挙されたものだけ | `.github/workflows/validate.yml`、`web/package.json` |
| PR の `doxguard`（個人パス・ホームパス・メールの検査）と `secret-scan` も走る。fixture やドキュメントに個人パスを書かない | `.github/workflows/doxguard.yml`、`.github/workflows/secret-scan.yml` |

## 設計上の約束（守る）

- 画面の JS を読み込んだ瞬間に、循環 import の相手の変数を同期で読まない（`CLAUDE.md` の設計原則の索引にある。`web/src/app/boot-guard.ts` の冒頭）
- 本線は Rust 版（`develop`）。`go/` は凍結してあり、ビルド・改修・調査をしない
- 日本語 UI 文言は `ja.json` / `en.json` / `vi.json` の 3 つへ同時に足す
- 実装の進め方の詳しい規約は `CLAUDE.md` と `CLAUDE/coding.md`。**着手前に読む**（理由: 設計原則の索引と、コーディング規約がここにあり、この指示に全部は写していない）

## 承認の経緯

見た目は 2026-10-20 に利用者が承認した（動くモック: [mock.html](mock.html)）。モックは構成と動きを確かめるためのもので、実物の寸法・色ではない。モック中の項目名・文言は合成データ。D1〜D6 は承認時に未決定だったものをおすすめどおりに置いた既定で、README にある。
