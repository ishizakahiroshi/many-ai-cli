---
type: reference
status: stable
tags: [ui, components, reuse]
owner:
review_status: reviewed
related: []
last_reviewed: 2026-10-11
---

# Web UI部品と確認文言の共通化

> 最終更新: 2026-10-13(火) 12:56:37 — アイコンだけの操作の列に使う `ui-toolbar` を追記
> 2026-10-11(日) 20:58:29

Rustが同梱するWeb画面は、同じ意味の操作に同じ外観を使う。新しい画面を作る前に、
`web/src/styles/ui-components.css` と `web/src/app/ui-components.ts` を確認する。
画面のCSSへボタンや確認枠の色・文字サイズ・角丸・影を再定義しない。
画面側は配置とその画面固有の意味を持ち、イベント・通信・focus・後始末も引き続き所有する。

## 既存テーマと部品の段階

背景・面・文字・境界・危険・警告には既存のCSS変数を使う。新しい意味がなければ
同じ値の変数を増やさない。カスタムテーマはsurfaceを変え、accent/provider/warningは
light/darkの土台を保つ（`theme-tokens.ts`）。主ボタンは専用foregroundと暗めのaccentを
使い、surfaceの本文色をそのままaccent背景へ置かない。

| 部品 | 使い分け | 使用しない場面 |
|---|---|---|
| primary | 一つの主要な安全操作。保存・起動 | 削除・停止、状態badge |
| secondary | 主操作と並ぶ否定・補助操作 | 本文リンク、状態表示 |
| danger | 停止・削除等、失われる結果がある実行 | 通常の保存、単なる強調 |
| quiet | コピー・補助説明等、繰り返す行内操作 | 重大な実行、説明のないicon |
| warn | 既存の警告を伴う実行 | 危険操作、警告表示だけのbadge |
| modal sm/md/lg | 短い確認/通常フォーム/情報量の多い枠 | drawer、局所popover、専用認証画面 |
| input | 通常のtext/select/textarea | terminal、IMEや認証プロトコルの再設計 |
| general icon | close/copy/edit/delete/refresh/check/stop | provider画像・session状態iconの代替 |

ボタンは `uiButtonClass(variant, size, options)`、枠は `uiModalClass(size)` で
classを供給する。静的HTMLでは同じclassを直接付ける。既存classはDOM探索・配置・
意味のある状態に必要なら併用する。`ui-button--compact` は32px、regularは36px、
coarse pointerでは44px以上の操作領域を確保する。アイコン形状は色の第6variantではない。
独立したclose位置・底寄せsheetは専用配置を保ち、共通icon操作の領域は32/44pxに揃える。

常に見えているアイコンだけの操作の列（サイドバー上端など）は、親に `ui-toolbar` を付ける。
列の中のボタンは塗りと横の余白を持たず、variantの意味はアイコンの色で表す。primaryは列に1つまで
（accentの薄い面）、dangerは危険色のアイコン、その他はsecondary。塗りのprimaryを並べると列が色の帯になる。

専用配置の寸法も `--ui-control-size` を参照し、compact/regular/coarseの切替に追従する。
通常入力は `ui-input` を付け、narrow/coarseでは既存 `--fs-lg` の16pxを使って
iOSのfocus zoomを避ける。terminalやcode editor、PIN認証専用の入力は用途を保つ。

modalは420/620/840pxを上限とし、左右16pxずつのviewport余白を確保する。
`ui-modal__title` / `ui-modal__body` / `ui-modal__actions` を使う。
native dialogのtop-layer、overlayのz-index、局所popoverの位置は呼出し側で保持する。
全画面overlayの `.aac-wheel-overlay` と局所popupの `data-wheel-native` を維持する。

SVGは `uiIconHtml(name, size)` を使い、currentColorに追従させる。SVG自体はdecorativeで、
外側のbuttonへaccessible nameを持たせる。既存 `providerIconHtml` / `stateIconSvgHtml` は
意味が異なる供給元としてそのまま使う。hoverはmouse向け、keyboardはfocus-visibleを
表示し、hoverだけに必要情報を置かない。

footerは `--ui-footer-*` の専用paletteを使う。provider、session状態、Git diff/graph、
terminal/codeの配色を一般の主操作色へ寄せない。

## 「しますか？」の確認と操作変数

`confirmationLabels(action, vars?)` が `{ question, decline, accept }` を返す。
呼出し側はupdate/delete/save/stop等の操作キーを渡す。日本語のサ変操作は
`{action}しますか？ / {action}しない / {action}する` という共通テンプレートを使う。
例えばupdateは **更新しますか？ → 更新しない / 更新する**。

否定を先、実行を後に置く。`appConfirm` の `action` を使う場合は汎用のOK/cancelを
明示指定せず、共通文言を使う。titleを省略すれば共通questionを使い、対象名・件数・
取り消せない結果は本文へ残す。本文から動詞や疑問文を自動抽出しない。
close/continue等の非サ変はlocale別の平坦な3キーを使う。英語・ベトナム語もlocale
辞書で生成し、日本語の接尾辞をJSで連結しない。文言をHTMLへ挿入する場合はescapeする。

Hub停止は **停止しない / すべて停止する / Hubのみ停止する** の3択を保つ。
null/sessions/shutdown、独立checkbox、停止APIの組み合わせを2択に潰さない。
編集へ戻る、継続/UTF-8の選択、認証情報を残して登録解除する等の別経路、入力phraseを
要求する確認は、その意味を示す文言を保つ。否定のfalseが必ず無操作とは限らない。
CLI由来の承認候補・番号・回答payloadは変更しない。

文言と外観の変更でEsc/外側/Enter/初期focus/復帰/listener解除を変更しない。
`window.confirm` fallbackのOS標準ボタン文言は変更できないため、既存fallbackを保持する。

## 機械検査とレビュー

`node scripts/check-ui-components.mjs` はCSSとliteral inline styleの色・font-size、
共通ファイル外のcontrol外観、新しいactionなしappConfirm、literal DOM style代入を検査する。
CSSのquote/comment/括弧を読み、既存のpinned esbuildでも構文だけを解析して
警告・不正なselector/at-rule・未閉鎖・余分なbraceを失敗にする。bundleや製品資材は生成しない。
方向別border、named color、font shorthand、literal style.cssTextも検査する。
theme/component/footerの正本以外でcustom propertyへ生の色を上書きすることも検出する。
`scripts/ui-components-exceptions.json` は既存例外のexact fingerprint・最大出現数・理由を持つ。
ファイル全体の除外ではなく、selector/condition/property/valueが変われば再レビューする。
削除済みの例外はstaleとして報告する。新しい例外を自動採番・自動追加する運用はしない。

これはCSS engineやJS dataflowの完全解析ではない。動的style生成、classの実効cascade、
runtimeのDOM配置と送信payloadは変更元で別にレビューする。
ValidateのWeb jobで型検査、UI検査、guardのnegative testsを実行する。
`node --test scripts/check-ui-components.test.mjs` と
`bun test ./tests/ui-components.test.ts` はビルドせずに走る。

画面変更の検収ではdark/light/custom、keyboard、狭幅/touch、modalの操作と返値、
footer/sticky controlsの到達性を実画面で確認する。静的検査だけで実画面合格とはしない。
