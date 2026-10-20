---
type: reference
status: draft
tags: [ui, sidebar, session-list-tabs]
owner: user
related: [FACTS.md, REVIEW.md, PROGRESS.md]
last_reviewed: 2026-10-20
---

# #P-20261020-006 many-ai-cli: session delete keeps the tab

更新: 2026-10-20。セッションを削除すると、見ていたサイドバーのタブが別のタブへ勝手に切り替わる不具合を直す。[根拠](FACTS.md)・[独立レビュー指示](REVIEW.md)・[進捗看板](PROGRESS.md)も読む。コード中の行番号は基準 SHA 時点のもので、着手前に必ず実物を開いて確かめる。

## 対象と目的

- 作業 repo: `ishizakahiroshi/many-ai-cli`（公開）
- 作業 branch: `dots/P-20261020-006-session-delete-keeps-tab`（この指示書もこの branch にある）
- Draft PR の base: `develop`（Rust 版の本線。Go 版は `go/` に凍結してあり触らない）
- 挙動の基準 SHA: `754f5f5782b1bb7762c3b730e73ce93f97113a8d`
- 管理 Issue: https://github.com/ishizakahiroshi/many-ai-cli/issues/28
- 実装 diff の起点は初回指示公開 commit `0c23b0eae3f5a430e5d4ea76d97876e59096a6f8`（基準 SHA とは別）

サイドバー上部にはタブ（Tab 1 / Tab 2 …）があり、セッションはどれか 1 つのタブに属する。利用者があるタブを見ながら、そのタブのセッション（画面に開いているもの）を削除すると、見ていたタブが別のタブへ切り替わってしまう。目的は、**削除してもタブが切り替わらないこと**。

## 原因（実コードで確認済み・根拠は FACTS.md）

開いているセッションを削除すると、画面は「隣のセッション」を開き直す。この隣を、タブを無視した全体の並びから選び、開くときにタブを動かさない指定をしていない。

1. `web/src/app.ts` の `removeLocalSession`（1814 行付近）が、`orderSessions()`（全タブ共通の並び）で「削除カードの直上、先頭なら直下」を隣として選ぶ（1817〜1828 行）
2. 同関数の 1864〜1869 行で `activateSession(next)` を呼ぶ。`preserveSessionListSelection` で包んでいない
3. `activateSession` は `updateSessionListActiveCard`（`web/src/app/session-list.ts` 155 行）を通って `revealSessionListOwner`（`web/src/app/session-list-tabs.ts` 187 行）を呼ぶ。これは、`preserveSessionListSelection` の中でなければ、開いたセッションの所属タブへ切り替える
4. 隣が別のタブの所属だと、タブごと切り替わる

同じ関数の周りの自動アクティブ化は包んである（`session-list.ts` の `render()` 1871 行、`ws-client.ts` 615 行、`session-list.ts` 898 行）。削除後の付け替えだけが漏れている。

合成入力での測定（画面ではない）: プロジェクト a のセッション #1・#2 を Tab 1、プロジェクト b の #3 とプロジェクト c の #4・#5 を Work に置く。全体の並びは 1,2,3,4,5。Work を見ながら #3 を削除すると隣は #2（Tab 1）になり、タブが切り替わる。#4・#5 の削除は同じタブ内の隣なので切り替わらない。

## 修正の方針（決定済み・変えない）

隣を「見ているタブ（`selectedId`）の中」から選ぶ。見ているタブの中に隣が無いときだけ従来の式で選び、そのときはタブを動かさずに開く。**タブは常に動かさない。**

### 純関数（`web/src/app/session-list-tab-state.ts`）

```ts
export function sessionListNeighborAfterRemoval(state: SessionListTabState, sessions: SessionSnapshot[],
    ordered: number[], removedId: number): { id: number; sameTab: boolean } | null
```

- `owners = sessionListOwners(state, sessions)`。削除前の全セッションで計算する
- 削除するセッションの所属が `state.selectedId` のとき、`ordered` のうち `state.selectedId` に属する id だけを順に並べた列で「直上、先頭なら直下」を選び、あれば `{ id, sameTab: true }`
  - その列で `idx > 0` なら `idx - 1`、`idx === 0` で 2 件以上なら `1`
- それ以外（見ているタブの中に隣が無い、または削除するセッションが見ているタブの所属でない）は、`ordered` 全体に同じ式（`app.ts` 1825〜1826 行と同じ）を使い `{ id, sameTab: false }`
- 全体でも隣が無ければ `null`

