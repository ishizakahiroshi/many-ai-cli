# [完了] 障害対応記録: サイドバー上部のボタン配置が意図と逆（New Session 左・停止右）

> 最終更新: 2026-05-18(月) 00:43:51

## 症状

`#new-session-bar` のボタン順が「+ New Session（左）→ 停止ボタン（右）」になっており、
操作上は停止ボタンが右端に孤立した形になっていた。

- `+ New Session` ボタンが左側に幅いっぱい広がり、停止ボタンが右端に押し出されている
- 停止ボタンを先に目にしてほしいという UI 上の優先度と逆の配置

## 根本原因（root cause）

`web/src/index.html` の `#new-session-bar` 内で `#new-session-btn` を先に、
`#kill-all-btn` を後に記述していたため、flex レイアウト上で左→右の順に並んでいた（395〜401 行）。

## 修正内容

DOM の記述順を入れ替え、停止ボタンを先に置くよう変更。

before:
```html
<button id="new-session-btn" ...>+ New Session</button>
<button id="kill-all-btn" ...> ... </button>
```

after:
```html
<button id="kill-all-btn" ...> ... </button>
<button id="new-session-btn" ...>+ New Session</button>
```

`#new-session-btn` は `flex: 1` で伸長するため、DOM 順の変更だけで
「停止（左）→ New Session（右）」配置になる。CSS 変更なし。

## 変更ファイル

| ファイル | 内容 |
|---|---|
| `web/src/index.html` | `#new-session-bar` 内のボタン記述順を入れ替え（395〜401 行） |

## 検証

- <TODO: 再ビルド後にサイドバー上部を確認し、停止ボタンが左・New Session ボタンが右に表示されることを確認>
- <TODO: New Session ボタンが残りの横幅いっぱいに広がっていることを確認>
- <TODO: 停止ボタンのクリックで全セッション終了が動作することを確認>
