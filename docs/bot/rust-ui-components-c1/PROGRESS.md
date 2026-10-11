---
type: reference
status: draft
tags: [rust, ui, handoff]
owner: local-conductor
review_status: draft
related: [README.md, FACTS.md, REVIEW.md]
last_reviewed: 2026-10-11
---

# #P-20261011-005 many-ai-cli：C1資料の入口

> 最終更新: 2026-10-11(日) 19:19:10

管理Issue: [#16](https://github.com/ishizakahiroshi/many-ai-cli/issues/16)。送信・受付・調査提出・独立レビュー・手元検収の状態遷移は、このIssueのコメントに集約する。

## 指示と証拠

- [作業指示](README.md): 目的、五軸の棚卸し、許可/禁止、提出形式。
- [確認済み事実](FACTS.md): 公開source SHAと手元との差、停止モーダル、既存共通部品、自動CIの範囲。
- [独立レビュー手順](REVIEW.md): sourceと集計を別担当が検証する。レビューは未実施。
- 調査成果 `INVENTORY.md`: 未提出。提出後にリンクを追加する。
- 最初の指示公開commit: `fad045cba803e7827d31cdaddcb4f333f1f39f32`。レビューdiffの起点。管理Issue追記後の最終固定指示URLはIssue本文・送信文へ載せる。branch最新版URLと区別する。

## 完了していない受入

1. dotsの受付（番号、repo、固定指示commit、基準SHA、今回の能力、担当分離）。
2. 公開SHAの全画面名簿、五軸の分類、再現できる計数、停止モーダルの契約一覧。
3. 調査担当と別担当によるレビューと、修正後SHAの再確認。
4. 手元AIによる全差分・集計の照合、普段のビルド元にある追加変更の確認、非公開棚卸しへの反映。
5. 実画面の受入。sourceの相違を実画面のばらつきと断定しない。

次工程はC2の仕様資料と利用者確認。C3/C4の製品変更、C5の機械検査、ビルド、稼働版への反映は今回のC1成果では完了しない。

常設監視/自動配送の接続は今回の依頼で起動しない。手元担当のセッション中の直接読取りと、セッション終了後の監視を区別する。