### 薄い export（`web/src/app/session-list-tabs.ts`）

`state()` は同ファイルの非公開関数なので、純関数を `state()` と `all()` で呼ぶ薄い export を足す。名前は状態モジュールの関数と衝突しないようにする（例: `neighborAfterSessionRemoval(removedId, ordered)`）。

### 配線（`web/src/app.ts` の `removeLocalSession`）

- 隣の計算は、これまでどおり `sessions.delete(id)` の**前**で行う
- `sameTab: true` のときは包まずに `activateSession(neighbor.id)`。`revealSessionListOwner` が「タブごとの最後に開いたセッション」の記憶を更新するため
- `sameTab: false` のときは `preserveSessionListSelection(() => activateSession(neighbor.id))`。タブを動かさず、画面だけ従来どおり隣を開く
- `sessions.has(neighborId)` で消えていたときの先頭へのフォールバック（`sessions.keys().next().value`）は残し、その場合も包む
- `preserveSessionListSelection` と新しい export を `app.ts` に import する。画面の JS の読み込み順（循環 import）を壊さない

## 実装の単位（C1 のみ）

共有ファイルの持ち主は 1 担当。

### C1: 純関数・export・配線・fixtures

変更してよいファイルは次の 4 つだけ。

1. `web/src/app/session-list-tab-state.ts`: 純関数を追加
2. `web/src/app/session-list-tabs.ts`: 薄い export を追加
3. `web/src/app.ts`: `removeLocalSession` の隣の選び方と `activateSession` の呼び方を差し替え、import を追加
4. `web/src/app/session-list-tab-fixtures.ts`: fixtures を追加

fixtures は既存の流儀（`session(id, extra)` ヘルパーと `normalize` / `add` / `move` の別名）に合わせて追加する。入力は上の測定と同じ（Tab 1 に #1・#2、Work に #3・#4・#5）。**5 件**、期待値は次のとおり:

| 入力 | 期待 |
|---|---|
| Work を見ていて #4 を削除 | `{ id: 3, sameTab: true }` |
| Work を見ていて #3（Work の先頭）を削除 | `{ id: 4, sameTab: true }`。同じテスト内で、従来の式（`ordered` 全体の直上）なら #2（別のタブ）になることも確かめ、対照にする |
| Tab 1 に #1・#2、Work には #3 だけ。Work を見ていて #3 を削除 | `{ id: 2, sameTab: false }` |
| 全体で #1 の 1 件だけ。それを削除 | `null` |
| Tab 1 を見ているが、削除するのは Work の所属の #4 | `{ id: 3, sameTab: false }` |

`web/package.json` の `test` の列挙には `dist/app/session-list-tab-fixtures.js` が**すでに入っている**ので、列挙は足さない（FACTS.md に確認した事実を書いた）。

## 境界（触らないもの）

- 手動でのタブ切替、カードのクリック、検索、次/前のセッションへ、スワイプなど、利用者の操作で別のタブのセッションを開く経路。タブが追従するのが正しい
- 承認の自動切替（`web/src/app/state.ts` の `maybeAutoSwitchToNextApproval`）と、新規セッションの自動切替（`web/src/app/ws-client.ts` の `pendingAutoSwitch`）。別のタブのセッションへ移ること自体が目的
- 見ているタブの最後の 1 件を削除したあと、画面に別のタブの会話が開く件（サイドバーは空のタブ、右側は別タブの会話になる）。起動時・再接続時にすでにある状態と同じで、この修正では変えない。直す案は別に管理する
- `rust/`、`go/`（凍結）、`web/src/i18n/*.json`、`web/src/styles/*`、CI workflow、依存、`web/dist/`（生成物）
- `web/tests/session-list-tabs-dom.test.ts`（既存のまま通ることを確かめるだけで、編集しない）

## 検証

- dots の作業環境では依存を取得できないことがある。**取得できないまま検査が通ったことにしない。** できなかった検査は「未実施」と書き、PR の CI を根拠にする
- 手元でやってほしい静的検査（できれば）: `web` ディレクトリで `bun run check`（`tsc --noEmit`）、`bun test ./src/app/session-list-tab-fixtures.ts`、`bun test ./tests/session-list-tabs-dom.test.ts`。リポジトリ直下で `node scripts/check-web-module-init.mjs`。**`bun run test` は内部でビルドするので、この指示では使わない**（ビルドは PR の CI に任せる）
- 基準値（基準 SHA・2026-10-20 に手元で実行）: `session-list-tab-fixtures.ts` は 22 件通過、`session-list-tabs-dom.test.ts` は 14 件通過。追加後は fixtures が 27 件になる
- PR の `Validate` は frontend の型検査・UI 部品検査・モジュール初期化検査・ビルド・fixtures を走らせる。実行された step 名と結果を、skip を成功に数えずに報告する
- 利用者の Windows 実機・稼働 Hub・実画面での動き（タブを 2 つ作り、Work の先頭のセッションを開いたまま削除しても、Work タブが選ばれたままであること）は dots には確かめられない。**手元の利用者が確認する。** dots の完了報告で「画面で確認した」と書かない

