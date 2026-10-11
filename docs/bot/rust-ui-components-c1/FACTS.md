---
type: reference
status: draft
tags: [rust, ui, evidence]
owner: local-conductor
review_status: draft
related: [README.md, REVIEW.md, PROGRESS.md]
last_reviewed: 2026-10-11
---

# #P-20261011-005 many-ai-cli：C1の確認済み事実

> 最終更新: 2026-10-11(日) 18:51:57

## 利用者の目的と段階

2026-10-11にRust版の全画面の部品統一計画の実施とdots利用を依頼された。同じ役割のボタン・モーダル・アイコン・状態表示を共通化し、新規画面で既定部品を流用できることが目的。C1は棚卸し、C2は部品仕様の利用者確認、C3/C4は実装、C5は流用を保つ検査。今回の送信対象はC1。出典: 利用者の今回依頼と、その対象計画を手元担当が確認（私有計画そのものは外部へ渡さない）。

## sourceの基準

公開候補 `dots/rust-recovery-resume-3` は `e1ab622aa7708ae883a3cdd83d042fbf2cf31e38`。2026-10-11に `git ls-remote` とfetchで確認。本パケットはこのcommitから分離した。手元の候補にはその後の変更もあり、公開sourceと手元のsourceの同一性は未確認ではなく、相違があると確認済み。相違の全内容は手元検収で扱う。

`.omitnix/index.json` のgenerated.commitは `a114727651bdca2884c3d4b92262bbae240f6b2c`、dirty=true。2026-10-11の確認時点で調査SHAと異なる。索引は入口に使い、現行sourceの証拠には使わない。JSON/PNGなどの未解析が名簿にあるため、未解析を不存在・未使用扱いにしない。

## 代表例の描画元

調査SHAの `web/src/app/settings.ts:818` に `appConfirmShutdown` がある。`model-picker-overlay` を再利用し、`confirm-dialog confirm-dialog--danger` と `confirm-btn` のDOMを生成する。タイトルと本文は `shutdown_confirm_title` / `shutdown_confirm`、ボタンは `spawn_cancel` / `shutdown_sessions_only` / `shutdown_confirm_run` の翻訳キーを使う。2026-10-11、調査SHAのsourceを手元担当が読んで確認。

既存の契約: overlayが無いときのwindow.confirm fallback、Escと外側クリックのキャンセル、Enterのshutdown、shutdownボタンの初期focus、外部公開停止checkboxとstopExpose戻り値、クリック/keydown listenerの除去、`shutdown` / `sessions` / nullの三つの戻り値。挙動はsourceで確認したもので、実Hubで実行していない。

`web/src/styles/close-button.css` には既に閉じるアイコンの共通スタイルと名簿、寸法上書き、意図した除外、`body.close-left` への対応がある。既存共通部品が無いとは断定できない。2026-10-11、調査SHAのファイルを読んで確認。

## 保持する境界

- Rust版と関連Webを対象とする。Go版は退役予定で、追加調査・回帰確認をしない。出典: `CLAUDE.md` の今後の開発方針（2026-10-09）と利用者指定、2026-10-11に確認。
- サーバロジックは変更しない。後続のUI移行でも停止処理・承認・通信契約は維持する。
- 全画面overlayの `.aac-wheel-overlay` と部分popoverの `data-wheel-native` の規則を維持する。出典: `CLAUDE.md` の設計原則、`scripts/check-wheel-overlays.mjs`。後者は具体的検査ロジックが長いため調査時に直接参照する。
- 既存の外部UIライブラリ追加なしの方針。色方向、部品種別、幅の段階はC2で利用者の確認を受ける。今回は仕様決定しない。
- 実Hubのビルド・起動・停止・再起動・ブラウザリロードと画面受入は利用者側。出典: `CLAUDE.md` のAI作業共通ルール。画面未確認を静的検査で代替しない。

## GitHubでの自動実行

調査SHAの `.github/workflows/validate.yml` はpull_requestとmain/developのpushで発火し、Goの検査とfrontend buildを含む。`.github/workflows/rust-windows-check.yml` はRust候補branchの指定path変更またはmanual dispatch。指示branchのdocsだけのpushはRust Windows検査の対象ではない。2026-10-11にworkflow先頭を確認。Draft PR作成時の自動CIと手元での製品buildを区別し、workflowを変更して検査を抑制しない。

## 未確認

全画面の見た目、CSSの適用後の値、種類数の集計、dotsの今回の解析/書込み能力、C1成果、独立レビュー、CI結果、Windows/モバイル受入は未確認。過去計画の計数は今回のSHAで再測定する。添付画像の現物は今回dotsへ渡していない。
