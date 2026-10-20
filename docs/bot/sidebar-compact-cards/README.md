---
type: reference
status: draft
tags: [ui, sidebar]
owner: user
related: [FACTS.md, REVIEW.md, PROGRESS.md, mock.html]
last_reviewed: 2026-10-20
---

# #P-20261020-004 many-ai-cli: sidebar compact cards

更新: 2026-10-20。承認済みの[動くモック](mock.html)に沿って、サイドバーのセッションカードに「コンパクト表示」を足し、選択中セッションのヘッダーへ「起動時の権限モード」と「ブランチ」を足す。[根拠](FACTS.md)・[独立レビュー指示](REVIEW.md)・[進捗看板](PROGRESS.md)も読む。コード中の行番号は基準 SHA 時点のもので、着手前に必ず実物を開いて確かめる。

## 対象と目的

- 作業 repo: `ishizakahiroshi/many-ai-cli`（公開）
- 作業 branch: `dots/P-20261020-004-sidebar-compact-cards`（この指示書もこの branch にある）
- Draft PR の base: `develop`（Rust 版の本線。Go 版は `go/` に凍結してあり触らない）
- 挙動の基準 SHA: `754f5f5782b1bb7762c3b730e73ce93f97113a8d`
- 管理 Issue: https://github.com/ishizakahiroshi/many-ai-cli/issues/27
- 実装 diff の起点は初回指示公開 commit `7390fd9f70da8b2b096e175d08ebe2d9a18e2d3b`（基準 SHA とは別）

1 つのタブに入るセッションが 8〜10 件で限界になる。今のカードは 2 段（1 段目: 状態・#番号・プロバイダ・モデル+題、2 段目: 状態情報・完了サマリー・補足チップ・残量・子トグル・ブランチ）でおよそ 55px あり、グループ見出しも 1 行ずつ取る。同じ縦幅に入る件数を増やして、承認待ち・完了・⚠ を探すスクロールを減らすのが目的。

## 承認済みの変更

1. 設定で「表示密度（標準 / コンパクト）」を選べるようにする。コンパクトでは **1 行カード**と**細いグループ見出し**にする。**既定は標準**で、標準表示のマークアップ・CSS は変えない。
2. 全セッションで同じ値になりがちで毎回並ぶだけの「起動時の権限モード」・ブランチ・1M は、コンパクトのカードから外す。
3. 外した「起動時の権限モード」と「ブランチ」は、選択中セッションのヘッダー（`#session-info-chip`）へ足す。**1M は足さない**（フッターのステータスバーに「ctx %」の隣で既に出ている）。
4. 「いつもと違う」セッションにだけ、カードに小さな ● を出す。カーソルを 0.4 秒止めると詳細ポップが出る。

## 既定として置いた判断（D1〜D6）

承認時に未決定だったものを、おすすめどおりに置いてある。迷って止まらず、この既定で進める。

| # | 既定 |
|---|---|
| D1 | 「いつもと違う」の基準は、**同じプロジェクトグループ内で最も多い値**。グループ = `renderSessionList` が組む project ノードに入る全セッション（親と子を含む）。権限モードの空（フラグなしで起動）は「指定なし」という 1 つの値として数える。ブランチの空（git 外）も 1 つの値として数える。同数で最頻値が決められないときは、値が複数あるぶん全員にマークを出す。1 件だけのグループは基準＝自分自身なのでマークなし |
| D2 | 標準表示のカードは変えない |
| D3 | 既定は標準。選択は**端末ごと**（`localStorage` のみ。サーバー同期しない。Hub 側 Rust の設定項目も足さない） |
| D4 | ポップはカーソルを 0.4 秒止めたときだけ出し、外れたら即消す。下の行は押し下げず、重ねて出す。0.4 秒は既存の `web/src/app/project-group-path-popover.ts`（`SHOW_DELAY_MS = 400`）と同じ値で、同じ作法に倣う |
| D5 | ホバーできない環境ではポップを出さない（選択すればヘッダーで権限・ブランチが読める） |
| D6 | コンパクトで 1 行に出さない補足（メモ・役割チップ・headless・worktree・引き継ぎ・ボード更新待ち）と、D1 で「いつもと違う」権限・ブランチは、**まとめて 1 つの ●** にし、中身はポップに全部列挙する |

