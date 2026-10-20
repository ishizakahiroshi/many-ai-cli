---
type: reference
status: draft
tags: [ui, sidebar, session-list-tabs]
owner: user
related: [README.md, FACTS.md, PROGRESS.md]
last_reviewed: 2026-10-20
---

# #P-20261020-006 独立レビュー指示

目的は、[README.md](README.md) の仕様に対する実差分と失敗経路の確認。実装者とは別の担当が、初回指示公開 commit から実 code SHA までの差分を読む。文書と生成物を無関係差分と誤認しない。

## 見る観点

1. **範囲**: 変更が README の 4 ファイル（`session-list-tab-state.ts`・`session-list-tabs.ts`・`app.ts`・`session-list-tab-fixtures.ts`）だけである。`git diff --stat` で示す。`rust/`・`go/`・i18n・CSS・CI workflow・依存に差分が無い
2. **純関数**: 見ているタブの中の直上、先頭なら直下。見ているタブの中に隣が無いとき、削除するセッションが見ているタブの所属でないときは従来の式になる。全体でも隣が無ければ `null`。入力の `ordered` に `removedId` が含まれない場合や、`sessions` に無い id が混ざる場合に落ちない
3. **親子**: 削除するセッションが親で、子が同じタブにいる場合の隣の選び方（子が隣になりうる）。子がいなくなったあとの扱いが従来より悪くなっていない
4. **配線**: 隣の計算が `sessions.delete(id)` の前にある。`sameTab: true` は包まず、`sameTab: false` は `preserveSessionListSelection` で包む。`sessions.has(neighborId)` が偽のときのフォールバックも包んでいる。削除したのが開いているセッションでないとき（`activeSessionId !== id`）は、これまでどおり何も開かない
5. **タブが動かないこと**: 包んだ `activateSession` の中で、`revealSessionListOwner` がタブを切り替えないことを、コードの経路で示す（`preserveSelectionDepth` の増減が `try/finally` であること）
6. **読み込み順**: `app.ts` に足した import が循環 import を悪化させない。`node scripts/check-web-module-init.mjs` の結果を、実行された step 名つきで確かめる
7. **fixtures**: 5 件が仕様の表と一致している。「従来の式なら #2 になる」対照がテスト内にある。プロジェクトが別になる入力（`cwd` を分ける。FACTS.md の「検査」の表）で期待値が実際に出ている。既存の 22 件が通ったまま
8. **fixtures の実行**: 新しい fixtures が CI のログで実行されている（`session-list-tab-fixtures` の件数が 27 になる）。skip を成功に数えない
9. **個人情報**: fixture・コメント・文書に、個人パス・ホームパス・メールアドレス・実在のプロジェクト名が入っていない
10. **範囲外の挙動**: 承認の自動切替・新規セッションの自動切替・利用者の操作によるタブ追従が、これまでどおり動く経路のまま（差分に触れていない）

## 提出

- review 対象の code SHA、再現手順と重大度つきの指摘、実行した command とその結果、未検証の範囲
- 修正後の新しい SHA は影響範囲を検査し直す。**古い PASS を引き継がない**
- 指摘なしでも、対象範囲を書く
- 管理 Issue にコメントする
