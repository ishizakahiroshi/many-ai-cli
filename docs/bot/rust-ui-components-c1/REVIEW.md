---
type: reference
status: draft
tags: [rust, ui, review]
owner: independent-reviewer
review_status: draft
related: [README.md, FACTS.md, PROGRESS.md]
last_reviewed: 2026-10-11
---

# #P-20261011-005 many-ai-cli：C1独立レビュー手順

> 最終更新: 2026-10-11(日) 19:38:10 JST

実施記録は末尾。前半のレビュー手順を保持する。

## 担当と対象

調査担当とは別の担当が、調査基準SHAと成果SHAを直接読む。最初の指示公開commitから成果headまでの全差分を確認する。レビュー担当の名前/実行識別子、基準SHA、成果SHA、実施したコマンドと結果、未実施を記録する。モデルIDは取得できなければunknownとし、推測しない。

## 確認すること

1. 変更が本フォルダの資料と許可された再集計スクリプトだけか。hookが必要とする索引の機械再生成がある場合、文書追加と生成metadataに由来する差分だけか。製品source、workflow、i18n、lockfileなどを変えていないか。
2. メイン・サイドバー・タブ・モーダル・設定・モバイル・分離表示の入口と実装CSSが名簿にあるか。五軸すべてに種類数と相対パス・行の代表例があるか。
3. 種類数はクラス名の数で代用していないか。単位・対象リスト・除外・正規化・重複排除が説明され、同じSHAから再測定できるか。
4. cascade・継承・テーマ・media条件を無視していないか。`var()` のfallback、alpha違い、動的inline style、HTMLのSVG、絵文字・文字記号も対象としているか。静的に確定できないものを「実画面で確認済み」にしていないか。
5. 実際の相違が二例以上の値で示され、危険操作・セッション状態・provider識別などの意図した差を区別しているか。既存のトークン、`close-button.css` の共通部品と左寄せ設定を見落としていないか。
6. `appConfirmShutdown` の描画元、呼出元、CSS、fallback、三つの戻り値、外部公開停止checkbox、Esc・外側クリック・Enter・初期focus・listener後始末を名簿にできているか。
7. C2の利用者判断を確定していないか。各画面の確認手順と期待状態が具体的か。dots提出と手元追加差分の確認、source検査と実画面受入が別の状態か。
8. 依存・外部通信・実Hub起動なしで再集計できるか。GitHub CIの発火・結果・skipを混同していないか。秘密・私有会話URL・個人ホームの絶対パスが無いか。

## 結果の残し方

findingごとに重要度、ファイル・行、根拠、満たしていない要求、修正案を書く。修正後は新しい成果SHAで再レビューする。合格なら未確認も列挙し、「C1 source棚卸し合格」と書く。実画面や親計画全体の合格へ広げない。

成果とレビューのSHA、担当、結果を管理Issueコメントと同じ案件会話へ報告する。手元担当はその報告と実headを照合し、別製品のAIとして差分全行・計数を読み直す。レビュー指示書を作っただけでは独立レビュー完了にならない。

## 独立レビュー第1巡と修正候補

- 担当: dots review_rust_ui_inventory（調査担当 inventory_rust_ui_components と別context）。モデルIDは取得できず unknown。
- source: e1ab622aa7708ae883a3cdd83d042fbf2cf31e38。
- 対象成果SHA: 61daa7f312c397192f8afefd402203954e3a1889、tree 67b972cf327e0b4ca0fb6f8031a0e327f26bcbe7。
- diff起点: fad045cba803e7827d31cdaddcb4f333f1f39f32。既存指示追記から成果まで全差分を確認。成果3ファイルは固定Git blobとローカルbytes/hash一致。
- 結果: request changes（文書のsource参照・特殊操作契約・cascade記述の修正）。製品source修正の指摘ではない。

提出用指摘要約と対応:

