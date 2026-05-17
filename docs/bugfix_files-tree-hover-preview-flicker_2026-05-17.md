# [完了] 障害対応記録: Files ツリーの画像ホバープレビューが2種類出てチカチカする

> 最終更新: 2026-05-17(日) 23:08:20

## 症状

Files タブの左ペイン（ツリー）で画像ファイルにマウスを乗せると、ホバープレビューが2つ同時に DOM に存在し、マウス移動のたびに前面に来るものが切り替わってチカチカして見える。

- 再現条件: ツリーが一度でも再描画された後（フィルタ入力・ファイル一覧更新など）に画像ファイルへ hover
- 影響範囲: Files タブを持つ全セッション、画像・動画ファイルが含まれるプロジェクト

## 根本原因（root cause）

`web/src/app.js` の `FilesTreeView` IIFE 内、`renderTree` 関数（旧 11215 行）の **ローカル変数** として `hoverPreviewEl` / `hoverPreviewTimer` が宣言されていた。

```
FilesTreeView IIFE
└─ renderTree(treeRoot, opts)          ← 再描画のたびに呼ばれる
     let hoverPreviewEl = null         ← ここ（ローカル）
     let hoverPreviewTimer = null      ← ここ（ローカル）
     function hideHoverPreview() { ... }
     function positionHoverPreview(e) { ... }
     function showHoverPreview(node, e) { ... }
```

`renderTree` が呼ばれるたびに新しい closure が生成される。この closure が持つ
`hideHoverPreview` はその呼び出し分の `hoverPreviewEl` しか操作できない。

ツリーが再描画されると：

1. **旧 closure の `hoverPreviewEl`**（`document.body` に append 済みの div）は誰も除去しないまま残る
2. 旧 item 要素のイベントリスナーは DOM から消えるため旧 `hideHoverPreview` は呼ばれない
3. 新 closure で `showHoverPreview` が動くと新 preview div が `document.body` に追加される
4. 旧・新の2つの `.files-image-hover-preview` が同時に DOM に存在

Z-index や描画タイミングで前後が入れ替わり、チカチカとして現れた。

## 修正内容

`hoverPreviewEl` / `hoverPreviewTimer` / `hideHoverPreview` / `positionHoverPreview` を
`renderTree` の外（IIFE スコープ）に移動し、`renderTree` 冒頭で `hideHoverPreview()` を呼ぶ。

**before（renderTree 内ローカル）:**
```javascript
function renderTree(treeRoot, opts) {
  ...
  let hoverPreviewEl = null;        // renderTree ローカル
  let hoverPreviewTimer = null;

  function hideHoverPreview() { ... }
  function positionHoverPreview(e) { ... }
  function showHoverPreview(node, e) { ... }
```

**after（IIFE スコープに移動）:**
```javascript
// FilesTreeView IIFE 直下（renderTree の外）
let hoverPreviewEl = null;
let hoverPreviewTimer = null;

function hideHoverPreview() { ... }
function positionHoverPreview(e) { ... }

function renderTree(treeRoot, opts) {
  ...
  hideHoverPreview();  // 再描画前に必ず前回の preview を除去

  function showHoverPreview(node, e) { ... }  // sessionId を使うためここは残す
```

## 変更ファイル

| ファイル | 内容 |
|---|---|
| `web/src/app.js` | `hoverPreviewEl` / `hoverPreviewTimer` / `hideHoverPreview` / `positionHoverPreview` を IIFE スコープに移動。`renderTree` 冒頭に `hideHoverPreview()` 追加 |

## 検証

- <TODO: 再ビルド後、Files タブで画像ファイルにホバーし、フィルタ入力でツリーを再描画させてから再度ホバーしてもチカチカが起きないことを確認>
- <TODO: 複数回 renderTree が呼ばれる操作（検索フィルタ変更・ファイル一覧リフレッシュ）後でも hover preview が1つだけ表示されることを確認>
- <TODO: mouseleave で preview が正常に消えることを確認>

## 備忘

`document.body` に append するフローティング要素（tooltip・hover preview 等）は、
それを管理する変数・関数が **呼び出しごとに再生成される closure のローカルスコープ**
に置かれると同種の問題が起きる。管理変数はコンポーネント（IIFE）スコープに置くのが原則。