## 実装の単位（順に C1 → C2 → C3 → C4）

共有ファイルの持ち主は 1 担当。並列にしてよいのは読み取りだけの調査。

### C1: 「いつもと違う」判定の純関数と fixtures（画面に触らない）

- 新規 `web/src/app/session-card-notes.ts`。**import を持たない純関数**にする（node:test から DOM なしで検証するため。既存の `web/src/app/session-strip-branch.ts` と同じ切り分け。循環 import での初期化順の事故を避ける意味もある）
- `computeBaselines(sessionsInGroup)`: グループ内の権限モード・ブランチそれぞれの最頻値を返す。同数で決まらないときは「基準なし」を返す（D1）
- `collectCardNotes(session, baselines)`: そのセッションの ● に載せる項目を `{ kind, labelKey, value }` の配列で返す。含めるのは、権限・ブランチが基準と違うとき、`note` / `role` / `execution_mode === 'headless'` / `worktree_branch` / `handoff_from` / 後継（呼び出し側が求めた ID を引数で受ける）/ `board_notify_pending`。配列が空なら ● を出さない、という判断をこの関数だけが持つ
- fixtures `web/src/app/session-card-notes-fixtures.ts`: 1 件だけのグループ / 全員同じ値 / 1 人だけ違う / 同数で割れる / 空と値が混ざる / git 外だけのグループ / 補足だけあるセッション
- `web/package.json` の `test` の列挙へ `dist/app/session-card-notes-fixtures.js` を足す

### C2: ヘッダーへ「起動時の権限モード」と「ブランチ」を足す

- `web/src/app/settings.ts` の `renderSessionInfoChip` の契約バッジの後ろへ 2 つ足す
  1. 権限モード: 既存の `permissionModeChipHtml(s.permission_mode)`（`web/src/app/session-list.ts`）を再利用する。文言は今のまま「起動時: 〜」で、**起動時の値であってライブ値ではない**ことを言い切る文言を変えない。空なら出さない。ヘッダー内の文字サイズは `web/src/styles/tabs.css` の `.session-info-chip …` の並びで合わせる
  2. ブランチ: `abbreviateBranchName(s.branch)`（`web/src/app/session-strip-branch.ts`）の短縮表示。全文は `data-tooltip`。git 外（`s.branch` が空）のときはセッション帯と同じく出さない。クリックで Git タブを開く。**処理を複製せず、カードの `.card-branch` が使う経路を共通化して両方から呼ぶ**（`web/src/app/session-list.ts` の 963〜1002 行付近を先に読む）
- 狭いときの扱い: タブバーは `overflow-x: auto`、チップは `flex-shrink: 0`。ブランチは最大幅を決めて省略する
- 値が変わったときにヘッダーが追従することを確かめる: `web/src/app/ws-client.ts` の 632 行付近（`cur.branch = m.branch`）と、`renderSessionInfoChip` の呼び出し 3 か所（`session-list.ts` の 222・304・1563 行付近）を読み、追従しない経路があれば呼び出しを足す
- i18n は `web/src/i18n/ja.json` / `en.json` / `vi.json` の 3 つへ同時に足す。既存の `card_branch_tooltip` / `card_branch_no_git` を再利用できるなら新規キーを足さない

### C3: コンパクト表示（1 行カード・細い見出し・詳細ポップ）と設定の切替

