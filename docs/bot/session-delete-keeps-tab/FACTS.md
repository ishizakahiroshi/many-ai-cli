---
type: reference
status: draft
tags: [ui, sidebar, session-list-tabs]
owner: user
related: [README.md, REVIEW.md, PROGRESS.md]
last_reviewed: 2026-10-20
---

# #P-20261020-006 根拠（実コードで確認した事実）

確認日: 2026-10-20。確認した SHA: `754f5f5782b1bb7762c3b730e73ce93f97113a8d`。行番号はこの SHA のもので、着手前に実物を開いて確かめる。ここにある事実と実コードが食い違ったら、README の「止まる条件」3 に当たる。

## 削除後の付け替え

| 事実 | 出典 |
|---|---|
| `removeLocalSession` は、削除するのが開いているセッション（`activeSessionId === id`）のとき、`orderSessions()` で「削除カードの直上、先頭なら直下」を `neighborId` に覚える。計算は `sessions.delete(id)` の前 | `web/src/app.ts` 1814〜1828 行 |
| 削除後、開いていたセッションなら `set_activeSessionId(null)` にして、`neighborId`（無ければ `sessions.keys().next().value`）を `activateSession(next)` で開き、`return` する。`preserveSessionListSelection` で包んでいない | `web/src/app.ts` 1857〜1871 行 |
| `orderSessions()` は全セッションを `buildSidebarTree` に通した順を返す。タブでは絞らない | `web/src/app/state.ts` 255〜263 行 |
| `activateSession` は `updateSessionListActiveCard` を通る（マルチペイン委譲の経路も同じ関数を通る） | `web/src/app/session-list.ts` 155〜171 行の記述 |
| `updateSessionListActiveCard` は `revealSessionListOwner(id)` を呼ぶ（160 行） | `web/src/app/session-list.ts` 155〜171 行 |
| `revealSessionListOwner` は、`preserveSelectionDepth` が 0 のとき、開いたセッションの所属タブが選択中のタブと違えば `select(owner)` でタブを切り替える。同じタブなら切り替えず、「タブごとの最後に開いたセッション」の記憶（`rememberSessionListActive`）だけ更新する | `web/src/app/session-list-tabs.ts` 183〜193 行 |
| 所属は `sessionListOwners(state, sessions)`。ルートのセッションの lifecycle identity から `state.memberships` を引き、子はルートに従う。無ければ `state.defaultId` | `web/src/app/session-list-tab-state.ts` 144〜153 行 |
| 見えている木はタブごとに `filterSessionListTree` で絞られ、木の順は保たれる（利用者が見るカードの順と、`orderSessions()` を `sessionListOwners` で絞った順は一致する） | `web/src/app/session-list-tab-state.ts` 222〜227 行。`web/src/app/session-list-tabs.ts` 179〜181 行 |

## 「タブを動かさない」包みの先例

| 事実 | 出典 |
|---|---|
| 初回アクティブ化は `preserveSessionListSelection(() => activateSession(sessions.keys().next().value))` | `web/src/app/session-list.ts` 1870〜1873 行 |
| 再接続時の復元は `if (reconnectRestore) preserveSessionListSelection(() => activateSession(id))` | `web/src/app/ws-client.ts` 615 行 |
| プロジェクト箱の復元も包んでいる | `web/src/app/session-list.ts` 898 行 |

## 他の `activateSession` 呼び出し元

web/src の `activateSession(` の呼び出し元は 37 箇所（fixtures を除く）。

- 利用者の操作が起点のもの（カードのクリック、検索、次/前のセッションへ、スワイプ、通知、設定画面、承認の一覧など）は、タブが追従するのが正しい
- 自動で別のタブのセッションへ移ること自体が目的のもの: 承認の自動切替（利用者が設定でオンにしたときだけ）は `web/src/app/state.ts` 311 行、新規セッションの自動切替は `web/src/app/ws-client.ts` 778 行（`pendingAutoSwitch`）
- 削除後の付け替え（`web/src/app.ts` 1868 行）は、タブを動かしたくない自動起点で、包まれていない唯一のもの

