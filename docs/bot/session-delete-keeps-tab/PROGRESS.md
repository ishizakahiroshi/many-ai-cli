---
type: reference
status: draft
tags: [ui, sidebar, session-list-tabs]
owner: user
related: [README.md, FACTS.md, REVIEW.md]
last_reviewed: 2026-10-20
---

# #P-20261020-006 many-ai-cli: session delete keeps the tab 看板

更新: 2026-10-20。この看板は手元が更新する（dots は書かない）。状態の正本は管理 Issue のコメント。

- 管理 Issue: https://github.com/ishizakahiroshi/many-ai-cli/issues/28
- 指示書の保存先: `ishizakahiroshi/many-ai-cli` の `docs/bot/session-delete-keeps-tab/`
- 作業 branch: `dots/P-20261020-006-session-delete-keeps-tab`
- Draft PR の base: `develop`
- 挙動の基準 SHA: `754f5f5782b1bb7762c3b730e73ce93f97113a8d`
- 初回指示公開 commit: `0c23b0eae3f5a430e5d4ea76d97876e59096a6f8`

| 工程 | 担当 | Issue 上の状態 | 証跡・残件・次の一手 |
|---|---|---|---|
| 受付 | dots | 未受付 | 受付返信（番号・読んだ指示 commit・環境能力）を待つ |
| C1 純関数・export・配線・fixtures | dots | — | — |
| 独立レビュー | dots（実装とは別担当） | — | review 対象 code SHA を待つ |
| 手元の取り込み審査 | 手元 AI | — | 提出後に全行を読む |
| 手元検収（実機の動き） | 利用者 | — | ビルド後に、タブ 2 つで先頭のセッションを開いたまま削除し、タブが動かないことを目視 |

| 項目 | 値 |
|---|---|
| 実 code SHA | — |
| review 済み SHA | — |
| Draft PR | — |
| CI（実行 step と結果） | — |
