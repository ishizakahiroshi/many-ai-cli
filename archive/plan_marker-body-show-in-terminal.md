---
type: plan
status: done
tags: []
owner: 
review_status: draft
related: []
last_reviewed: 2026-07-04
---
# [完了] 承認マーカー本文をターミナルにも表示する（案 B → 案 C → 案 E）

## context配分

| 章 | 種別 | 内容 |
|---|---|---|
| C1 | plan | 経緯・案 B の振り返り・案 C 採用判断 |
| C2 | fix  | hub-marker-filter.ts の本文 pass-through 化＋テスト書き換え |
| C3 | plan | INK popup 衝突の再発監視と、再発時の切替先（案 2/案 3） |
| C4 | fix  | 案 E 採用：マーカーブロック内の全 ANSI 剥離（review-sheet で決定） |

## C1. 経緯と方針判断

- 2026-06-23 案 B で「OPEN..CLOSE で囲まれた承認マーカーブロックは xterm に描画しない」を採用（Claude Code の INK popup が絶対カーソル位置指定で xterm スクロールと衝突して文字化けする問題の根本対処）
- 2026-06-24 ユーザー指摘: ターミナル側に質問本文が出ないと、AI が何を聞いてきたのか CLI を見ても読めない（popup は出るが体験が分断する）
- 提示案:
  1. [タグ除去] OPEN/CLOSE タグだけ削って本文は流す（INK 衝突の再発リスクあり）
  2. [provider判定] Claude Code のときだけ案 B のまま、それ以外は案 1
  3. [プレースホルダ] xterm には「↓ 確認パネル」1 行だけ
- ユーザー選択: **案 1（案 C と命名）**

## C2. 実装

### 変更内容

- `web/src/app/hub-marker-filter.ts`
  - 冒頭コメントの「案 B」記述を「案 C（2026-06-24）」に書き換え。経緯・トレードオフを明記
  - `filterHubMarkersPure` のループで、`inMarker` / `inDone` 中の本文バイトを `out` に流すよう変更（従来は捨てていた）
  - OPEN/CLOSE のマーカー文字列自体は引き続き剥がす
  - close 後の `\x1b[J`（OPEN 到達前に部分流出した popup 残骸の掃除）は残置
- `web/src/app/hub-marker-filter-fixtures.ts`
  - 全テストを案 C 期待値に書き換え（本文が `out` に残る・タグだけ消える）
  - 「絶対カーソル位置指定もブロック内なら剥がれる」テストは「本文と一緒に xterm に流れる（案 C のトレードオフ）」に意図変更

### 検証

- `make build`（ユーザー実行）→ ブラウザリロードで承認マーカー応答時にターミナル本文が表示されるか確認
- popup 側の選択動作・PTY 返送が壊れていないこと（`pendingTextTail` 経路は触っていないので維持されるはず）

## C3. INK popup 衝突の再発監視

### 監視ポイント

案 B で潰した「Claude Code の INK popup と承認マーカーが同時発生したとき、ブロック本文中の絶対カーソル位置指定（`\x1b[24;3H` 等）が xterm の画面高オーバーフローと衝突して文字化け」が再発し得る。

- 発火条件: Claude Code セッションで AI 応答中に INK popup が描画される + 同応答中に承認マーカーブロックが含まれる
- 観察手段: 承認応答時にターミナル本文・action-bar とも崩れないかを目視確認

### 再発時の切替先

- 案 2（provider 判定）: Claude Code セッションのみ案 B、それ以外は案 C
- 案 3（プレースホルダ）: 「↓ 確認パネルを見てください」1 行のみ表示し本文は popup 側に集約

切替は `filterHubMarkersPure` の caller（`terminal.ts:filterHubMarkersForDisplay`）でセッション provider を見て分岐するか、`filterHubMarkersPure` のシグネチャに mode 引数を足す。

## C4. 案 E への移行（2026-06-24）

### 決定経緯

- C3 で「INK 衝突が実害として出たら案 D/案 2 などへ切替」としていたが、ユーザーから「ホバー化で根本解決できないか」の追加質問
- 案 A（action-bar のホバー化）は 2026-06-23 時点で検証済みで効かない（マーカー本文は AI が PTY に流す生バイト。action-bar の DOM 配置と関係なく xterm に届く）
- そこから「Web 側フィルタ層で衝突を生む ESC シーケンスだけ剥がせば根本解決」と整理し、検討シート `docs/local/marker-ansi-cleanup_review.html` を作成して意思決定
- 決定: A1=案 E（全 ANSI 剥離） / A2=MANY-AI-CLI と DONE 両方に適用 / B1=テスト追加 / B2=本 plan に C4 追記

### 実装

`web/src/app/hub-marker-filter.ts`:

- マーカー本文を直接 out に push する案 C 実装を変更し、`markerBuf` / `doneBuf` を state に追加してブロック本文を buf に貯める
- CLOSE 到達時に buf を UTF-8 decode → `stripAnsiFromString` で ANSI 剥離 → encode して out に push
- `stripAnsiFromString` は OSC（`\x1b]…BEL|ESC\`）/ CSI（`\x1b[…final`）/ ESC+単一バイト / 裸 ESC を順に剥がす
- chunk 跨ぎ ANSI も buf に累積されるので CLOSE 時に完全な形で剥離できる

`web/src/app/terminal.ts`:

- `filterHubMarkersForDisplay` の state 受け渡しに `markerBuf` / `doneBuf` を追加

`web/src/app/hub-marker-filter-fixtures.ts`:

- `stripAnsiFromString` 単体テスト追加（CSI / OSC / 単一 ESC / 裸 ESC を網羅）
- ブロック内の絶対カーソル位置指定が剥がれることを assert（旧案 C テスト「本文と一緒に xterm に流れる」を反転）
- ブロック内 SGR 色指定も剥がれること
- chunk 跨ぎ ANSI（`\x1b[24` + `;3H`）が正しく剥がれること
- DONE ブロックの ANSI も剥がれること
- ブロック外の ANSI は素通し（プロンプト等の通常装飾は保持）

### 検証

- `make build`（ユーザー実行）→ ブラウザリロードで多選択肢の `[MANY-AI-CLI]` を出させて
  1. ターミナルに質問本文が出ること（案 C 時点で達成済み）
  2. INK popup の絶対カーソル位置指定が混在していても画面崩れしないこと（案 E で新規達成）
  3. action-bar の選択肢ボタンと PTY 返送が壊れていないこと

### 案 E のトレードオフ

- マーカーブロック内の色情報・強調等は失う。現状そこに色は付いていないので実害なし
- ブロック「外」の AI 応答地の文の ANSI は触らない（プロンプト・thinking 表示の色は保持）
- 副作用として案 C で残っていた INK 衝突再発リスクが原理的に消失