- **実装方式（着手時に決める・おすすめは B）**: A = 標準のマークアップを CSS だけで 1 行に畳む / B = コンパクト用の行を別関数で組む。**B をおすすめ。** 標準のマークアップに触らないので、標準の見た目が変わらない保証になる。B でも、部分更新が探すクラス名（`.card-done-slot` ほか。`session-list.ts` 749 行付近と、同ファイルの `querySelector` を全部洗う）は同名のスロットで残す。決めた方式と理由は管理 Issue のコメントに 3 行で書く
- 1 行に出すもの: 状態アイコン・#番号・プロバイダ・題（モデル名つき）・完了サマリー（待機中のみ。名前の右に薄い字。残り幅で省略）・状態情報（⚠ / 終了理由 / ⚙ のどれか 1 つ）・メモ件数・🔕・子トグル（▼ n）・●（`collectCardNotes` が空でないときだけ）・残量バー+%（小）
- 1 行に出さないもの: 1M・起動時の権限・ブランチ・補足チップ（ポップで読む）。⋯ と × は右端に重ねて出し、幅を取らない（ホバー・フォーカス時のみ。キーボードのフォーカスでも操作できること）
- グループ見出し: 高さを詰め、件数の 3 つの数字は残す。▦ ★ × はホバー時のみ。★ が付いているときは ★ だけ常時
- 詳細ポップ: `project-group-path-popover.ts` の作法（400ms 遅延・出ていれば即時）。中身はモデル名・題の全文・完了サマリー全文・残量・● の項目。コンパクトのときは、カード自身の `c.dataset.tooltip`（`session-list.ts` 1292 行付近）を外して二重表示を避ける。再描画・スクロール・カーソルが外れたときに、遅延タイマーと表示中のポップを必ず片付ける
- 設定: `web/src/index.html` の「文字サイズ」の行（`#fontsize-select`）の並びに「表示密度（標準 / コンパクト）」を足す。保存は `localStorage` のみ（D3）で、`web/src/app/user-prefs.ts` に新しい `STORAGE_` 定数を足す。**サーバー同期の対応表 `_USER_PREFS_PATH_TO_LS`（239 行付近）には載せない**。起動時の反映は `settings.ts` の即時関数（`applyFontSize(...)` の並び、1280 行付近）。`#sessions` に `density-compact` クラスを付け外しし、切替後に `renderSessionList()` を呼ぶ
- i18n は 3 ファイルへ同時に足す

### C4: 検査・CHANGELOG

- `CHANGELOG.md` の `## [Unreleased]` に、C2 と C3 の利用者向けの変更を 1 行ずつ。書式は直近の項目に合わせ、書く前に 1 項目読む
- 設定画面の説明が `README.md` に載っていれば、1 行足す。無ければ足さない
- PR の CI（`Validate`・`doxguard`・`secret-scan`）の結果を、実行された step 名と結果で報告する（skip は成功に数えない）

## 境界（触らないもの）

- 標準表示のカードのマークアップ・CSS・挙動
- Rust 側（`rust/`）、`go/`（凍結）、サーバー同期する設定項目、サイドバーの並び替え・グルーピング規則、フッター（`web/src/app/token-statusbar.ts`）、セッション帯（`web/src/app/session-strip.ts`）
- CI workflow、依存の追加、`web/dist/`（生成物）

## 検証

- dots の作業環境では依存を取得できないことがある。**取得できないまま検査が通ったことにしない。** できなかった検査は「未実施」と書き、PR の CI を根拠にする
- 手元でやってほしい静的検査（できれば）: `web` ディレクトリで `bun run check`（`tsc --noEmit`）、リポジトリ直下で `node scripts/check-ui-components.mjs`、`node scripts/check-web-module-init.mjs`、`node scripts/check-text-hygiene.mjs`
- PR の `Validate` は、frontend の型検査・UI 部品検査・モジュール初期化検査・ビルド・fixtures を走らせる。**fixtures（C1 のもの）は `bun run test` の中で、`web/package.json` の列挙に載っているものだけが走る**。列挙し忘れると走らずに緑になるので、PR の CI ログで `session-card-notes-fixtures` の実行を確かめる
- 利用者の Windows 実機・稼働 Hub・実画面での見た目（行の高さが揃うか・ポップが下の行を動かさないか・ヘッダーが約 960px の幅で見切れないか）は dots には確かめられない。**手元の利用者が確認する。** dots の完了報告で「画面で確認した」と書かない

## 完了条件

