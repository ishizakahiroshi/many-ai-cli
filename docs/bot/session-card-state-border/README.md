# #P-20261020-005 many-ai-cli セッションカード状態枠 実装指示

> 最終更新: 2026-10-20(火) 12:30:08

- 案件: #P-20261020-005 many-ai-cli セッションカード状態枠（GitHub の Issue 番号・PR 番号とは別）
- GitHub Issue: https://github.com/ishizakahiroshi/many-ai-cli/issues/29。送信・受付・提出・レビュー・手元検収の状態と報告は、この Issue のコメントに書く。
- 指示書の保存先: `ishizakahiroshi/many-ai-cli` の `docs/bot/session-card-state-border/`（公開 repo。公開前提の内容だけを置く）
- 作業対象 repo / PR base: `ishizakahiroshi/many-ai-cli` / `develop`
- 作業 branch（dots が作る）: `dots/P-20261020-005-session-card-state-border`
- 担当: 実装は dots。独立レビューは別に頼む（この指示書の範囲外）。実画面の確認とビルドは利用者が行う。

## 返信の書き方

返事は毎回この案件のスレッドの中だけに書く。受付・進捗・質問・停止・修正提出・完了の 1 行目は `#P-20261020-005 many-ai-cli：<報告内容>` にする。スレッドの外には書かない。Slack だけの報告は手元から見えないので、受付・提出・質問・停止・完了の各段で、必ず管理 Issue にも短いコメントを書く。

## 目的

サイドバーのセッションカードで、「承認待ち」と「未確認で停止」が流し見で気づけない。今は、承認待ちが薄い背景と左バー、未確認で停止が状態の丸の色だけで、利用者の認知の負荷が高い。この 2 つの状態のとき、カードの枠を橙・黄にして目立たせる。

見た目は、利用者が参考モック（同じフォルダの `mock.html`。ブラウザで開く）で選び終えている。この指示書はその選択を CSS と TypeScript に落とす。見た目の判断を足さない。

## 利用者が選んだ内容（変更しない）

- Q1 見せ方: 全周に枠。承認待ちは橙の枠がゆっくり光る。未確認で停止は黄の枠が静かに付く（動かない）。
- Q2 色: 承認待ち `#f97316`（Light テーマは `#ea580c`）、未確認で停止 `#facc15`（Light テーマは `#c48500`）。
- Q3 重なり: 状態の枠を優先する。カードに色ラベルがあるときは、その色を左バーに残す。選択中のカードが承認待ちのときは、橙の枠の外側に青い細い輪（`--accent`）を残す。
- 枠の太さ 2px。背景の色づけは、承認待ち 12%、未確認で停止 7%。光の広がり 10px。状態名の帯は付けない。

## 変更する範囲（この 3 ファイルだけ）

1. `web/src/styles.css`
2. `web/src/styles/session-list-tabs.css`（17 行目の 1 行だけ）
3. `web/src/app/session-list.ts`

`go/`、`rust/`、`web/dist/`、ほかの CSS・TS、`.github/` は触らない。

## 具体的な変更

### 1. 色の定義（`web/src/styles.css`）

Dark の `:root`（`--warn` `--yellow` の並び、74〜75 行付近）に追加する。

```css
--state-appr: #f97316;
--state-stop: #facc15;
```

`[data-theme="light"]`（`--warn` `--yellow` の並び、193〜194 行付近）に追加する。

```css
--state-appr: #ea580c;
--state-stop: #c48500;
```

カスタムテーマは、背景の色味だけを変えて警告色は土台の Dark / Light のままなので、追加の定義は要らない（`web/src/app/theme-tokens.ts` の冒頭の記述）。

### 2. カードに `stopped-unseen` クラスを付ける（`web/src/app/session-list.ts`）

- カードを作る箇所（1280〜1284 行付近の `approvalClass` と同じ並び）に、`activity.className === 'stopped-unseen' ? ' stopped-unseen' : ''` を足し、`c.className` の連結に入れる。
- 状態だけを更新する `updateSessionCardStateInPlace`（1830〜1831 行付近）に、`card.classList.toggle('stopped-unseen', activity.className === 'stopped-unseen');` を足す。
- `stateActivityDecoration` の判定は変えない。

### 3. 枠の CSS（`web/src/styles.css`）

現在の次の 2 規則を、下のブロックへ置き換える（重複を残さない）。

- 1321 行 `.card.awaiting-approval { background: ... }`
- 1358 行 `.card.awaiting-approval::before { background: var(--warn); ... }`

新しいブロックは、`.card.orchestration-parent`（1387 行付近）の直後に置く。親・子セッションの左ボーダーと色ラベルの枠色より後に置かないと、枠の色が負ける。