## 完了条件

- C1 の 4 ファイルの変更が済み、5 件の fixtures が追加され、PR の CI が実行範囲つきで緑
- `removeLocalSession` が、隣を見ているタブの中から選び、タブを動かさずに開く（`git diff` の対象ファイルで示す）
- 変更ファイルが上の 4 つだけである（`git diff --stat` で示す）
- 独立レビュー（REVIEW.md）の指摘が直り、**最終の code SHA** を対象に再レビューされている
- 未実施のもの（実機の動き・ビルドの実行）は、未実施と書いてある

## 提出

- コードは作業 branch に push し、`develop` 向けの Draft PR にする。報告に code SHA・実行した command と exit code・残件・rollback の方法を書く
- 管理 Issue に、受付・着手・完了・レビュー・停止・質問・最終提出のたびに短いコメントを書く（**Slack だけの報告は手元で検知されない**）
- `PROGRESS.md` は手元が更新する。dots は書かない
- commit 末尾に `Agent: dot`、PR にラベルを付ける。merge はしない

## 受付と返信の書き方

- 最初に同じ案件のスレッドへ、認識した案件番号・読んだ指示の commit・実行環境（できる検査とできない検査）を返す
- 返事は毎回この案件のスレッドの中だけに書き、受付・進捗・質問・停止・修正提出・完了の 1 行目を `#P-20261020-006 many-ai-cli：<報告内容>` にする。スレッドの外に書かない
- 進行中の補足は再起動の依頼ではない。読んだ revision を返す

## 許可と禁止

**許可（書く）**

- 作業 branch `dots/P-20261020-006-session-delete-keeps-tab` へのコード・テストの commit と push
- `develop` 向けの Draft PR の作成と更新
- 管理 Issue へのコメント
- 独立レビューの指摘の修正
- 作業環境で使える既存の道具による静的な検証

**禁止（書かない）**

- `main` / `develop` 等の実 base への直接 push、merge、release、deploy
- 稼働中の Hub の起動・停止・再読み込み、実アカウントでの確認、認証情報・秘密値・Hub のトークンの扱い
- 上の「境界」に挙げたファイル・経路の変更、新しい依存・外部サービス、CI workflow の変更
- 無関係な branch・Issue・Project への書き込み
- 私有情報の記載。fixture は合成データだけにする

## 取り消されたとき

- 作業 branch・Draft PR への保存が取り消されたら、状態を確かめてから 2 回まで送り直してよい。branch を動かす前の blob・tree・commit の作成は、そのまま送り直す。branch の作成と更新は先に作業 branch の head を読み、送る前のまま（作成ならまだ無い）なら送り直し、送ろうとした commit になっていれば通っていたとみなして次へ進み、どちらでもなければ止まる。force push はしない。Draft PR の作成は、同じ branch からの PR が既にないかを確かめてから送り直す
- 3 回目も取り消されたら止まり、base の commit SHA と `git format-patch` の出力を Google Drive の `bot/blob/2026/10/P-20261020-006/v1/` に置く（直下とその配下のすべてのサブフォルダへのファイルの保存とフォルダの作成を許可する。`manifest.json` に相対パス・bytes・SHA256 を書き、最後に保存する）。手元が当てて Draft PR にする
- 審査で拒否されたときは Drive にも回さずに止まり、どこまで通ったか（branch の head・PR の有無）を報告する
- 「再試行しない」は、取り消された同じ操作だけを指す

## 止まる条件

次のどれかに当たったら止まり、管理 Issue にコメントして、同じスレッドへ報告する。

1. 同じ理由の CI 失敗が 2 回続く
2. この指示の変更範囲の外を触る必要が出た
3. 指示と実コードの食い違い（例: 書いてあるファイルや関数が無い、行番号の先の中身が違う）が見つかった
4. 重大なレビュー指摘が 2 回直しても残る
5. 持ち主の操作が要る（GitHub の確認画面・CI が始まらない）
6. 1 工程が 90 分を超える
