# [完了] 障害対応記録: ミニマップにカーソルを当てるとティックが上部に詰まる

> 最終更新: 2026-05-18(月) 00:43:51

## 症状

チャット履歴ペインのミニマップ（右端の縦バー）にマウスカーソルを乗せると、
ミニマップのティック（各メッセージを表す小バー）が上部に詰め寄り、消えてしまう。

- ホバー前: ティックがミニマップ全体に均等分散して表示される
- ホバー後: ティック全体がミニマップの上部に収束し、下半分が空白になる
- カーソルを外すと元に戻る

## 根本原因（root cause）

`web/src/app.js` の `setupMinimapMagnification` 関数（修正前 10812 行）の `onMove` ハンドラ。

```javascript
// 修正前: すべてのティックに固定高さを設定
const BASE = 8;   // px
tk.style.height = (BASE + extra).toFixed(1) + 'px';
tk.style.flex = 'none';   // ← flex 分配から外す
```

`flex: 1`（コンテナ高さを等分）で均等配置されていたティックに対し、ホバー時に
`flex: none; height: 8px` を設定することで全ティックが flex 分配から外れる。

- 各ティックの高さ合計 = `N × 8px + (N-1) × 3px gap`（カーソル付近のみ最大 18px）
- ティック数が少ないと合計高さ < コンテナ高さになり、ティック群が上端に詰まる
- `flex-direction: column` のデフォルトは `justify-content: flex-start` なので先頭から積まれる

例: ティック 10 本 × 8px + 9 × 3px gap = 107px、コンテナ高さ 400px の場合、
ティック群は上部 107px に収まり残り 293px が空白になる。

## 修正内容

高さ固定 → `flex-grow` の比率調整に変更。合計高さが常にコンテナ高さと一致する。

```javascript
// 修正後: flex-grow だけ変える（height/flex は触らない）
const MAX_ADD = 4;
tk.style.flexGrow = (1 + extra).toFixed(2);

// onLeave も flexGrow だけリセット
tk.style.flexGrow = '';
```

カーソル付近のティックは `flex-grow` が最大 5（= 1 + 4）になり比例的に大きく見える。
遠いティックは `flex-grow ≈ 1` のまま。合計高さは常にコンテナ高さと一致するため
ティックが上部に詰まることがなくなる。

`web/src/styles.css` の transition も合わせて変更:

```css
/* 修正前 */
transition: height 0.08s ease, opacity 0.15s;

/* 修正後 */
transition: flex-grow 0.08s ease, opacity 0.15s;
```

## 変更ファイル

| ファイル | 内容 |
|---|---|
| `web/src/app.js` | `setupMinimapMagnification` の `onMove` / `onLeave` を `height`+`flex:none` → `flexGrow` 調整に変更、`BASE` 定数削除、`MAX_ADD` を 10 → 4 に変更 |
| `web/src/styles.css` | `.mm-tick` の `transition` を `height 0.08s` → `flex-grow 0.08s` に変更 |

## 検証

- <TODO: ミニマップにホバー → ティックが均等分散したままカーソル付近が拡大されることを確認>
- <TODO: カーソルを外すとティックが元の等分状態に戻ることを確認>
- <TODO: メッセージ数が少ない（5 件以下）場合でも上詰まりが起きないことを確認>
- <TODO: メッセージ数が多い（100 件以上）場合でもミニマップが崩れないことを確認>

## 備忘

- `flex-grow` アプローチにより合計高さは常にコンテナ高さと一致するため、
  `overflow-y: auto` / `overflow: hidden` どちらでも動作に影響しない
- `MAX_ADD = 4` は flex-grow の追加量（単位: 無次元）。カーソルのティックの
  flex-grow が最大 5 になり、遠いティックの 1 に対して約 5 倍の高さになる
- 元の `MAX_ADD = 10` は「ピクセル加算量」として使われていたが、
  flex-grow 単位で 10 を使うとカーソル付近のティックが極端に大きくなりすぎるため 4 に変更