```css
/* 状態の枠: 承認待ち（橙・光る）と未確認で停止（黄・静か）。枠は 1px の border と 1px の内側の輪で 2px に見せ、カードの大きさを変えない */
.card.awaiting-approval { border-color: var(--state-appr); box-shadow: inset 0 0 0 1px var(--state-appr); background: color-mix(in srgb, var(--state-appr) 12%, var(--bg-elev)); }
.card.stopped-unseen { border-color: var(--state-stop); box-shadow: inset 0 0 0 1px var(--state-stop); background: color-mix(in srgb, var(--state-stop) 7%, var(--bg-elev)); }
.card.awaiting-approval::before { background: var(--state-appr); opacity: 1; width: 5px; }
.card.stopped-unseen::before { background: var(--state-stop); opacity: 1; }
/* 色ラベルは左バーに残す */
.card[class*="card-color-"].awaiting-approval::before,
.card[class*="card-color-"].stopped-unseen::before { background: var(--card-ident-color); }
/* 選択中で承認待ち: 橙の枠の外側に青い細い輪。左バーは選択中の青のまま */
.card.active.awaiting-approval { border-color: var(--state-appr); background: color-mix(in srgb, var(--state-appr) 12%, var(--bg-elev)); box-shadow: inset 0 0 0 1px var(--state-appr), 0 0 0 2px var(--accent); }
.card.active.awaiting-approval::before { background: var(--accent); opacity: 1; width: 5px; top: 3px; bottom: 3px; }
@media (prefers-reduced-motion: no-preference) {
  .card.awaiting-approval { animation: card-attention-glow 1.8s ease-in-out infinite; }
  .card.active.awaiting-approval { animation-name: card-attention-glow-active; }
  @keyframes card-attention-glow {
    0%, 100% { box-shadow: inset 0 0 0 1px var(--state-appr), 0 0 0 0 transparent; }
    50% { box-shadow: inset 0 0 0 1px var(--state-appr), 0 0 10px 1px color-mix(in srgb, var(--state-appr) 55%, transparent); }
  }
  @keyframes card-attention-glow-active {
    0%, 100% { box-shadow: inset 0 0 0 1px var(--state-appr), 0 0 0 2px var(--accent); }
    50% { box-shadow: inset 0 0 0 1px var(--state-appr), 0 0 0 2px var(--accent), 0 0 10px 3px color-mix(in srgb, var(--state-appr) 55%, transparent); }
  }
}
```

`.card:hover` の背景は、状態の枠が付いているカードでは状態の背景が勝ってよい（ホバーの専用表現は足さない）。

### 4. 状態の丸の色

- `web/src/styles.css` 1427 行 `.card-state-pill.awaiting-approval` の `background: var(--warn)` を `background: var(--state-appr)` に変える。ほかの指定（`color`・`border-radius`）は変えない。
- `web/src/styles/session-list-tabs.css` 17 行 `.card-state-pill.stopped-unseen { color: var(--yellow); }` を `color: var(--state-stop);` に変える。

### 触らないもの

- 子起動の確認待ち（`pending-spawn-confirm`、紫）と入力待ち（`awaiting-input`）。
- 上部のセッション帯（`session-strip.css`）、端末のライブ状態（`terminal.css`）、タブの数字の色。これらは別の判断（利用者の未決定事項）で、今回は旧色のまま残る。
- 状態を決める処理（`stateActivityDecoration`、`isSessionListStopped`）。

## 検証

dots の作業環境では依存を取得できないことがある。検査は、作業 branch から出す Draft PR の CI（Validate）で行う。

- 合否は、PR の head SHA の CI で決める。`web` の型検査（`bun run check`）と、リポジトリの Web 向け検査スクリプトの結果を、実行した範囲として書く。skip は成功に数えない。
- `updateSessionCardStateInPlace` と `stateActivityDecoration` を参照するテスト・fixture があれば、壊れていないことを書く。
- 作業環境で実行できなかった検査は「未実施」と書く。成功と言い換えない。

## 完了条件

- 上の 3 ファイルだけの変更で、PR の CI が通っている。
- `go/`・`rust/`・`web/dist/` に差分がない。
- 状態を決める処理に差分がない。
- 実画面の確認（暗い画面・明るい画面・カスタムテーマ）と、ビルドは、利用者が行う。dots は「実画面は未確認」と書く。

## 提出

- コードの変更は、作業 branch `dots/P-20261020-005-session-card-state-border` へ push し、`develop` 向けの Draft PR を出す。commit の末尾に `Agent: dot` を付ける。
- 提出の報告に、実装 commit の SHA、実行した検査のコマンドと結果、未実施の検査、戻し方（PR を閉じるだけ）を書く。
- コード以外の成果物は無い。Google Drive への提出は不要。
- 受付: 案件番号、読んだ指示 commit、環境で実行できる検査の範囲を、このスレッドへ返す。

## 許可と禁止

許可（書く）:
- 作業 branch `dots/P-20261020-005-session-card-state-border` への、上の 3 ファイルの変更の push。
- `develop` 向けの Draft PR の作成と更新。
- 管理 Issue へのコメント。
- 独立レビューの指摘の修正。

禁止（書かない）:
- `develop` / `main` への直接 push、merge、release、deploy。
- 上の 3 ファイル以外の変更。`.github/` の workflow の変更。
- 新しい依存の追加。認証情報の利用。
- 実業務の Issue・Project の作成や書き込み。

取り消し: 作業 branch・Draft PR への保存が取り消されたら、状態を確かめて 2 回まで送り直してよい。branch の作成・更新は先に作業 branch の head を読み、送る前のままなら送り直し、送ろうとした commit になっていれば次へ進み、どちらでもなければ止まる。force push はしない。Draft PR の作成は、同じ branch からの PR が無いことを確かめてから送り直す。3 回目も取り消されたとき、審査で拒否されたときは止まり、branch の head と PR の有無を報告する。「再試行しない」は、取り消された同じ操作だけを指す。