## 削除で別のセッションを開かない経路

| 事実 | 出典 |
|---|---|
| ペインの `onSessionRemoved` はレイアウトからそのセッションを外すだけで、別のセッションを開かない | `web/src/app/multi-pane.ts` 1171〜1196 行 |
| Files / Git タブの `onSessionRemoved` は、同じリポジトリの別セッションへタブの対象を付け替えるが、`activateSession` は呼ばない | `web/src/app/files-view.ts` 941〜970 行付近 |

## 検査

| 事実 | 出典 |
|---|---|
| `web/package.json` の `test` は `bun run build` のあと `node --test dist/app/…-fixtures.js …` を並べる。`dist/app/session-list-tab-fixtures.js` は**すでに列挙されている**。`web/tests/session-list-tabs-dom.test.ts` はこの列挙に入っていない（CI で走るかは未確認なので、この指示では編集しない） | `web/package.json` |
| 基準 SHA での手元の実行結果: `bun test ./src/app/session-list-tab-fixtures.ts` は 22 件通過、`bun test ./tests/session-list-tabs-dom.test.ts` は 14 件通過（`web` ディレクトリで、2026-10-20） | 手元の実行 |
| fixtures の流儀: `session(id, extra)` が `started_at` に `2026-10-08T01:00:NNZ`、`cwd` に `/repos/example` を入れる。`state()` が `normalize(null)` に `add(value, 'Work', 'work')` したものを返す。`move(s, sessions, id, 'work')` が `moveSessionListFamily` | `web/src/app/session-list-tab-fixtures.ts` 15〜17 行、1〜14 行 |
| 手元の測定では、`cwd` を `/repos/a`（#1・#2）・`/repos/b`（#3）・`/repos/c`（#4・#5）に分けると、別のプロジェクトとして 1,2,3,4,5 の順に並んだ。fixtures でも `session(id, { cwd })` で同じ入力を使い、期待値が出ることを実行して確かめる | 手元の測定。`web/src/app/sidebar-tree.ts` の `buildSidebarTree` |

## 測定（合成入力・画面ではない）

`sidebar-tree.ts` の `buildSidebarTree` / `flattenSidebarTree` と `session-list-tab-state.ts` の実関数を使い、`app.ts` 1823〜1827 行の式をそのまま再現した。プロジェクト a（#1・#2）→ Tab 1、プロジェクト b（#3）・c（#4・#5）→ Work、全体の並びは 1,2,3,4,5。

| 削除するセッション（所属） | 従来の式で開かれる隣（所属） | 結果 |
|---|---|---|
| #1（Tab 1） | #2（Tab 1） | タブはそのまま |
| #2（Tab 1） | #1（Tab 1） | タブはそのまま |
| #3（Work） | #2（Tab 1） | タブが切り替わる |
| #4（Work） | #3（Work） | タブはそのまま |
| #5（Work） | #4（Work） | タブはそのまま |

## 未確認

- 実画面（Hub を起動し、タブを 2 つ作って削除する操作）での見え方。dots には確かめられないので、手元の利用者が確認する
- 削除でタブの順序やセッションの所属が変わる経路は、読んだ範囲では見つかっていない（別の経路があれば、README の「止まる条件」3 として報告する）

## 設計上の約束（守る）

- 画面の JS を読み込んだ瞬間に、循環 import の相手の変数を同期で読まない（`CLAUDE.md` の設計原則の索引にある。`web/src/app/boot-guard.ts` の冒頭）。`app.ts` に import を足すときは、`node scripts/check-web-module-init.mjs` の趣旨に反しないか確かめる
- 本線は Rust 版（`develop`）。`go/` は凍結してあり、ビルド・改修・調査をしない
- 実装の進め方の詳しい規約は `CLAUDE.md` と `CLAUDE/coding.md`。**着手前に読む**