1. appLegacyResetNoticeのEsc/外側/cancel不可、showApprovalToastのopt-in操作、subscriptions confirmStackingConsentのcheckbox必須/Enter暗黙確定なしが一般confirmと区別されていなかった。特殊subcaseと操作名簿へ追記。
2. PINはsettings.css、pane ph-closeはtabs.css/pane-placement.css、mobile approval sheet/mas-closeはmobile-home.cssが主CSS。参照を訂正。
3. spawn-confirm狭幅ではwidth変更後もbase max-widthが残り、max-height=min(90dvh,760px)。mtl-closeはcoarse時min-height44pxが重なる。cascade制約を追記。
4. slash-pickerを端末入力の子面に明記。S41行のMarkdown表を修正。
5. CSS再集計がJSの閉じ方を調べたと読める記述を訂正。source操作補助表と未検証の細部を分離。

独立実行結果: script self-check6成功/0失敗/0skip、固定source218件のmanifest全行・CSS4種類の全値一覧/計数が再集計と一致。差分範囲は許可folderのみ、whitespaceエラーなし。既存共通部品/停止契約に修正必須指摘なし。source git.cssのEOF未閉鎖warning2件も独立確認。

未確認: 修正候補の新成果SHAでの再レビュー、Windows/実画面/手元追加差分。修正済みという調査担当の記述だけで合格へ変更しない。再レビュー対象SHAと結論は次の巡およびIssue16へ追記する。

## 第2巡の指摘と修正

対象成果SHA: 7f80b94fee3cd6fb46b839dc3fa9e27b6a76fd1a。同じ独立担当が4blobとローカルhash一致、修正diffとsourceを直接確認。self-check6成功、誤refをexit1で拒否する合成確認も期待一致。

|重要度|対象と根拠|指摘/修正|
|---|---|---|
|P2|第1巡 INVENTORY S17/§3.3/§3.5、settings.ts:52–112/711–754、subscriptions.ts:251–303|特殊close/checkbox/Enter契約の欠落。特殊subcase＋補助名簿に追記済み|
|P2|第1巡 INVENTORY S25/B07/B08/S38/P07|主要CSS参照の誤りをsettings.css/tabs.css/pane-placement.css/mobile-home.cssへ修正済み|
|P2|第1巡 INVENTORY P03/B08、spawn-confirm.css:317–323、styles.css:1305–1314|狭幅max-width残留/coarse min-heightの条件を追記済み|
|P2|第1巡 INVENTORY §3.3|CSS再集計でJS閉じ方を確認したかのような記述を訂正、21行source操作補助名簿と未検証を分離|
|P3|第1巡 INVENTORY S41|Markdown表の途中空行を削除済み。slash-pickerはS05子面へ追記|
|P2|第2巡 INVENTORY §3.4/§3.5、session-list.ts:371–389|既存stateIconSvgHtml共有7種が未記載。provider spriteと別の共有helper/利用元を追記|
|P2|第2巡 INVENTORY close補助S40、detached-grid-launcher.ts:338–351|起動後ではなくclose後に起動処理。外側イベントもmousedownへ訂正|
|P3|第2巡 INVENTORY close補助S33、shortcut-help.ts:54/66|close表示文字はEsc、外側mousedownもあり。B07 paneは常時表示と明確化|

全指摘は文書のみ修正。修正した新SHAは独立再レビュー待ち（この時点で合格とはしない）。計数scriptおよびsourceは不変。

## 第3巡の停止と追加依頼の範囲

54c4002938124c8f49d21cc07a80e8eaa004d605の最終固定差分読取は実行審査cancel、同じ読取の許可根拠を提示した1回の再試行もcancelとなり停止した。別経路で読み直して合格扱いにはしていない。7f80b94までの独立確認と、54c4002以降の最終確認待ちを区別する。

利用者の追加依頼によりINVENTORY §3.1.1へ代表12行の直書き色/候補変数/既定dark-light値/保留理由を追記。196色の全件分類やC2採用決定ではない。この追補も独立レビュー未完了。成果の保存成功はレビュー合格と別である。
