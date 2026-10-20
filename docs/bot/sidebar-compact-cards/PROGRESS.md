---
type: reference
status: draft
tags: [ui, sidebar]
owner: user
related: [README.md, FACTS.md, REVIEW.md]
last_reviewed: 2026-10-20
---

# #P-20261020-004 many-ai-cli: sidebar compact cards 看板

更新: 2026-10-20（提出後）。この看板は手元が更新する（dots は書かない）。状態の正本は管理 Issue のコメント。

- 管理 Issue: https://github.com/ishizakahiroshi/many-ai-cli/issues/27
- 指示書の保存先: `ishizakahiroshi/many-ai-cli` の `docs/bot/sidebar-compact-cards/`
- 作業 branch: `dots/P-20261020-004-sidebar-compact-cards`
- Draft PR の base: `develop`
- 挙動の基準 SHA: `754f5f5782b1bb7762c3b730e73ce93f97113a8d`
- 初回指示公開 commit: `7390fd9f70da8b2b096e175d08ebe2d9a18e2d3b`

| 工程 | 担当 | Issue 上の状態 | 証跡・残件・次の一手 |
|---|---|---|---|
| 受付 | dots | 受付済み | 番号の衝突なし・固定指示 commit 一致（Slack の報告を Issue #27 へ転記済み） |
| C1 判定関数と fixtures | dots | 完了 | fixtures 8 件。CI の Test frontend fixtures で実行を確認 |
| C2 ヘッダーへ権限・ブランチ | dots | 完了 | 幅（約 960px）は利用者の目視待ち |
| C3 コンパクト表示と設定 | dots | 完了 | 手元のレビュー指摘（スクロールで消える・ボード更新待ち・localStorage）を修正 |
| C4 検査・CHANGELOG | dots | 完了 | CHANGELOG 2 項目・README 1 行。CI 全 step success |
| 独立レビュー | dots（実装とは別担当）＋手元の別製品 AI 3 本 | 完了 | 詳細は Issue #27 |
| 手元の取り込み審査 | 手元 AI | 完了 | Drive patch を git am。tree 一致を確認 |
| 手元検収（実機の見た目） | 利用者 | 待ち | ビルド後に、行の高さ・ポップ・ヘッダー幅を目視 |

| 項目 | 値 |
|---|---|
| 実 code SHA | 440c1eac0e0796f45d0bae154556953bd8f07f63 |
| review 済み SHA | ab2b3ad6（dots 実装分）。440c1eac の修正差分は別担当の再レビューなし |
| Draft PR | https://github.com/ishizakahiroshi/many-ai-cli/pull/32 |
| CI（実行 step と結果） | web-check の全 step success、doxguard / gitleaks / instrumentation / package-manager-consistency pass、release-token-scopes は skipping |