- C1〜C4 の作業が終わり、PR の CI が実行範囲つきで緑
- 設定で「コンパクト」を選ぶと 1 行カード・細い見出しになり、「標準」へ戻すと今の見た目に戻る。**標準の DOM・CSS の差分が無い**（`git diff` の対象ファイルで示す）
- 起動時の権限・ブランチが基準と違うセッションにだけ ● が出て、ポップにその中身が出る（fixtures と同じ判定関数を通る）
- 選択中セッションのヘッダーで、起動時の権限モードとブランチが読める。ブランチのクリックがカードの `.card-branch` と同じ経路を通る
- 完了サマリーが、標準で出ていたセッションと同じ条件（待機中のみ）でコンパクトにも出る
- i18n 3 ファイルに同じキーが揃っている
- 独立レビュー（REVIEW.md）の指摘が直り、**最終の code SHA** を対象に再レビューされている
- 未実施のもの（実機の見た目・ビルドの実行）は、未実施と書いてある

## 提出

- コードは作業 branch に push し、`develop` 向けの Draft PR にする。報告に code SHA・実行した command と exit code・残件・rollback の方法を書く
- 管理 Issue に、受付・着手・各 C の完了・レビュー・停止・質問・最終提出のたびに短いコメントを書く（**Slack だけの報告は手元で検知されない**）
- `PROGRESS.md` は手元が更新する。dots は書かない
- merge・release はしない

## 受付と返信の書き方

- 最初に同じ案件のスレッドへ、認識した案件番号・読んだ指示の commit・実行環境（できる検査とできない検査）を返す
- 返事は毎回この案件のスレッドの中だけに書き、受付・進捗・質問・停止・修正提出・完了の 1 行目を `#P-20261020-004 many-ai-cli：<報告内容>` にする。スレッドの外に書かない
- 進行中の補足は再起動の依頼ではない。読んだ revision を返す

## 許可と禁止

**許可（書く）**

- 作業 branch `dots/P-20261020-004-sidebar-compact-cards` へのコード・テスト・文書の commit と push
- `develop` 向けの Draft PR の作成と更新
- 管理 Issue へのコメント
- 独立レビューの指摘の修正
- `CHANGELOG.md` の Unreleased と、該当すれば `README.md` の更新
- 作業環境で使える既存の道具による静的な検証

**禁止（書かない）**

- `main` / `develop` 等の実 base への直接 push、merge、release、deploy
- 稼働中の Hub の起動・停止・再読み込み、実アカウントでの確認、認証情報・秘密値・Hub のトークンの扱い
- `go/`、`rust/` の変更、新しい依存・外部サービス、CI workflow の変更
- 無関係な branch・Issue・Project への書き込み
- 私有情報の記載。fixture は合成データだけにする

## 取り消されたとき

- 作業 branch・Draft PR への保存が取り消されたら、状態を確かめてから 2 回まで送り直してよい。branch を動かす前の blob・tree・commit の作成は、そのまま送り直す。branch の作成と更新は先に作業 branch の head を読み、送る前のまま（作成ならまだ無い）なら送り直し、送ろうとした commit になっていれば通っていたとみなして次へ進み、どちらでもなければ止まる。force push はしない。Draft PR の作成は、同じ branch からの PR が既にないかを確かめてから送り直す
- 3 回目も取り消されたら止まり、base の commit SHA と `git format-patch` の出力を Google Drive の `bot/blob/2026/10/P-20261020-004/v1/` に置く（直下とその配下のすべてのサブフォルダへのファイルの保存とフォルダの作成を許可する。`manifest.json` に相対パス・bytes・SHA256 を書き、最後に保存する）。手元が当てて Draft PR にする
- 審査で拒否されたときは Drive にも回さずに止まり、どこまで通ったか（branch の head・PR の有無）を報告する
- 「再試行しない」は、取り消された同じ操作だけを指す

## 止まる条件

次のどれかに当たったら止まり、管理 Issue にコメントして、同じスレッドへ報告する。

1. 同じ理由の CI 失敗が 2 回続く
2. この指示の変更範囲の外を触る必要が出た
3. 指示と実コードの食い違い（例: 書いてあるファイルや関数が無い）が見つかった
4. 重大なレビュー指摘が 2 回直しても残る
5. 持ち主の操作が要る（GitHub の確認画面・CI が始まらない）
6. 1 工程が 90 分を超える
