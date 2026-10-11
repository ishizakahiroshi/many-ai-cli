---
type: reference
status: draft
tags: [rust, ui, inventory, c1]
owner: dots-inventory
review_status: draft
related: [README.md, FACTS.md, REVIEW.md, PROGRESS.md]
last_reviewed: 2026-10-11
---

# #P-20261011-005 many-ai-cli：Rust UI C1 source棚卸し

> 最終更新: 2026-10-11(日) 19:38:10 JST

**実画面は未確認。これは固定公開sourceの棚卸しであり、見た目の乱れ・動作合格・C2仕様決定ではない。**

## 1. 基準・方法・再現

- 調査source: `e1ab622aa7708ae883a3cdd83d042fbf2cf31e38`。受付時（2026-10-11）の `dots/rust-recovery-resume-3` 先端も同じ。利用者の未公開commit/未commit変更は含まない。
- 指示4文書: `6f24b79bcd5544bca5b28d2c8a7c6b98ff8ae2b6`。レビューdiff起点: `fad045cba803e7827d31cdaddcb4f333f1f39f32`。
- `CLAUDE.md`、`CLAUDE/development.md`、`CLAUDE/operations.md` を読了。本依頼の限定書込み許可を優先。source変更、製品build、Hub操作、Go追加調査はしていない。
- `.omitnix/index.json` は入口として確認。generated.commit=`a114727651bdca2884c3d4b92262bbae240f6b2c`、dirty=true。coverage: discovered1966/analyzed1526/unresolved9/unknown1/unclaimed430。JSONやPNGのunclaimedを未使用とは解釈しない。索引の古い値を集計には使わない。
- 指定glob 215ファイル（HTML1/CSS38/TS173〔fixture34含む〕/localeJSON3）に、入口を確かめる `app.ts` / `app-entry.ts` / `i18n.ts` 3件を追加して218件を固定。全パス・bytes・SHA256は末尾。`git show`のバイトをUTF-8全読取するためNULのあるapproval-queue-tab.tsも落とさない。
- 追加読取は既存wheel検査script、上記運用文書/索引、READMEの仕様、workflow/hook。これらはUI値の母集団に含めない。vendor、debug、sw、types、画像バイナリ、Rust/Goサーバ本体は値計数対象外（理由: UI部品の自前定義と第三者資産/通信型/サーバ処理を混ぜない）。vendor CSSが実画面へ影響すること自体は除外していない。

再集計（既存Python3とGitだけ。ネットワーク、依存導入、Hub、ビルド不要。scriptはstdout以外へ書かない）:

```sh
python3 docs/bot/rust-ui-components-c1/recount_ui.py --self-test
python3 docs/bot/rust-ui-components-c1/recount_ui.py --repo . --ref e1ab622aa7708ae883a3cdd83d042fbf2cf31e38 > c1-counts.json
```

`--repo`はこのSHAを持つGit checkout。scriptはworktreeを読まず固定git objectを読む。違うSHAは拒否する。`c1-counts.json`は再生成する利用者側の一時出力で、今回の追加提出ファイルではない。

### 計数の単位と限界

1. CSSはコメントを空白化して行番号を保ち、文字列・括弧内の区切りを保護し、@media/@supports/@container/@keyframesを記録する。色は宣言値中のhex/rgb(a)、変数はvar()の参照名、文字はfont-size宣言値。小文字化・rgb内空白除去だけを行い、`#fff`と`#ffffff`、alpha違い、単位違い、fallback違いは同値と決めつけない。`var(--x, fallback)`の色fallbackも数えるが、実際に採用されたとは言わない。
2. 全sourceの字句集計は別欄。コメント・fixture・locale・SVG属性・TSテンプレートの候補を含むため「画面で使用された色数」ではない。CSS値集計との合算は禁止。
3. button/overlay/input候補の宣言ベクトルは背景・枠・文字・寸法・余白・角丸・focus/opacity等の値の並びで重複排除する。セレクタ名の数ではない。ただし1部品のbase/hover/theme/mediaが別ruleになり、複数部品が1ruleを共有する。**ベクトル数は最終的なUI部品種類数ではない**。状態の結合やcascadeを解いた画面上の種類数は未確認。
4. selector名のbutton/btn/chip/pill/close等で候補抽出するため、名称に出ないクリック可能領域や非button pillの混入があり得る。下の手動名簿が役割・操作を補う。自作scannerはCSS/TSの完全パーサー・ブラウザ・到達性解析ではない。CSSの`font` shorthand内サイズ、JS計算で作る色やSVGは自動font-size/hexの全列挙にならず、下の動的名簿で別記する。
5. `git.css:899`のmediaと`:911`の`.git-commit-actions`がEOFで未閉鎖（最終912行）。宣言の字句収集はEOFを終端としてwarning2件を出す。ファイル修復は本件範囲外。ブラウザの回復解析後の見た目は未確認。件数からこの注意を隠さない。

## 2. 画面の名簿（画面群41、同じDOM部品を別画面として水増ししない）

各行は入口/描画責務の異なる画面群。設定内24セクション、9ビュータブ、モーダル内の子ボタンをこの41に加算しない。CSS欄の `styles.css` は共通トークン/全体規則を併用する意味。各sourceは固定SHA。末尾の表示条件確認表を全群へ適用する。

|ID|画面群・入口と描画元|主なCSS/条件・継承|
|---|---|---|
|S01|主shell/ヘッダ/sidebar、index.html:27/1227/1745、app.ts|styles.css、session-list.ts、sidebar-tree.ts。セッション状態でhidden/activeが変化|
|S02|セッション一覧とタブ/編集、index.html:1279/1612、session-list-tabs.ts:344、session-list.ts:216|session-list-tabs.css、styles.css。native dialog編集、選択/drag/長押しは別状態|
|S03|上部セッションstrip、index.html:1779、session-strip.ts:215/468|session-strip.css。project/scope/グリップ、幅条件|
|S04|メインビュー選択9タブ、index.html:1751–1759、settings.ts setActiveTab|tabs.css、styles.css。terminal/multi/chat/files/git/review/orchestration/approval/history|
|S05|端末と入力/action-bar/添付/音声、terminal.ts、attachments.ts、voice.ts/voice-whisper.ts。slash-pickerはindex.html:1864、app.ts:2649–2755、styles.css:2009–2098の子面|terminal.css/voice.css、styles.css。xterm vendor、IME、入力フォント、mobile条件|
|S06|マルチペイン/配置メニュー、multi-pane.ts、flexible-pane-views.ts:40、pane-placement-menu.ts:121|terminal.css/pane-placement.css/tabs.css。既存DOMを移す、複製しない|
|S07|チャット/生ログモーダル、chat-history.ts:1347、index.html:1809|chat.css、styles.css、highlight vendor。raw overlay別状態|
|S08|Files/プレビュー/移動dialog、files-view.ts:1911/1982、index.html:1827|files.css。画像・動画・音声・Markdown/コードを子表示として持つ|
|S09|Git graph/作業ツリー/commit詳細、git-view.ts:72/115、index.html:1830|git.css。lane色/差分色は意味を保持、760px規則EOF注意|
|S10|Review、review-view.ts、FilesTabManager mount、index.html:1756|review.css。Files/Gitと同時複製不可|
|S11|Orchestration/relayカード/役割、orchestration-dashboard.ts:183/406/492|styles.css、workflow-progress.css。状態とsubagent木の色を保持|
|S12|承認タブ/履歴、approval-queue-tab.ts:121/220、index.html:1833|approval-queue-tab.css/approval.css。NULを含むsourceを全読取|
|S13|承認popup/dock/実行内容確認、approval.ts:1585、approval-ui.ts、approval-dock.ts|approval.css。pending/answeredと候補のidentity契約、背面転送抑止|
|S14|セッション履歴一覧、history-lite.ts:75/101、index.html:1834|history-lite.css。検索/空/loading状態|
|S15|端末履歴viewer、history-viewer.ts:215|terminal.css、styles.css。offset・ページ取得と閉じ方は別契約|
|S16|設定画面24セクション、index.html:41/65–982、settings.ts|settings.css/各機能CSS。section一覧は下記。配色/theme/close-leftもここで選ぶ|
|S17|汎用確認/入力/危険操作/停止、settings.ts: appConfirm系/818|settings.css:871–965、spawn.css:576。div overlay、typed/warn/dangerを混同しない|
|S18|新規セッション/モデルpicker/作業folder、index.html:1285/2141、spawn-panel.ts:2211|spawn.css。モデルpickerのoverlayをS17と共用、内容は置換|
|S19|子起動承認、spawn-confirm.ts:251/616、ws-client.ts呼出|spawn-confirm.css。native dialog top-layer、520px以下width指定変更。ただしbase max-widthが残る|
|S20|CLI保守/確認/ログ、cli-maintenance.ts:429/593、index.html:343|cli-maintenance.css。2種類のnative dialog、状態表示|
|S21|Provider登録/履歴、index.html:1027/1113、provider-manager.ts:116/provider-manager-view.ts|settings.css、provider-icon.ts。独自icon画像/色入力を含む|
|S22|Subscription/usage詳細・確認、subscriptions.ts:137/420、usage-panel.ts:686/847|subscriptions.css、styles.css。popoverとprobe確認overlayを分ける|
|S23|Server/remote profiles、index.html:1135、server-modal.ts:446|server-modal.css。refresh/接続状態/close、操作契約変更なし|
|S24|外部公開popover/接続案内、host-expose.ts:263、mobile-connect.ts:281|host-expose.css/mobile-connect.css。QR/consent/wizardは同一機能の段階|
|S25|PINログイン、index.html:2144、remote-auth.ts:118|settings.css:584–639（styles.cssのimport）。認証動作・tokenは今回操作しない|
|S26|Workflow進捗pill/modal、workflow-modal.ts:391/449/510|workflow-progress.css。pill.dismissと単独closeを同一化しない|
|S27|引継ぎ通知/一覧/dialog、handoff.ts:100/187/240|styles.css/close-button.css。本文を無条件取得する機能へ広げない|
|S28|Relay/派生起動、relay-dialog.ts:514、derive-dialog.ts:200|styles.css/spawn.css。別フォーム、幅/入力/危険操作を確認|
|S29|Routine選択/実行、routines.ts:67|routines.css。native dialog、opener保持、履歴とフォーム|
|S30|Memo drawer/別窓、memo-panel.ts:387、window-channel.ts|memo.css。非modal/data-wheel-native、本文・添付、手元の未公開改良は対象外|
|S31|Prompt templates picker、prompt-templates.ts:82、shared-template-sync.ts|prompt-templates.css。provider/filter/送信前確認の意味を保持|
|S32|Search palette、session-search-palette.ts|session-search-palette.css。検索/選択/focus/close|
|S33|ショートカット/初回tour/空画面、shortcut-help.ts:94、first-run-tour.ts:57、zero-session-empty-state.ts|shortcut-help.css/first-run-tour.css/zero-session.css。入口別の補助表示|
|S34|About/bug report、index.html:1968、bug-report-modal.ts:242/396|styles.css/bug-report.css。report送信は実行しない|
|S35|Lightbox/ファイルmodal/端末expand、lightbox.ts:4、path-links.ts:283、expand-popup.ts:65|styles.css/files.css/terminal.css。全画面overlayと部分popupのwheel契約は別|
|S36|token/cost footer・送信履歴/詳細、token-statusbar.ts:354/750/1106|token-statusbar.css。意図した自前パレット、close例外|
|S37|Mobile home/セッションdrawer、mobile-home.ts:420/504、index.html:1227|mobile-home.css。720px/coarse、デスクトップとは別タップ領域|
|S38|Mobile承認専用/シート、mobile-approval-only.ts:114、mobile-approval-sheet.ts:76/145|mobile-approval-only.css/mobile-home.css:540–625。sheet背面はwheel検査の明示除外|
|S39|Mobile端末lite/詳細/short nudge、mobile-terminal-lite.ts:79/141、mobile-transcript.ts、mobile-short-nudge.ts|mobile-home.css/mobile-short-nudge.css。transcriptは描画材料で独立画面と数えない|
|S40|分離タブ/分離grid/launcher、detached-view.ts:269/393、detached-grid.ts:706、detached-grid-launcher.ts:271|detached-view.css/detached-grid.css、各元viewCSSを継承。follow/選択/Hubへ戻る|
|S41|起動失敗帯/Reload、boot-guard.ts:118–156|Object.assign(style)のinline #7f1d1d/#fff/#ef4444、font14px/1.5、z2147483647。通常theme外、Reloadは実行しない|

S16設定内の24セクション（index.htmlのdetails[data-section]を重複排除）:
general、appearance、token-statusbar、session、server-connections、subscriptions、ai-providers、nvidia-nim、jev、remote-auth、doctor、log、danger、approval-hub、approval-patterns、voice、trigger、file-open、notify-sound、quick-cmd、input-tools、deferred-enter、usage-links、slash-src。

未追跡の入口: serverが返す動的provider/icon画像、実設定依存の追加provider/remote、ユーザー独自theme、実際の分離窓/ブラウザ拡張・OSの入力ダイアログ。sourceの描画経路は追跡したが、環境固有の内容/到達性/最終スタイルを取得していない。vendor xterm内部DOMを自作UI部品の型数へ含めない。

## 3. 五軸の分類と状態

### 3.1 色・文字・段階

役割は7群（surface、本文/補助、accent/選択、危険/警告、セッション状態、provider識別、Git/コード/計測）。これは色値の種類数ではなく手動分類。トークン定義はstyles.css:47–223、カスタムtheme生成はtheme-tokens.ts:166/224。font-size既定段階は7（2xs .68、xs .72、sm .78、md .85、body .9、lg1、xl1.1 rem、styles.css:139–145）。header/statusbarは.72rem別alias、chatは13px、端末には利用者設定がある。半透明surface、fallback、providerの意味色は単純置換しない。

|代表|source上の値|判断|
|---|---|---|
|主surface|styles.css:50 `--bg:#0e0f12`、:52 `--bg-elev:#15171c`、light側:167以降|dark/lightの意図した差。統一対象は意味aliasの使い方|
|主文字/補助|styles.css:63 `--text:#e2e8f0`、:68 `--muted:#8994a3`|本文/補助の役割を保持。表示コントラストは実測待ち|
|footer固有|token-statusbar.css:155 branch `#bb9af7`、:156 git `#73daca`、:169 ok `#9ece6a`|自前計測パレットとして存在。global token化するかC2判断|
|状態/識別|styles.css:88–106 badge/provider変数、live-status-color.ts:43/176|意味を失う一色化はしない。動的inlineも最終値へ影響|

### 3.2 ボタン・入力の宣言profile

主要な形状/状態の手動分類は下表12類型。クラスの数を12と呼んでいない。背景/枠/文字/寸法/余白/角丸/状態の組が異なる実例を並べる。類型内にはサイズ・theme差があり、最終統合数はC2で決める。全候補宣言ベクトルは再集計JSONにselector/行/context/全採用値で出る。

|類型|実在例・値/状態|残す理由またはC2候補|
|---|---|---|
|B01 通常確認|settings.css:928 `.confirm-btn`: transparent/line/text、6px13px、radius-md、fs-sm。hover line-soft/bg-elev|キャンセル/副操作|
|B02 危険確認|settings.css:951 `.confirm-btn.primary`: danger-bg/danger、bold700、hover danger-bg-hover|「primary」が危険色である既存命名。青primaryへ機械置換しない|
|B03 警告確認|settings.css:958 `.confirm-btn.warn`: warn-bg/warn-text、font600、styles.css:2660 hover warn/bg|警告の意味を保持|
|B04 通常起動|spawn.css:562 launch accent系、spawn-confirm.css:308 is-primary accent/#fff、hover brightness1.08|B02と同じprimary語でも役割/値が違う|
|B05 起動副操作|spawn-confirm.css:290: minheight control-h-sm、6px14px、muted、fs-md、focus ring box-shadow|B01と余白/文字/高さ/focusが異なる|
|B06 単独閉じる|close-button.css:24–94: transparent/none/muted、radius-sm、.9rem、2px6px、hover danger-bg/danger|17selector共有。寸法変数の個別overrideあり|
|B07 tab/pane内閉じる|styles.css:2607 `.main-tab-close`、tabs.css:114–122 `.ph-close`＋pane-placement.css:81–105（後者は常時opacity1等を上書き）|tab内とpane内の別寸法/文脈。paneは後続CSSで常時opacity1。B06対象外|
|B08 mobile閉じる/選択|mobile-home.css `.mtl-detail-close`、mobile-home.css:540–625 `.mas-close`|mtl height36px/mas height44pxのsource指定。coarseではstyles.css:1305–1314 min-height44pxも適用されるためmtl実tap領域を36px固定とはしない|
|B09 設定header列|settings.css:38 `.settings-header-btn`、:53 close|隣接列の枠/余白を共有、closeだけB06へ剥がさない|
|B10 action確認|approval.css:713–723 back/go: transparent/line/muted-2 と accent/#fff|承認payload表示と実行契約を維持|
|B11 自前footer操作|token-statusbar.css:283/315/341: 固有色、hover #2a2f45、focus #7aa2f7|計測footerとしての意図差。theme統合は要判断|
|B12 chip/pill/状態切替|styles.css usage/chip、tabs.css view-tab、workflow-progress.css pill|選択/進捗と通常実行ボタンの違いを保持|

入力: settings.css:548 text（bg/text/line/radius-sm、focus accent）とspawn.css:600 model-picker（7px9px、radius-md、fs-md）、settings.css:388 select、terminal.css入力、files/review検索、provider色/画像入力を分ける。checkboxは危険確認の選択値と同義ではない。全てにbase/hover/active/disabled/focus-visible/checkedの存在を確認する観点を持ち、未指定状態はUA/継承に委ねられるため「状態が無い」とは断定しない。

### 3.3 モーダル・重なり面

操作機構の7種: (M1) hidden切替div全画面overlay、(M2) native dialog/showModal top-layer、(M3) anchored popover、(M4) 非modal drawer/pane、(M5) mobile sheet/背面mask、(M6) 別window、(M7) browser-native confirm/prompt/alert（CSS制御外）。**7は幅や色の種類数ではない**。幅/枠/閉じ方/暗幕を組にした代表profileは以下8件。全241候補ruleの176宣言ベクトルは別の単位。

|profile|機構・幅/枠/暗幕|閉じ方・状態確認点|
|---|---|---|
|P01 停止/汎用confirm|M1、min420px/100%、radius-md、padding14、line枠、rgba黒.5、z9100|Esc/外側/ボタン。停止のEnterはshutdown。詳細§4|
|P02 model picker|M1、min460px/100%、radius-lg、padding12、同overlay|検索/選択、S17とoverlay共用、同時表示を仮定しない|
|P03 spawn-confirm|M2、min620px/calc100vw-32px、radius-lg、padding0、backdrop黒.62|showModal/cancel/結果。520px以下width=calc(100vw - 16px)だがmax-width=calc(100vw - 32px)はbaseから残る。max-height=min(90dvh,760px)。実際のused幅は制約を併せて決まる|
|P04 CLI保守|M2、cli-maintenance.css:75/89、spawnに揃えたline/radius-lg/黒.62|確認とログは別dialog、実行中disable等を維持|
|P05 action-confirm|M1、approval.css:670–723|payload確認→戻る/実行、他の確認へ無条件置換しない|
|P06 memo|M4、memo.css、data-wheel-native|開いたまま端末操作。modal化しない|
|P07 mobile承認sheet|M5、mobile-home.css:540–625、mobile-approval-sheet.ts:76|tap/ドラフト抑制/自動表示、wheel例外は既存規則|
|P08 detachedタブ/grid|M6、detached-view.css/detached-grid.css|別window/follow/元画面復帰、overlay z-indexと比較不可|

S17の特殊subcase（通常confirmと同一契約にしない）:

- appLegacyResetNotice（settings.ts:711–754、呼出app.ts:2420）: 同じconfirm-dialog/overlayを使うが、✕/Cancel/外側clickによるcloseを提供せず、EscはcaptureでpreventDefault/stopPropagation。実行buttonの初期focus、3つのcheckbox値を返す。overlay不在のnullと通常のcancelを混同しない。sourceコメントにはEscapeでnullと残る箇所があるが、実装上の遮断を優先して記録する。これは改修依頼ではない。
- showApprovalToast（settings.ts:52–112）: checkApprovalOnStartupから条件付き表示。aac-wheel-overlay内の承認機能opt-in案内、yes/laterの2操作とyes初期focus。yesはdisabled後enable API、laterはdismiss API、いずれも要素をremove。単なる自動消滅toastとも、汎用Esc/外側cancel付きconfirmとも数えない。承認機能は実際に有効化していない。

server/bug-report/handoff/routine/relay/files/lightbox/shortcut/help等の個別宣言値は再集計profileに残す。閉じ方は下のsource補助名簿で扱い、CSS計数がJS契約も調べたとはしない。これら8代表を全種類の完全数とはしない。別DOM階層のz-index数字だけで前後を保証しない。native dialogのtop-layerは通常overlayのz-indexより別機構。

### 3.4 アイコン

表現は5種: (I1) HTML/TS inline SVG、(I2) provider共有symbol/use sprite、(I3) img/利用者provider画像、(I4) Unicode記号・emoji、(I5) CSS形状/色dot。SVGの形状違いと表現方式の5種を混ぜない。実寸/線幅/塗り/stroke/currentColor/文字fallbackの組で比較する。

- index.html:1751–1759 view-tab SVGは24×24 viewBoxの図形。新規/メモ/ルーチンsidebarは同:1257–1272でstroke1.8、orchestration:1757はstroke2。線幅差はsource上の事実、画面で気になるか未確認。
- provider-icon.ts:48の定義表、:171 symbol生成、:222 sprite、:311 presentation、:372 providerIconHtmlを既存共有部品として再利用候補にする。画像/テキスト/色カスタム、動的CSS変数、ロード結果でfallbackが変わる。
- 状態SVGはsession-list.ts:371–389の7種辞書とstateIconSvgHtmlで既に共有済み（terminal/session-strip/workflow/CLI保守）。SVG literal1個にテンプレートで各図形を差すため、字句SVG候補37を全アイコン種類数とは読まない。
- `✕`の単独close、`↪`引継ぎ、`⊞`分離grid、`!`危険、emoji `📁`など。字形・色emojiはOS/fontによって違う。Unicode生文字計数はコメント内の矢印等も含むためアイコン実個数にはしない。
- CSSでdot/pseudo-elementや`currentColor`を使う場合、hex0件でも無色ではない。SVGのpath/url画像を画面未確認のまま「重複icon削除」対象にしない。

### 3.5 流用の仕組み

既存流用機構は7群（R1トークン/別名、R2close共通selector、R3provider sprite/状態SVG helper、R4確認関数群、R5wheel除外契約、R6既存view DOMの移設、R7状態/設定の共通store）。欠如ではなく適用範囲を調べる段階。

- R1 styles.css:47–223、theme-tokens.ts:224、custom-themes.ts。root inline setPropertyがstylesheetより優先する。varの近似置換案は推論であり採用ではない。
- R2 close-button.cssの17共通selector、寸法2変数override、body.close-left。除外: ph/main-tab、mas/mtl、cwd-dropdown、wf-pill-dismiss、枠付きbutton列、tsb。about/handoff-notifyはstyles.css本体の後続個別ルールが残る。
- R3 provider-icon.tsのsymbol/use、provider色変数、およびsession-list.ts:371–389のSTATE_ICON_SVG/stateIconSvgHtml。状態SVGは16×16 viewBox・14×14出力の7種（flag/keyboard/dot/ring/cross/check/spawnPending、未知kindはdot）、provider spriteと別の既存共用helper。terminal.ts:52/2098、session-strip.ts:30/201、workflow-modal.ts:23/541/564、cli-maintenance.ts:27/334が使う。静的SVGの共有化案は未知の利用者画像を壊さないことが条件。
- R4 settings.tsのappConfirm/appConfirmTypedDanger/appConfirmShutdown/encoding確認。native spawn-confirmと同じAPIではない。
- R5 terminal.ts:159 `isModalOverlayOpen`と:1192 data-wheel-native。scripts/check-wheel-overlays.mjsはviewport fixed単純selectorを検査し、mobile sheet/部分popover/複合selectorには限界がある。
- R6 flexible-pane-views.ts:14/40のsingleRootsとFilesTabManager.mountPaneContent、detached表示。DOM IDを複製せずstate契約を保つ。
- R7 ui-side.ts:94 body class、user-prefs、theme/store。見た目の共通化が保存値や承認/送信契約を書き換えてはならない。

### 閉じ方・focusの補助名簿（source確認）

CSS再集計はJSの閉じ方を出力しない。以下は別途sourceを読んだ契約で、ブラウザでの動作結果ではない。明示focusが無い所のUA focusや全てのボタンの連打競合は未検証。

|画面/描画元|close/操作・後始末・初期focus|
|---|---|
|S22 subscriptions.ts:251–303|stacking同意はcheckbox未選択でOK disabled、changeで有効化しclickでもchecked確認。Enterの暗黙確定を実装せず、Esc/外側/cancelはfalse。checkboxをsetTimeout focus、同意時のみlocalStorage、listener除去/hidden/内容消去。overlay不在はwindow.confirm fallbackで別経路|
|S19 spawn-confirm.ts:423–616|open時Escは閉じるだけで拒否送信しない。deciding中cancelをpreventDefault、HTTP200後WS未達のfallbackでのみ解除可。type=buttonで暗黙submitを避ける。closeはcleanup/controller/timer終了。showModalのUA focusは未実測|
|S20 cli-maintenance.ts:429–475/593–612|確認cancelと実行、ログclose。native close eventでremove。実処理は実行していない|
|S23 server-modal.ts:446–488|✕、box外mousedown/touchstart、Esc capture(preventDefault/stopPropagation)。hidden、connectAbort、key/down listener除去。open関数に明示初期focus無し|
|S24 mobile-connect.ts:1087–1138|✕、box外（openerは除外）、Esc。最初のinput/buttonへfocus、closeでopenerへ戻す。reveal=false、snapshot消去、timer/visibility/key/down handler除去。秘密表示状態を残さない|
|S24 host-expose.ts:306–339|Esc/外側popover dismiss、key/down handler除去とDOM remove。設定操作自体は別契約で未実行|
|S25 remote-auth.ts:118/195–211|PIN input focus、EnterはpreventDefaultしてsubmitPINLogin。成功/失敗表示の認証経路であり、汎用cancel付きdialogとして扱わない|
|S26 workflow-modal.ts:485–520|✕/Esc/box外mousedown/touchstart/pill再click。pill自身は外側handlerから除外、DOMとlistenersを除去|
|S27 handoff.ts:187–205/380|✕/Esc/外側、DOM/listener除去、close button初期focus（一覧検索のfocusも後にある）|
|S28 relay-dialog.ts:475–549|✕/cancel/外側mousedown/Esc、prefs保存・generation進行・current session消去・hidden。planInput focus|
|S28 derive-dialog.ts:211–218/655–694|cancel/Esc/外側、keydown除去/DOM remove。promptArea focus。保存/開始actionは別処理|
|S29 routines.ts:52–69|native dialog.close、close eventでgeneration進行/timer停止/opener.focus。showModal、browserのEscと外側クリックは同一仕様ではない|
|S30 memo-panel.ts:300–301/483–506|drawer closeはhidden、入力focus。本文編集Escはfinish(false)で編集中止。非modal、画面全体のEscで他UIと同じ閉鎖を保証しない|
|S32 session-search-palette.ts:513/526–539/599–611|外側mousedown/Escでclose、検索input focus、↑↓で候補移動、Enterは選択実行または検索。Ctrl/Cmd+Kでopen|
|S33 shortcut-help.ts:48–67/94–117|表示文字Escのclose button/外側mousedown/`?`再入力でhidden。typing targetや他overlay表示中は起動抑制。Esc汎用閉鎖を仮定しない|
|S33 first-run-tour.ts:73–151|✕/Esc/外側はcomplete、laterは別処理。keydown除去、opener focus、次へbutton focus。閉じると後で見るを混同しない|
|S34 bug-report-modal.ts:76–95/337–393|✕/cancel/外側/keydown handler、DOM/参照/body class等を後始末、symptom focus。submit disableには未入力/log preview待ち条件、送信未実施|
|S35 lightbox.ts:22–29|Esc/外側でDOM/key listener除去。画像内部clickは同一ではない|
|S35 path-links.ts:228–327|✕/外側/Esc capture、file modal DOMとkey handler除去。pop-outは別操作|
|S40 detached-grid-launcher.ts:335–349|✕/overlay自身mousedownでremove。起動button押下はclose()→await launchDetachedPresetの順で、起動処理に先行してremove。共通Esc実装を想定しない|

未確認として残す細部: 各状態の実focus順・focus復帰/連打/既定Enterとclickの競合、provider画像dialog・Git/files子popup・footer詳細・mobile sheet全状態のイベント全組合せ。上表と停止契約以外の閉じ方を「すべて確認済み」としない。C2で共通部品へ移す際は、変更対象ごとにこの残りの契約を確定してから実装へ進む。

## 4. Hubを停止：source操作契約

|項目|確かめたsourceと意味|
|---|---|
|入口|app.ts:1838–1875 shutdown-btn click。mobile-home.ts:621も同ボタンへclickを委譲|
|公開状態|app.ts:1843–1848 cached ready→fetchExposeStatus(true)で再確認、取得失敗はcacheを使う|
|描画|settings.ts:818–878 appConfirmShutdown。index.html:2141 `model-picker-overlay`にdiv.confirm-dialog.confirm-dialog--dangerを作りinnerHTMLを置換|
|文言|ja.json:262–265、spawn_cancel。見出しHubを停止、本文停止方法を選択、sessions=「すべて停止」、shutdown=「Hubのみ停止」。action名だけで「セッションだけ」と解釈しない|
|fallback|overlay無しはwindow.confirm、OKならshutdown/stopExpose=exposeActive、cancelならnull。sessions選択肢はない|
|戻り値|null、または `{action:'shutdown'|'sessions', stopExpose:boolean}` の3分岐。booleanは別次元|
|外部公開checkbox|exposeActiveの時だけ既定checked。falseならstopExpose=false、表示対象だがcheckbox不在ならtrue|
|cancel|cancel button、Esc、event.target===overlayの外側clickがnull。内部clickは外側cancelにならない|
|Enter|document keydownがEnterを拾いclose('shutdown')。focused button別の分岐・input targetフィルタ・preventDefaultはこの関数には無い。ブラウザ既定clickとの競合は画面未確認で、C2の契約確認点|
|初期focus|app-confirm-ok（Hubのみ停止）へfocus。focus trap/元focus復帰の明示処理はこの関数内には見当たらない。OS/browser挙動は未検証|
|後始末|overlay clickとdocument keydown listener除去、hidden=true、innerHTML空、resolve。button listenerはDOM破棄とともに参照から外れる|
|停止実行|result nullなら何もしない。有値ならshutdown-btn.disabled=true。stopExposeならdisableExposeを先行（例外捕捉）。sessionsはkill-all→shutdown→window.close、shutdownはshutdown→window.close。通信失敗も各catchで進む既存契約であり今回変更なし|
|適用CSS|settings.css:871確認枠420px/radius-md/p14、:928通常、:951primaryはdanger。spawn.css:576 overlay fixed/.5黒/z9100/p16。danger modifier自体の専用ruleと決めつけず、実際のprimary ruleが色を与える|
|wheel|HTMLにaac-wheel-overlay。terminal.ts:159がvisible overlayを判定し、端末へのwheel転送を除外。静的gate成功でも全UIスクロール動作の保証ではない|
|閉じる左寄せ|この確認には単独✕がなく、cancel/2停止のaction列。close-leftでこの3ボタンが左寄せになるとはsourceから言えない。S17全体にclose共通化を無条件適用しない|

## 5. Cascade・動的値・source上の相違

- 読込順: index.html:18–24 vendor xterm→highlight→styles.css（その先頭33imports）→mobile-home→session-list-tabs→first-run-tour→session-strip。styles.css本体はimportsの後。後読込だけで常に勝つわけではなくspecificity/important/inline/継承を考慮する。
- 同じconfirm-dialogでもsession-list-tabs.css:23がnative dialogをdisplay:block/width420px/max-width:min(420px,90vw)/padding22pxへ上書きし、backdropは黒.55。settings.css:871のdiv/grid/p14/overlay黒.5と違う。modifier名confirm-dialog--danger/--warnには専用CSS ruleが無く、名前だけで種類を増やさない。
- global `font:inherit`（styles.css:263）、focus-visibleのaccent2px/offset2px（:270）に、spawn-confirm:303のoutline:none+box-shadow、tsb:357固有outlineが重なる。UA defaultやfont shorthandも最終値に関与する。
- dark/light selectors、body.close-left/detached/mobile-*、media720px/coarse、container幅、prefers-reduced-motionを保持して集計する。たとえばclose-button.css:134のph-close左寄せは、後続pane-placement.css:101の`.pane-slot .ph-close`でorder0/margin-right0へ戻る。適用文脈抜きで「すべて左に動く」と言わない。
- theme-tokens.ts:227–234、provider-icon.ts:311以降、live-status-color.ts:43–45/176–177、settings.ts:3712–3778等のinline/style.cssTextは動的。serverデータや利用者設定で変わる実行時値を計数済みとはしない。

相違の具体的対照（どちらも固定sourceに実在）:

|比較|値A|値B|解釈|
|---|---|---|---|
|通常確認/子起動確認の副操作|settings.css:928 padding6px13px/fs-sm/text/明示minheight無し|spawn-confirm.css:290 padding6px14px/fs-md/muted/minheight32px|source上の密度・文字差。統合候補、視覚的乱れ未判定|
|危険primary/起動primary|settings.css:951 danger-bg/danger/font700|spawn-confirm.css:308 accent/#fff/font600|役割差として残す可能性が高い。primary名称だけ統合しない|
|確認枠/モデル枠|settings.css:872 min420px/p14/radius-md8px|spawn.css:588 min460px/p12/radius-lg12px|source上の幅・角丸・余白差。C2幅段階の候補|
|単独close/footer close|close-button.css:43 transparent/muted→danger-bg/danger|token-statusbar.css:283固有配色→:293 #2a2f45|意図したfooter配色と明記されている例外|
|sidebar SVG線幅|index.html:1257 stroke-width1.8|index.html:1757 stroke-width2|同じ24viewBoxでも線幅差。アイコン役割差か目視確認|

## 6. C2で決める事項とC3/C4候補（未決定）

1. 中性/アクセント/危険/警告/成功/識別の役割名と、既存primaryの意味衝突をどう表すか。具体色を今決めない。
2. 文字7段階、height32/36とmobileタップ領域、radius/spacingトークンの適用範囲。端末/コード/emoji/font差を残す基準。
3. buttonをbase+variant+size+stateへ寄せる場合の互換契約。close例外を維持するか、行内closeと単独closeの2系統にするか。
4. modal幅段階とdiv/native dialog/popover/drawer/別窓の境界。Esc/外側/Enter/初期focus/復帰/連打の仕様、dangerとcancelの強弱。停止のEnter target挙動は明示確認する。
5. custom theme、footer固有配色、provider/状態/Git色の優先順位。default値が同じだけのaliasを勝手に廃止しない。
6. 共通部品をCSS utility/小さなDOM helperのどちらで持つか。外部UI依存は追加しない。既存helperが保つ通信/承認/保存契約を変えない。
7. C5で許可リスト付きの生色・新close・overlay wheel・状態付け忘れ検査をどう保守するか。字句出現数の増減だけを回帰判定にしない。

|後続候補群|対象|先に守る契約|
|---|---|---|
|C3基盤候補|tokens/close/provider icon/confirm、S17 Hub停止|戻り値・公開停止・focus/Enter・wheel、C2確認を先行|
|C4主shell|S01–S06、S12–S15|一覧選択/承認identity、IME/端末、DOM移設|
|C4設定/起動|S16、S18–S25、S28|入力検証/disabled/警告、remote/authを変更しない|
|C4補助|S07–S11、S26–S36|ログ/添付/差分/通知の意味、非modal維持|
|C4mobile/分離|S37–S40|coarse/狭幅、tap領域、follow、元view state|

## 7. 手元の画面確認待ちと実施済みを分ける

起動・停止・再起動・reload・認証は利用者が行う。本調査は実Hubへ接続していない。以下は**未実施の受入手順**で、実行許可や合格を表さない。

|対象|利用者側確認|期待して照合するもの|
|---|---|---|
|全S01–S41|dark/light/custom、標準/大文字、720px前後/狭幅/coarseで各入口を開く|文字潰れ/欠落/overflow、hoverとfocusの区別。source差が意図差か確認|
|S01–S06/S12–S15|空/選択/多件/disabled/進捗、マウスとTab操作|一覧/タブの選択が分かり、端末/IMEを妨げない|
|S16–S25|各設定24セクション、popover→confirmの遷移、長い値|見出し/入力/操作が残り、保存/通信契約を維持|
|S17|開く→Esc/外側/cancel、checkbox有無、Tab/Enterのfocus位置|キャンセルで停止せずlistener残留なし。実停止は利用者が許可した試験でのみ確認|
|S19/S20/S29|native dialogのfocus/暗幕/scroll、他overlayとの重なり|top-layerと通常z-indexを混同しない、背景端末wheel抑止|
|S27/S30/S35/S36|close-left ON/OFF、inline close/単独close、drawer|例外と共有styleの意図が保たれ、memoをmodal化しない|
|S37–S39|mobile承認/入力/キーボード/ドラフト/戻る|tap領域、閉じ方、背面scroll、draft中抑制|
|S40|分離タブ/gridでfollow切替/元Hubへ戻る|rootの複製・state喪失をしない|
|手元追加差分|公開基準と普段のRustビルド元の全差分を親AIが照合|未公開memo/footer等を古い調査で上書きしない|

実施済み: source全対象218件の固定読取、CSS/字句再集計、script self-check、source操作経路確認。未実施: 実画面/Windows/mobile/利用者設定/製品build/製品tests。独立レビューとCIはREVIEW.md/PROGRESS.mdおよびIssue16の対象SHA付き記録を参照する。C1棚卸し提出で親計画を完了にしない。

## 8. 再測定値（自動生成欄）

|計数|生の出現数|重複排除数|
|---|---:|---:|
|CSS宣言 font_size|878|54|
|CSS宣言 hex|673|196|
|CSS宣言 rgb|320|161|
|CSS宣言 var|4343|130|

CSS rule block（@keyframes内含む）: 3627。状態selectorのrule数（同一ruleが複数状態を持つ場合は各欄へ）: active=16、checked=3、disabled=63、focus=26、focus-visible=30、hover=280。

|候補群|値を持つrule数|一意な宣言ベクトル数（部品数ではない）|
|---|---:|---:|
|buttons|557|398|
|inputs|164|124|
|overlays|241|176|

実行コード/HTMLのSVG literal候補37、img literal候補3、inline style候補311。式で構築されるSVG/画像やCSS shapeはこの個数に含まれない。記号Unicodeはコメント含む候補、実画面個数ではない。

### CSS font_size 値一覧（値×出現数）

`0`×2、`0.55rem`×1、`0.56rem`×1、`0.58rem`×2、`0.62rem`×4、`0.6rem`×1、`0.72rem`×1、`0.74rem`×2、`0.76rem`×1、`0.82em`×2、`0.82rem`×1、`0.83em`×1、`0.85em`×3、`0.85rem`×1、`0.8em`×2、`0.8rem`×2、`1.05rem`×1、`1.15rem`×2、`1.23rem`×1、`1.25rem`×3、`1.2rem`×3、`1.5rem`×2、`10.5px`×4、`10px`×3、`11.5px`×7、`11px`×8、`12px`×13、`13px`×2、`16px`×6、`18px`×1、`2.5rem`×1、`20px`×1、`22px`×1、`9px`×4、`calc(var(--nsb-size, 32px) * 0.5)`×1、`clamp(1.45rem, 3vw, 2rem)`×1、`clamp(1rem, 2.2vw, 1.45rem)`×1、`inherit`×2、`var(--chat-font-size)`×1、`var(--close-btn-fs, 0.9rem)`×1、`var(--fs-2xs)`×93、`var(--fs-body)`×47、`var(--fs-lg)`×41、`var(--fs-md)`×130、`var(--fs-sm)`×264、`var(--fs-sm) !important`×1、`var(--fs-small, 12px)`×4、`var(--fs-xl)`×9、`var(--fs-xs)`×149、`var(--header-font-size)`×23、`var(--header-font-size, 0.72rem)`×1、`var(--icon-btn-fs)`×11、`var(--session-strip-font-size, 13px)`×1、`var(--statusbar-font-size)`×7

### CSS hex 値一覧（値×出現数）

`#000`×5、`#0008`×2、`#0284c7`×1、`#0369a1`×1、`#050609`×3、`#0550ae`×2、`#06080f`×2、`#06b6d4`×1、`#08090b`×1、`#0a0c14`×3、`#0a0d15`×2、`#0a3069`×1、`#0d1117`×11、`#0e0f12`×3、`#0f1320`×5、`#0f172a`×1、`#0f1822`×1、`#111`×5、`#111521`×1、`#111827`×3、`#116329`×2、`#14171f`×7、`#15171c`×1、`#15803d`×1、`#161b22`×7、`#166534`×5、`#16a34a`×2、`#181c24`×5、`#181d2c`×1、`#1a1a1a`×1、`#1a1b26`×3、`#1a1d29`×1、`#1a1f2e`×4、`#1a2f1e`×1、`#1d2025`×1、`#1e2330`×11、`#1e2534`×1、`#1e293b`×1、`#1e40af`×1、`#1f2335`×1、`#1f2937`×3、`#20232f`×2、`#22262c`×1、`#22c55e`×6、`#232a3d`×2、`#232b40`×1、`#24292f`×6、`#2563eb`×7、`#292e42`×7、`#2a2f45`×9、`#2a3040`×7、`#2a3a4d`×1、`#2b3242`×1、`#2d3f55`×1、`#2e1065`×1、`#2e7d32`×5、`#334155`×5、`#34d399`×2、`#38a169`×4、`#38bdf8`×1、`#3b4261`×3、`#3b82f6`×30、`#3f4859`×1、`#3fb950`×2、`#4338ca`×2、`#444`×1、`#4568a9`×6、`#475569`×3、`#4a5568`×3、`#4ade80`×1、`#4c0519`×1、`#4c1d95`×1、`#4f46e5`×1、`#4f5bd5`×2、`#52525b`×1、`#565f89`×7、`#60a5fa`×6、`#6366f1`×9、`#64748b`×2、`#666`×1、`#6b7280`×10、`#6b74ff`×1、`#6cf`×3、`#6d28d9`×1、`#6e7781`×1、`#6e8eff`×2、`#6ee7b7`×1、`#6f6`×1、`#6fc`×2、`#713f12`×1、`#735c0f`×1、`#73daca`×2、`#777`×11、`#7aa2f7`×8、`#7c3aed`×3、`#7dcfff`×7、`#7dd3fc`×2、`#7f1d1d`×1、`#818cf8`×3、`#82071e`×1、`#8250df`×1、`#86efac`×6、`#8994a3`×1、`#8b5cf6`×1、`#8b92ff`×1、`#92400e`×2、`#93c5fd`×9、`#94a3b8`×8、`#953800`×1、`#99a`×5、`#9aa4b2`×25、`#9aa5ce`×2、`#9ca3af`×1、`#9cdcfe`×1、`#9d7cd8`×2、`#9ece6a`×5、`#a5b4fc`×3、`#a855f7`×2、`#a8b1c2`×1、`#b00`×1、`#b26b00`×1、`#b42318`×1、`#b6becd`×1、`#bb9af7`×3、`#bbf7d0`×1、`#be123c`×1、`#bfdbfe`×2、`#c0392b`×4、`#c084fc`×2、`#c0caf5`×11、`#c2410c`×1、`#c4973a`×1、`#c4b5fd`×3、`#c7d2fe`×1、`#c8f`×1、`#c9302c`×1、`#c9d1db`×1、`#ca8a04`×5、`#cbd5e1`×12、`#ccc`×2、`#cf222e`×1、`#d29922`×2、`#d33`×1、`#d4a017`×5、`#d69e2e`×4、`#d8b4fe`×1、`#d9534f`×1、`#d97706`×1、`#dafbe1`×1、`#dbeafe`×1、`#dc2626`×2、`#dcfce7`×2、`#dd6b20`×1、`#e06c75`×8、`#e0af68`×4、`#e2e8f0`×15、`#e53e3e`×11、`#e57373`×1、`#e5e7eb`×2、`#e66`×1、`#e6edf3`×3、`#eab308`×2、`#ede9fe`×3、`#ef4444`×12、`#efc66d`×3、`#f1f5f9`×1、`#f43f5e`×1、`#f44`×1、`#f4f6fa`×1、`#f59e0b`×13、`#f5a399`×1、`#f66`×2、`#f6c343`×3、`#f6f8fa`×2、`#f7768e`×4、`#f87171`×8、`#f8fafc`×1、`#f97316`×1、`#f9a8d4`×1、`#facc15`×1、`#fb923c`×3、`#fbbf24`×1、`#fc6`×3、`#fca5a5`×2、`#fda4af`×1、`#fde68a`×3、`#fecaca`×3、`#fecdd3`×1、`#fef3c7`×1、`#ff6b6b`×6、`#ff7474`×2、`#ff9e64`×2、`#ffe4e6`×1、`#ffebe9`×1、`#fff`×51、`#ffffff`×2

### CSS rgb 値一覧（値×出現数）

`rgba(0,0,0,.28)`×1、`rgba(0,0,0,.42)`×1、`rgba(0,0,0,.45)`×2、`rgba(0,0,0,.46)`×2、`rgba(0,0,0,.5)`×1、`rgba(0,0,0,.55)`×1、`rgba(0,0,0,.58)`×1、`rgba(0,0,0,.6)`×2、`rgba(0,0,0,0.15)`×1、`rgba(0,0,0,0.18)`×1、`rgba(0,0,0,0.22)`×1、`rgba(0,0,0,0.24)`×1、`rgba(0,0,0,0.25)`×2、`rgba(0,0,0,0.28)`×1、`rgba(0,0,0,0.3)`×1、`rgba(0,0,0,0.32)`×2、`rgba(0,0,0,0.34)`×1、`rgba(0,0,0,0.35)`×9、`rgba(0,0,0,0.36)`×2、`rgba(0,0,0,0.38)`×1、`rgba(0,0,0,0.4)`×11、`rgba(0,0,0,0.45)`×14、`rgba(0,0,0,0.48)`×2、`rgba(0,0,0,0.5)`×10、`rgba(0,0,0,0.55)`×20、`rgba(0,0,0,0.58)`×2、`rgba(0,0,0,0.6)`×5、`rgba(0,0,0,0.62)`×2、`rgba(0,0,0,0.65)`×1、`rgba(0,0,0,0.7)`×1、`rgba(0,0,0,0.78)`×1、`rgba(0,0,0,0.88)`×1、`rgba(0,0,0,0.9)`×1、`rgba(100,116,139,0.35)`×1、`rgba(100,116,139,0.38)`×1、`rgba(100,116,139,0.42)`×1、`rgba(100,116,139,0.5)`×1、`rgba(100,140,255,0.28)`×1、`rgba(108,255,204,0.05)`×1、`rgba(108,255,204,0.2)`×1、`rgba(122,162,247,.08)`×1、`rgba(122,162,247,.10)`×1、`rgba(125,207,255,.18)`×2、`rgba(128,128,128,0.12)`×1、`rgba(128,128,128,0.25)`×1、`rgba(13,15,23,0.7)`×1、`rgba(13,15,23,0.85)`×1、`rgba(13,17,23,0.82)`×1、`rgba(134,239,172,0.95)`×3、`rgba(139,92,246,0.12)`×1、`rgba(139,92,246,0.4)`×1、`rgba(139,92,246,0.5)`×1、`rgba(139,92,246,0.7)`×1、`rgba(147,197,253,0.18)`×1、`rgba(147,197,253,0.78)`×2、`rgba(147,197,253,0.92)`×3、`rgba(148,163,184,0.12)`×1、`rgba(148,163,184,0.22)`×1、`rgba(148,163,184,0.35)`×1、`rgba(148,163,184,0.45)`×1、`rgba(148,163,184,0.78)`×2、`rgba(148,163,184,0.85)`×3、`rgba(148,163,184,0.9)`×2、`rgba(15,19,32,0.85)`×1、`rgba(15,23,42,0.04)`×1、`rgba(15,23,42,0.05)`×1、`rgba(15,23,42,0.06)`×1、`rgba(15,23,42,0.38)`×1、`rgba(15,23,42,0.75)`×1、`rgba(165,180,252,0.95)`×1、`rgba(18,28,51,.40)`×1、`rgba(180,197,240,.4)`×1、`rgba(196,181,253,0.95)`×1、`rgba(203,213,225,0.75)`×2、`rgba(203,213,225,0.85)`×1、`rgba(203,213,225,0.9)`×1、`rgba(217,119,6,0.5)`×2、`rgba(22,30,43,0.92)`×4、`rgba(220,38,38,0.08)`×1、`rgba(220,38,38,0.12)`×1、`rgba(220,38,38,0.2)`×1、`rgba(229,115,115,0.08)`×1、`rgba(229,115,115,0.4)`×1、`rgba(234,179,8,0.12)`×1、`rgba(239,68,68,.2)`×1、`rgba(239,68,68,0.15)`×1、`rgba(239,68,68,0.18)`×1、`rgba(239,68,68,0.28)`×1、`rgba(239,68,68,0.5)`×1、`rgba(241,245,249,0.90)`×1、`rgba(244,63,94,0.06)`×1、`rgba(244,63,94,0.2)`×1、`rgba(244,63,94,0.34)`×1、`rgba(245,158,11,0.06)`×1、`rgba(245,158,11,0.08)`×1、`rgba(245,158,11,0.16)`×1、`rgba(245,158,11,0.18)`×1、`rgba(245,158,11,0.35)`×3、`rgba(248,113,113,0.45)`×1、`rgba(250,204,21,0.4)`×1、`rgba(251,191,36,0.4)`×1、`rgba(251,191,36,0.6)`×1、`rgba(251,191,36,0.95)`×5、`rgba(252,165,165,0.95)`×2、`rgba(255,102,102,0.06)`×1、`rgba(255,107,107,0.06)`×1、`rgba(255,107,107,0.08)`×1、`rgba(255,122,61,0.04)`×2、`rgba(255,140,0,0.8)`×1、`rgba(255,160,0,0.65)`×1、`rgba(255,204,108,0.2)`×1、`rgba(255,210,0,0.35)`×1、`rgba(255,255,255,0.025)`×1、`rgba(255,255,255,0.03)`×2、`rgba(255,255,255,0.035)`×1、`rgba(255,255,255,0.04)`×2、`rgba(255,255,255,0.05)`×5、`rgba(255,255,255,0.06)`×16、`rgba(255,255,255,0.08)`×19、`rgba(255,255,255,0.1)`×7、`rgba(255,255,255,0.12)`×1、`rgba(255,255,255,0.14)`×1、`rgba(255,255,255,0.15)`×1、`rgba(255,255,255,0.2)`×4、`rgba(3,5,15,0.7)`×1、`rgba(3,6,16,.58)`×2、`rgba(30,40,60,0.36)`×2、`rgba(34,197,94,0.08)`×2、`rgba(34,197,94,0.15)`×1、`rgba(34,197,94,0.2)`×1、`rgba(34,197,94,0.3)`×1、`rgba(34,197,94,0.35)`×1、`rgba(34,197,94,0.42)`×1、`rgba(56,189,248,0.45)`×1、`rgba(59,130,246,.15)`×1、`rgba(59,130,246,0.12)`×4、`rgba(59,130,246,0.28)`×1、`rgba(59,130,246,0.35)`×1、`rgba(59,130,246,0.45)`×3、`rgba(59,130,246,0.92)`×2、`rgba(7,9,14,0.72)`×1、`rgba(79,70,229,0.15)`×1、`rgba(8,13,25,.50)`×1、`rgba(91,124,246,.20)`×1、`rgba(91,124,246,.35)`×1、`rgba(91,124,246,.40)`×1、`rgba(91,124,246,0.12)`×1、`rgba(91,124,246,0.2)`×2、`rgba(91,124,246,0.25)`×1、`rgba(91,124,246,0.35)`×1、`rgba(91,124,246,0.4)`×1、`rgba(96,165,250,0.1)`×1、`rgba(96,165,250,0.25)`×1、`rgba(96,165,250,0.45)`×1、`rgba(96,165,250,0.55)`×1、`rgba(96,165,250,0.75)`×1、`rgba(99,102,241,0.12)`×2、`rgba(99,102,241,0.15)`×2、`rgba(99,102,241,0.25)`×2、`rgba(99,102,241,0.3)`×2、`rgba(99,102,241,0.45)`×3

### CSS var 値一覧（値×出現数）

`--accent`×324、`--accent-2`×5、`--accent-glow`×4、`--accent-hover`×1、`--accent-soft`×53、`--accent-text`×86、`--app-header-h`×4、`--approval-dock-shift`×1、`--approval-dock-w`×1、`--aqt-clamp-pre`×1、`--aqt-clamp-q`×1、`--author-color`×1、`--bad`×4、`--badge-running-bg`×3、`--badge-running-text`×11、`--badge-standby-bg`×5、`--badge-standby-text`×9、`--bg`×115、`--bg-2`×6、`--bg-deep`×60、`--bg-elev`×154、`--bg-elev2`×40、`--bg-elev3`×2、`--bg-hover`×17、`--bg-layer`×3、`--bg-soft`×11、`--bg2`×2、`--border`×13、`--border-soft`×1、`--button-fg`×3、`--card-icon-shift`×1、`--card-ident-color`×2、`--chat-filter-bar-height`×1、`--chat-font-size`×1、`--chip-bg`×3、`--close-btn-fs`×1、`--close-btn-pad`×1、`--control-h-lg`×3、`--control-h-md`×6、`--control-h-sm`×56、`--danger`×125、`--danger-bg`×25、`--danger-bg-hover`×3、`--env-color`×3、`--fg`×8、`--fg-muted`×4、`--font-mono`×3、`--fs-2xs`×94、`--fs-body`×47、`--fs-lg`×42、`--fs-md`×132、`--fs-sm`×266、`--fs-small`×4、`--fs-xl`×9、`--fs-xs`×149、`--green`×53、`--green-bg`×11、`--green-text`×34、`--hash-color`×3、`--header-font-size`×24、`--hold-progress-duration`×1、`--hover-bg`×1、`--icon-btn-fs`×11、`--input-icon-x`×1、`--input-icon-y`×1、`--lane-1`×1、`--lane-2`×5、`--lane-3`×6、`--lane-4`×1、`--line`×394、`--line-soft`×73、`--live-status-bg`×1、`--live-status-fg`×3、`--mobile-float-clear`×2、`--mobile-vv-bottom-offset`×1、`--mono`×10、`--muted`×341、`--muted-2`×102、`--nsb-size`×20、`--ok`×5、`--orange`×1、`--pane-cols`×2、`--pane-rows`×2、`--panel`×3、`--perm`×3、`--perm-bg`×1、`--prov`×8、`--prov-claude`×1、`--prov-codex`×1、`--prov-command-code`×1、`--prov-copilot`×1、`--prov-cursor-agent`×1、`--prov-grok`×1、`--prov-lm-studio`×1、`--prov-ollama`×1、`--prov-opencode`×1、`--radius-lg`×26、`--radius-md`×162、`--radius-pill`×4、`--radius-sm`×228、`--radius-xl`×10、`--red`×3、`--relay-color`×8、`--remote-bg`×1、`--remote-text`×3、`--row-hover`×9、`--session-strip-font-size`×1、`--session-strip-grip-h`×2、`--session-strip-h`×1、`--sidebar-default`×1、`--sidebar-max`×1、`--sidebar-min`×1、`--space-1`×52、`--space-2`×154、`--space-3`×37、`--space-4`×4、`--statusbar-font-size`×7、`--success`×5、`--text`×353、`--text-dim`×5、`--text-soft`×44、`--tool-order`×1、`--touch-target`×3、`--tsb-bottom-offset`×6、`--turn-card-h`×2、`--warn`×86、`--warn-bg`×13、`--warn-text`×35、`--warning`×10、`--yellow`×1

### source字句の対照集計（CSS集計と合算しない）

- hex: 716出現 / 200一意。全値と頻度はrecount出力の同名欄。
- rgb: 333出現 / 173一意。全値と頻度はrecount出力の同名欄。
- var: 4356出現 / 134一意。全値と頻度はrecount出力の同名欄。

### 条件一覧

- `@container (max-width: 235px)`
- `@container (min-width: 1040px)`
- `@keyframes aacli-compact-slide`
- `@keyframes aqt-badge-pulse`
- `@keyframes deferred-send-pulse`
- `@keyframes expose-dot-pulse`
- `@keyframes mtl-status-pulse`
- `@keyframes state-pulse`
- `@keyframes state-spin`
- `@keyframes status-chip-blink`
- `@keyframes usage-skeleton-shimmer`
- `@keyframes usage-spin`
- `@keyframes voice-processing-pulse`
- `@keyframes voice-pulse`
- `@keyframes waiting-attention`
- `@keyframes wakeword-pulse`
- `@media (hover: hover) and (pointer: fine)`
- `@media (hover: none), (pointer: coarse)`
- `@media (max-width: 1000px)`
- `@media (max-width: 1100px)`
- `@media (max-width: 1200px)`
- `@media (max-width: 1400px)`
- `@media (max-width: 1600px)`
- `@media (max-width: 480px)`
- `@media (max-width: 520px)`
- `@media (max-width: 560px)`
- `@media (max-width: 570px)`
- `@media (max-width: 600px)`
- `@media (max-width: 640px)`
- `@media (max-width: 720px)`
- `@media (max-width: 720px), (pointer: coarse)`
- `@media (max-width: 760px)`
- `@media (max-width: 768px)`
- `@media (max-width: 860px)`
- `@media (max-width: 880px)`
- `@media (max-width:720px)`
- `@media (min-width: 721px)`
- `@media (min-width: 721px) and (not (pointer: coarse))`
- `@media (min-width: 721px) and (pointer: fine)`
- `@media (min-width: 900px)`
- `@media (pointer: coarse)`
- `@media (prefers-reduced-motion: no-preference)`
- `@media (prefers-reduced-motion: reduce)`
- `@media(max-width:720px)`

## 9. 固定対象リスト（全218件）

|path|bytes|SHA256|
|---|---:|---|
|web/src/app-entry.ts|5212|ec19812e70de3b006803c11ef0a555e1b1a8ac5b10ba563be6e23bd616a4750d|
|web/src/app.ts|132460|1b786c6da3add149718bc3202784e4e2e5587210e9ee52ff04b2307b064817e5|
|web/src/app/action-bar-resize.ts|6551|2e66cf1a5977ddb7b728d81bcb76970737fa81dc7bce21da5e841d050c3658b1|
|web/src/app/alt-drag-select-fixtures.ts|5551|09a77759ec05ae60c3de6f5a250cb4eccbe71974039d965c1d5745e402e7f52a|
|web/src/app/alt-drag-select-view.ts|11659|2fccd1a477c4a68af6b4c0c26bd47610f8cdd67547f1d370c3cfa43e2e28adb5|
|web/src/app/alt-drag-select.ts|7158|e8b862471c79003891c809d7435253e891f25b763a19d5dcdfab1ff8ff3c68ed|
|web/src/app/alt-scroll-rail-fixtures.ts|4521|0b042f67d24f6c7a8f4e871cf59f41eec189896f389d60a85b2ffd5629b54c92|
|web/src/app/alt-scroll-rail-view.ts|21556|5ea0870fefb72605da2d4be232b870957a53dce4fa9c13828cc75614e7c92563|
|web/src/app/alt-scroll-rail.ts|6018|6e22a5c0d29bd8cc8ec5d752ee7487e8d25a8dcff5c96a006392c55cb185a685|
|web/src/app/approval-answered-fixtures.ts|8330|adb049ead80c21ecfeff935aac35fd746347a7ac1e6fe301da7a6bbdb556dbc4|
|web/src/app/approval-answered.ts|11053|1b77686cdc026c8c45d69a1ca839ca3948ef280236b2537d7d68320b8c70de00|
|web/src/app/approval-dock.ts|3498|9b945154dd310bb6b4c8288341e30ca8665b2f72858e7c42ef38ff984ba77d78|
|web/src/app/approval-owner-fixtures.ts|5475|5caad6530968c085b82dcbca7121ca94ef3d22a0f2625005202f48e5c86c9c83|
|web/src/app/approval-owner.ts|3447|ce70c0e4b7538ad6ef8ce9e8465708711d2197f1daeaa6e08b88df4a6add8fd3|
|web/src/app/approval-parser-fixtures.ts|34910|12068aaaaacc4830e30324ce1b125efe1dd02fa76100bf39ae235df5b0b3076b|
|web/src/app/approval-parser.ts|35472|94918c8c90314a888f6d1f280f2959df1e63e4604c3695087c6e58a6faba59d8|
|web/src/app/approval-queue-tab.ts|30725|9eb23491a40c56d38141e06a6b1703502da4429683db98903ee4ad447e9988e8|
|web/src/app/approval-store-fixtures.ts|22593|af84121be7e7626131f0c32a39512d7c822838ddaacce8c1da60b55025d4b63b|
|web/src/app/approval-store.ts|24805|5d2336f4b53f85f758585f3222a6be53fdbf406bc7378459d7ed9a1ab9b3275b|
|web/src/app/approval-ui.ts|11619|6d2fe9a1953567e47f0b444530a766db9445e683871721a5d26ecddb952152d7|
|web/src/app/approval.ts|99315|05f59f726a32de85f88f942cab4e9c4d69fc616efadf2b02d4d8d27dd0bfe473|
|web/src/app/attachments.ts|20088|14511dbba434751a7aa0b5f01e55deea715344d84000741df61eeca1d270ac70|
|web/src/app/boot-guard.ts|6662|88f244c5a72dcb0526ac21dc34a845f56387e7f54b4bc12e363840ee401d0800|
|web/src/app/bug-report-modal.ts|14886|3463c20e1f2d0a8922592bbb23f26aef88c7dc2502bf802fa88addb3c2be8a60|
|web/src/app/chat-history-dom-fixtures.ts|5129|d7d5b0d3e2fc2450fd6e2e52c329dcb37c8354513a00aa6610b5572a4c3a5163|
|web/src/app/chat-history.ts|81378|0bb89b7093be2d1e84530c2e58d1b071a06e5d0c9e8f8475c517d4f209657493|
|web/src/app/chat-payload.ts|15622|42236e8ef02399184c309a6c8047bfc3b1fee1c6cbc73425adf37ffeda4f08ee|
|web/src/app/cli-availability-fixtures.ts|11809|cb070cdfc3d47b98c6de24a627772ef8fcade03e97a75b7372cdeb152a508d14|
|web/src/app/cli-availability.ts|15787|bbb497e019ddd57fc0579211aff812d5dc78d8e3082be33933edd1d362359149|
|web/src/app/cli-maintenance.ts|32968|03106f60ca7741448cb636bd38e00c9ae8ecc414983a82d9761c8fdad02baf5f|
|web/src/app/command-palette-store.ts|8517|d19ea5c45942dd58a847a84c95ebcb03253474a8309cfea86c3d9904ef419226|
|web/src/app/cr-erase-filter-fixtures.ts|4159|7bc63b3819e8437864415c3f116a98f0779d3f93bc3dd26b3430f25b43721afb|
|web/src/app/cr-erase-filter.ts|4197|a04dab197fee1d6a5763701815249fbfb42a94b66661e854d3e4271a321352cc|
|web/src/app/cursor-hide-filter-fixtures.ts|9012|ba2d80a2a768d64c8e97ef62702f2d6e6a74beb6271a199a9d76d32b47aaab87|
|web/src/app/cursor-hide-filter.ts|10052|403603d1964d239baf6f6706a284b83270e188efcd8e73b24798f3e4e0f3c218|
|web/src/app/custom-themes.ts|1009|7c2c21e86095118f27e20571b558936052921a36ec60b627057623732c94c470|
|web/src/app/cwd-path.ts|4823|ca0360a930491f3704567a28bd8589d98be8ba42e6b87c51d9c9ed91bedc75dd|
|web/src/app/deferred-enter.ts|7971|d13135bd8b8630e1983270b6a981f0b1859ed92e9b2154e652048cd710fed291|
|web/src/app/derive-dialog-store.ts|7295|ede2bab92d67ed203f6c02adad58c31439105ca0d5686e27981ae0f202f8be84|
|web/src/app/derive-dialog.ts|34477|4bded655eda87569b0d070a6c17129717f354e08d0560c1120b6ac5212f10ebe|
|web/src/app/detached-grid-launcher.ts|14241|e1181b46becbea8797bf4bba4caf7527bc806e526ba3116cd0a9c773a6ca214d|
|web/src/app/detached-grid.ts|32081|a9330c421c7d344a0e2bbf1c2102b4ced6f91feb481148782fdd14bcdeec9a1a|
|web/src/app/detached-view-mode-fixtures.ts|3058|1fd4090ca312a99bd2294e920230bd2dab5c2a66a3ae597cf280df1d3b7e25fe|
|web/src/app/detached-view-mode.ts|9304|7ce5942e556a3861f27d164c1b552036327750d7a5888754cc11f47f5c388527|
|web/src/app/detached-view.ts|17754|21f8f90ce070b1692b462ad7076ca1d9edc6876f7c482054709bf7544f790277|
|web/src/app/done-summary-fixtures.ts|3687|44c4809d4ffac536c2e18f93fe3c4d8d50544d22affa9c4a4c16382e53aada3e|
|web/src/app/done-summary.ts|3810|85d27b048510a877fe6a1579b13c14096c2c6ee090286205b3cbafaa9a3247ea|
|web/src/app/expand-popup.ts|7414|5260ab34056172d0412ff7ec744fac1a80d4325d287a2c100255a70742944ec9|
|web/src/app/files-view.ts|136168|ae09369de870d281a2b11f2dbf448927c239dff4b05e5ecfa9b9134c6fd7f0aa|
|web/src/app/first-run-tour.ts|6311|c13db3c802ad80e42a1e07d120d3e22c0f0b3bade5649c9125f7866aacc515c9|
|web/src/app/flexible-pane-state-fixtures.ts|6315|0878a788867460fd22baa6574bf0122f87d14122e20f23166d109162e13efefc|
|web/src/app/flexible-pane-state.ts|9185|60d2b660bf147fc23a10120a24a5ae215b310488ffb67e3ba138709e15c462a7|
|web/src/app/flexible-pane-views.ts|7582|683dcbab5ac281a45f48792a59afc27fe291b4cb01ed69413a5c8b2a890006ca|
|web/src/app/git-view.ts|75004|0fd9d875af117db04e8cf30ba9e3556280a25514ae777dd7c61ceb87d8715825|
|web/src/app/grok-chat-viewer.ts|9420|050837306f0e52e29d6282c4f71f5c322c388f6632a1f9e6b00ab1fc0732aefe|
|web/src/app/handoff-store.ts|9711|7fb25218e2e40b5d26eff0f7bda7b97e198478f6d070bea7b82f63ac53a923ad|
|web/src/app/handoff.ts|20482|bb8ce15e5d7964bbb0e01cf6cd9a66a9a5f972983bc646cc63fb102c55d42443|
|web/src/app/history-lite.ts|7284|1f42ff80d554f19844ed0a57fc85a65fed784999036412b60fe1c22408c55bdd|
|web/src/app/history-viewer.ts|9884|cf6442785adca757745e7c4d654a39d33fff887052fb04a830758230345184e0|
|web/src/app/host-expose.ts|14906|cfb46d302c817fbf4e212c917fd5694461d832e5a89cd0844a5e997104d4afc3|
|web/src/app/hub-marker-filter-fixtures.ts|22788|a7f6f7220282f90590bf7f10392b11c74c19d471e460ea87015f65553bb4d896|
|web/src/app/hub-marker-filter.ts|24336|b1a6022042d7c32b2f1df51c7a61d7ed5853a7241d2f6fe13c54226784fde417|
|web/src/app/input-icon-position.ts|10878|abf84d69acd3655ebd2287f85f3c5381f46e8e77fabe95bf9b47c7527c92b91b|
|web/src/app/input-tools-order.ts|8565|c0caa41a349c74e0d726a3c58829c6da7e8e2a8e6e649067ee3c83160e123f69|
|web/src/app/jev-opt-in.ts|2588|6527a97cce6472655416fe426d345a03c37165bc58969b9f6255403de5b9b969|
|web/src/app/lightbox.ts|1193|8b859465e2ab70198b3313475c05f6e75947bba59f2121ce94290646a6d686e5|
|web/src/app/live-status-color.ts|7853|63f4775a54adb2be08ea071271249bd5333aba4f88fba2bfcc8fc860aedcb3d7|
|web/src/app/live-status-fixtures.ts|2155|7e1c2435100fc8a374fe8c045cfc5707f864873c8de38f3a183f5a498f38c973|
|web/src/app/live-status.ts|2787|fec1bbc9604cf9f998db0f5af5b427423a03c3e9e73c8a38f3dfcab69c30c64d|
|web/src/app/longproc-fixtures.ts|2855|f069b3c461fa58a5da65eb69abc33e4400573fd416d3bdc225a4ad7a7b4a0d0e|
|web/src/app/longproc.ts|3336|7f280bded5c2eff19b878a24b61c27ef35e59f93cebfb36022fabe448fa26a55|
|web/src/app/memo-model.ts|11253|23367dc0d4091db964703bf3f25dc1838220eda7b80b4954971016029eb9ad4e|
|web/src/app/memo-panel.ts|27373|5491bcafba616bd8729aa2e0265f61ee03444ab65c8fa825238cd8bacdd4eff1|
|web/src/app/mobile-approval-only.ts|5859|672d07af72bd65b0cb63b1004815c5c19418894aac0d4c144db3ce4601fc63ed|
|web/src/app/mobile-approval-sheet.ts|18499|65d387ad93dc00f53f2266a0f43243a5f622700c8bd6cc4c908a0bdbb0b106d4|
|web/src/app/mobile-connect.ts|55487|dd8182d17be6082045c7c8c93605de7ec63b5035f07a72f1b35bb5c7afecf45f|
|web/src/app/mobile-home.ts|26816|238b414984c1a6bd1cc224112f7011d8e42b433d39baca2b4a29debdc12b3046|
|web/src/app/mobile-short-nudge.ts|6858|e65d0d329332830abaaa7b1a9ebf8678443cec1c5713962b2a7d664322eee82e|
|web/src/app/mobile-terminal-lite.ts|10053|54495bf0428cb689913fd375f6c3de153b6ed10dd074818593cf3748b8e5f98b|
|web/src/app/mobile-transcript.ts|7324|8b344baa06400a0c7a7d94615be637f6e6b1ad93b4e07322f0021f58285b0098|
|web/src/app/mouse-mode-tracker-fixtures.ts|4020|1b81dc070fa69b03bd8b228de69e6e4d5fa489f146d9848dd16ef118fd8f164f|
|web/src/app/mouse-mode-tracker.ts|6013|fa670619cff18084a83ba9bbea472c66dc4196de454159d8ffd123bc0a3a2d6e|
|web/src/app/multi-pane.ts|69979|8842047a0f668c8ab0fddf1b6039dd6156cfb5c3195e82c2a218293f557e9c2e|
|web/src/app/multi-scope-fixtures.ts|10103|707c18abb884e78a774cc8aa24854ed66b97094f3a769c00afacb9f304abaa3a|
|web/src/app/multi-scope.ts|8792|d487acaf5951729abe621307632648ae9e963e4f2a41334e656f70976f666a01|
|web/src/app/notification-volume.ts|965|3a066a0d1b9191cdbc8dbf35b070cf0befb950183c2750469178db32ec116f30|
|web/src/app/orchestration-dashboard.ts|33090|1a9f4830140e8d30956eed4d9519a036bfd32551469eb6df46eb199fa2182c61|
|web/src/app/orchestration-roles.ts|1071|a78cf1022d059377b634959d2566449bef2f7746d258f4121e2ccbbf36c4d51b|
|web/src/app/pane-placement-menu.ts|10346|05584aaea590752c73c48ce5b8fcb521b4d12d316ea5b110fcacc0f52c856e2b|
|web/src/app/path-detect-fixtures.ts|15285|cbea045c492cb295020a85e3de5db2186e9fc531da8da2ac67cf9d92e3d04613|
|web/src/app/path-detect.ts|11154|d273a372f9229595b434da8e6702f270130c5b856e5e227506ba8a9a184b004a|
|web/src/app/path-links.ts|22570|3f5faf9cfd5769cf92a96e14a4aac8e3a85d7a9d58abaa8c6dd445cfad2ab4be|
|web/src/app/project-view-memory-fixtures.ts|6727|c7b1c98bbb221fe98313cf3f8f95d626520aed5403b128cd586d8c8915a86f73|
|web/src/app/project-view-memory.ts|7352|2d955220ffcb2343dc38b3fcf2dc704bbf11a5d68edd24487f5e8fce648b00d5|
|web/src/app/project-view-store.ts|3755|9a5d113557460cca003d81b3f800017d5b7553fa6a56809f6e92651223ac8338|
|web/src/app/prompt-templates.ts|25990|b00674c457735c7624d07376bbcd34427b0c73c312cc92f0f2e1682bb4299f3a|
|web/src/app/provider-capabilities-fixtures.ts|2161|500ca010ea40dbdd48a416e96d22970ebe54c4b54f22ac85206198df5117c1a6|
|web/src/app/provider-capabilities.ts|3518|b3911493da7d69dd82f019f77da7e0ef29d9081762edf086e34e6a67533e1b11|
|web/src/app/provider-icon.ts|21828|8776189d3e0a6e892a968ca29dc416659912675c8b546f00d52253e10d37ec17|
|web/src/app/provider-manager-view-fixtures.ts|11084|286c3f11e9f11aef23c2bc50f0d7d28ddfed4f6869e78f9adc506d8cebf3140e|
|web/src/app/provider-manager-view.ts|16711|82f7e6bd4bd3dd9cacd8de49be785828bbe0d1697c9c84ab7a6244d8ba120faf|
|web/src/app/provider-manager.ts|56651|d9d379562508d6cc6c836eef0b7cee604361e400bfc5f86028b06866666a0010|
|web/src/app/provider-order.ts|3401|7f23e54290eb61c09a59d76d6bd8670ee50c25b67abfdbe25b2e9e0e638216b9|
|web/src/app/provider-store.ts|19309|bab1fcb68c10b1015ab465c152b7b77da546b97bedbe75896ae709edaa70f650|
|web/src/app/pwa.ts|5990|31935a0d292855532d75e40d8c7a81c8d8ee355fedee44af68d775b75256a28a|
|web/src/app/relay-dialog.ts|24251|9c254c2f58816fc573ba2e5383c12bd593476e8ce0abf103528ab4c37924ec6b|
|web/src/app/remote-auth.ts|7451|b01d678cfc8c785427b7d05de3dbea5d7a4a54033546dc109191c66942ee107c|
|web/src/app/residue-sweep.ts|6562|f05e344f5ea05f8db595e8304aaaecb00ffa3ea5029e2e0716a728345df86f67|
|web/src/app/review-load-generation.ts|742|3792c49676a7782b9d86dacebe3d5fd7216eb27b5231b6cca62e031c60099a62|
|web/src/app/review-view.ts|35839|b4524c576bb23e46316b5db86a6833a784860d3f65aea09b3b38b34611858702|
|web/src/app/routines-model.ts|1613|e2d6bd5a45711d4465cc50efe27d0c132c24d81dcad06738713c68e264548a5c|
|web/src/app/routines.ts|16222|40f180187b5470cb4e48c749c07f0d38cb41e2deadf247804f7161e40aa931bc|
|web/src/app/server-modal.ts|20522|e2de1fa36ae197b141dbd44cc711138e0d2656e3aba5828f28451aca778d8146|
|web/src/app/session-list-tab-fixtures.ts|9238|3ff088422bbfc84649562cbddf19fce42a34d13fb6ec8008988d55e85b8aafd6|
|web/src/app/session-list-tab-state.ts|10247|342a13e4e64ff521e585e34eae94c60f27432be721e90cf6e3985c7ad8cebaeb|
|web/src/app/session-list-tabs.ts|29227|0482b5aac83cbfb110db631d911a731c39735c6df837db6a114e7ea5ca729ef8|
|web/src/app/session-list.ts|102088|e8a4f4b232c6a905edc7257ec4afc4ed58aaf3d05c7e9c2cc2b6fd0e898ed2e6|
|web/src/app/session-search-palette.ts|22999|031be2ff02917660da9e80b6e60cf5b0a47b1aea6ebe053036f9f72cbc2d59ec|
|web/src/app/session-spawn-fixtures.ts|4846|2afeb2342923261bb17e72fddfbf0574d1fa39f04b85d319327b876e750273a6|
|web/src/app/session-spawn-state.ts|5135|01c80143b903ed2ff52c166fd76c233047afed9c146161cc81b1e7455393ac7a|
|web/src/app/session-spawn-tracker.ts|5153|3b7520cb727437ebfb3fff1ff39cf23c00ffe6980349f4175e27285d5325fa47|
|web/src/app/session-strip-branch.ts|1723|c6d753343fa75d50cbeb54536a364df899db416cb1caf2f547d0a0a18760e8ac|
|web/src/app/session-strip-fixtures.ts|2992|292efcad0b29cdb89475bb8b46d6b741a85a6b99d1d0dbac5b8e26b847f14fbe|
|web/src/app/session-strip.ts|24096|810d466bdafec1e25736236ba51c98a918694b8c96e6c949f96f1d3052bc95d6|
|web/src/app/session-swipe.ts|3291|930c7aa7f481fde1fa828901adab7494051cbb2b58708b4a761aba99284a37cb|
|web/src/app/session-view-position.ts|1864|97b7ad850c496bb55ceaeb630b640bf65b8ccb0f37195c97f175ea04fd5f6728|
|web/src/app/session-view-preferences.ts|2149|fc1db77ec880d4ca8a54117e5d7096c5c4f0129f765362ca6be17a485297bf93|
|web/src/app/settings.ts|194413|fa144d37a058a4ed4f65892d3348e9cd151bd576221a346c48f8c2713d07b370|
|web/src/app/shared-template-sync.ts|1038|6c2f8f61d6a7c9f8b2dc92fb78b102aca5253a85941f21579d5d2f6f1fc4427d|
|web/src/app/shortcut-help.ts|4353|5c2eac1ae250cf77e571208570b6740494e1912a6afd930ec8f56c9d6771bc14|
|web/src/app/sidebar-tree-fixtures.ts|11695|985f1c6bbf48fa22d823e983f961a156ff6c36bedd54c65dc2c59d480dcc6d28|
|web/src/app/sidebar-tree.ts|14744|15921e66f57d9fcf897aa2144c4993a2b006233ad26aab628d00f1c1e53635ae|
|web/src/app/spawn-approval-memory.ts|11953|b82a357adb6dcebf07d9ac0f14b8496bbe4aabe50df835fd6b3da17ba6f20271|
|web/src/app/spawn-confirm-store.ts|22460|7ac997923b080dd031d2dc9c2ab3a3741c2463f1d59cda0fc9445cd4463498bd|
|web/src/app/spawn-confirm.ts|32496|d096431167c7c4e1478fcbafea03f34feaeb1ddc746f0ed5a14d0c9eb6ef3c11|
|web/src/app/spawn-model-groups.ts|5598|b2308c2d452294e396e94a0f992c8571cf8b6ff45e834e4ff1d398a61c500e49|
|web/src/app/spawn-panel-fixtures.ts|13319|ebcd61c9da99f12885701a3d619cdb682b13e6f978d8d1096f0b34b48e2a97e2|
|web/src/app/spawn-panel.ts|144146|2109b2b363df70ab32b22e6b83d7816baf02e4e9916f2f0d4e7ffbdb58f5cdbf|
|web/src/app/spawn-provider-options.ts|2396|b64fe23b6ed5f2c3ce7b53f89e90e8b7ea09413d0ef07dbf2667484c5c054ab6|
|web/src/app/state.ts|19948|2075f33f62a6d6283743b8906eac509a9021a44ee5591e81849ad60c98e29696|
|web/src/app/subagent-tree.ts|6293|5084991ae3efdcb9b82e34262e953bcc29047dfef74ce589ffc401e5fd56f759|
|web/src/app/subscriptions.ts|20206|cb27b7ef04a6e9ce632083c1ad7fc046758f50349ee789a6a79e2d0fdbbc495f|
|web/src/app/tab-bar-order.ts|7519|8afa2fbacece16dcbc01cbb0e3752006195759f16193c32c49b4a01e846ba3ce|
|web/src/app/terminal-history-strategy-fixtures.ts|4407|f7ac75742983f4c560296c6a5fc60fd3c19472a3c77b1813b61d8bf4ea105223|
|web/src/app/terminal-history-strategy.ts|5231|b8c1299d6e0474ca4eebe3c8b7cab4092ef5c09bb0ff147874c190847f255cfb|
|web/src/app/terminal.ts|123690|8ba584e2aae7d263919900a70e1e80b1ddf28144ddc77c7c51d152f4bea41169|
|web/src/app/theme-tokens-fixtures.ts|2645|1fde75144ab089950dbe18ba54b73ba72d5cda6e2fa400f613eca46093ea5712|
|web/src/app/theme-tokens.ts|9487|8b0b067e885d26ca3053f36b63eae81b28ebe8b7152970bd6e24cd91bdbb14ae|
|web/src/app/token-statusbar-owner-fixtures.ts|3005|755737ac6571da595f341868051a29c664c345403fdcb56c0c8e0285eaffd1d9|
|web/src/app/token-statusbar-owner.ts|893|93c516674befac3a687a39c9261ad5b246b55fe3a8b3333a93e3e528c621c7a8|
|web/src/app/token-statusbar.ts|63157|7f541bd67e181692f4878184e6c5a17b99964a9478720709d9b2021a49f04af1|
|web/src/app/transcript-message-fixtures.ts|3108|b998de0382a34b66e438d8748d501260e94bc2ac95763d1f5916712ec2017cf7|
|web/src/app/transcript-message.ts|4536|6161280cd188986bc636e40f22dc2d906555fc8fecdc0455a10f4bcc46bf5a28|
|web/src/app/turn-end-notify-store.ts|4604|5c4bece76b82daf64895409ef41f128ccfea6ab66c203eb3d07c4aa8befcf809|
|web/src/app/ui-side.ts|6518|2613ee5eef254a7aafca6983e44d3780b39c2506de4e6aa7c2ec8192fdf71fba|
|web/src/app/url-detect-fixtures.ts|11096|bf466cc2239ebf423ee2839f614cab2126a867e77756da95e2607d11235ef5db|
|web/src/app/url-detect.ts|6194|1b00908447829c4b2697032a11152ac19463108d7ed554cbc37285331e02366a|
|web/src/app/url-links.ts|6370|a8573fa49c0342f0337b75675b27cdc24225207c3598508becc5134ebf7a52ea|
|web/src/app/usage-limit-fixtures.ts|2687|c7cd034d0b99109084587aadcff82412c64f54ff751aaeffa9acd03c0756594a|
|web/src/app/usage-limit.ts|3538|56a944b2cf6e7f3ba7eecd7e901dee8b407f9289be5bb89d0e1f9f18e41f092f|
|web/src/app/usage-panel.ts|45434|51202f46f6e29b5400615d1b88637fa6d8dfe6bf5f0a6bed99b0dccc9eadc63a|
|web/src/app/user-prefs-put-queue.ts|988|e3c96e0abe36f4726748a58f7f05a4d1f2f0c565ae11e1f28fa165a85e14305f|
|web/src/app/user-prefs.ts|38045|1085387ae64d111358b8e9160c4dc44cd8db1b7eca9c32c19c31ee9f4ed5653e|
|web/src/app/util.ts|12359|28d493f72cdde1c4c08e08bf9b2eec0653ddb554299107398108e4823e993476|
|web/src/app/voice-engine.ts|2374|28333dd430d55d4e2f053ab4aa5270c368421bae7714ad931547a30be25fdc64|
|web/src/app/voice-whisper.ts|11432|e2ae8f4fb57a004f29562c35077a9f56f4195a30687ce7646fb74d3c9f6e89af|
|web/src/app/voice.ts|15714|963a2ba682cda12f693387e2d966ec9ae0375daad97b21cbf34feaba43ea38a4|
|web/src/app/web-menu.ts|2811|69c044bf832b9bb9d6a10a2dad673f0245532167a02975a6634c0c56b52b58e7|
|web/src/app/window-channel.ts|3634|5f987ba987fcbfc315348cbbf3bfd5588de8a01bc6db512a1f3ddf301dbfb5d4|
|web/src/app/workflow-modal.ts|45261|edecd5b0b758bfb56cf4c928bc158d21a80e8d7bf5810afae52817abe4f50591|
|web/src/app/workflow-progress-fixtures.ts|11990|d02b32d3ad3c8094411c8ab64ea29eed038ed74d4b012cddc42b252fcd44abe3|
|web/src/app/workflow-progress.ts|17157|b946b8149b8e12977380811c226033d93d0d6939a14c29ec1f14bf5b6d1274dd|
|web/src/app/workflow-store.ts|6095|2de301c63f9aa455249a24391c3777b8ddf7e11b4d9e6ea06a530f8620b9ff79|
|web/src/app/ws-client.ts|41640|bcb92aa906dbc43a4f50c2cfad46a1c52ceaa3354b5c80ddd5a9e6f76911e743|
|web/src/app/zero-session-empty-state.ts|12034|fc7820cbe09b7349d1286b66b3dbc7a3e34541e2b0f1c64eefcb1a3ad97eb7e8|
|web/src/i18n.ts|2733|51b3324cb520b4965146b9b183e4fe282c94c7f4803079579b41b356c5df5241|
|web/src/i18n/en.json|154907|ffdce760a13443a2248cec7fd93c8ff61de3a7cffcbbc5f1c801975924ed2f84|
|web/src/i18n/ja.json|182585|9f4f15fc32d379002e680c50b137082201dcfa83e55df06a288e40ef06f7e682|
|web/src/i18n/vi.json|173548|f95b79500a5255d8797fb54bcb28cc5323d39e2e70d2b9401d7f3f296d292807|
|web/src/index.html|177188|ebfce2ac555c95749dfcfa65086b7c7273ddcafaa6a3a3fc7477c9fd3aa8bf95|
|web/src/styles.css|127813|2d986ff7c635d9192e8062b22fd8f9b3bc56b55db62ada078742fe3d343f374b|
|web/src/styles/approval-queue-tab.css|10413|e487a0926b8a9190cef04a5b97c3fb37c8445b09f3fb3bf9d130055c9111e0ac|
|web/src/styles/approval.css|37213|87720d8a334ea7bf702731b9bfad02edd53e6a0ee53c0eb84918a8194c6759d3|
|web/src/styles/bug-report.css|4761|221808cd664a5a6e790a45c376fc14e949c947522a5a30a3c21fed7cb389cdd1|
|web/src/styles/chat.css|21104|ed014b82e3367fce73fdded4688c7ef0a34cd584fb92aa3ddf2baed2c649fba5|
|web/src/styles/cli-maintenance.css|7670|49db16907c2dc28d6f2bc4fb421c83af109091c7c0c29dca7889e58251873741|
|web/src/styles/close-button.css|6176|ea1ee3cbd4ecbe62301aca15e3e15b32797fd905d74e7d17e17d193cfbe816e6|
|web/src/styles/detached-grid.css|5062|012be5513c8c380b56444205a5010bd9c6efd14404fbe237d8bdd1fc2f6aa894|
|web/src/styles/detached-view.css|2118|94ce1aa52af77d09f92679f15d54336311b1a20c664aa2db45bebead05c05720|
|web/src/styles/files.css|24149|e1cdeb2f1ea863626170e56fd539b7c359cc26a95cc6265493383ca9c141925b|
|web/src/styles/first-run-tour.css|2100|67c9d61182bc0fc915b19e93c30c799a8fed202a88f6744c94d78e755317a388|
|web/src/styles/git.css|23903|5fe15d744d949601d94a16a2e0c80647c7ca36882e6ffc9ea4ad9e187ff86816|
|web/src/styles/history-lite.css|2039|beb57a2ad99106fd8345c778b682cca0e622e5e7a46be1beed4973231b59ff3a|
|web/src/styles/host-expose.css|3563|2d0a300e83fb634a467e2fed145bfe1ff10de3f36cd845849d6babf9f62f7554|
|web/src/styles/memo.css|7709|21ea245c0ada82f3212e2876659ab1b7f08f2ae876fda17166cfad06a4b83664|
|web/src/styles/mobile-approval-only.css|2689|102976fda0b91220f33b7ca6306a5ab54514d5467cc9cc926b54999051fca2d7|
|web/src/styles/mobile-connect.css|15388|ac2c6cc493053ba9975d74e9193f6f7d536f227ef3c6279ede803831a56ab015|
|web/src/styles/mobile-home.css|28621|91d54b0a143db81cceda0f794e2edd947d53c3937b86d8aa097eb19dcf76eb7b|
|web/src/styles/mobile-short-nudge.css|1293|5cf05866bb98afda1e089a855f37793a0b738f7d076ba0329360610d54c0ded7|
|web/src/styles/pane-placement.css|7007|aa65f66faf00fa7a759d5186ad49a0b7188bb788e8e7be20fe621e4ff87f39ed|
|web/src/styles/prompt-templates.css|6144|04cf6f9a571f7daa2da9c5613114c7fc88b022dd39d095e070b9b0157c19f13d|
|web/src/styles/review.css|9275|b971453f5e66de188f6fe471bbbb1b912ec3babf62dacd9c110e6dbf0df9f214|
|web/src/styles/routines.css|2920|2204f7a02faee7601e6e2703c67b2fc0cdf01ea818407a4c32f86e47944d2e82|
|web/src/styles/server-modal.css|6877|1d2c5faaaf6632f195550b486d5b677f4a0c1cf374a53d0b0d87923e75605317|
|web/src/styles/session-list-tabs.css|4322|287a3bf7911fa7b0231e4789896d97384276cb833ea2392c72665179b07f47ff|
|web/src/styles/session-search-palette.css|5534|67500def1a1b94681a19c4ff9e6cf5b83aac39e1d7e6b32cbb0e278b64b006ba|
|web/src/styles/session-strip.css|7621|a4a8529c647137c55b222a5089171e565ef881e997172a247ff5ea2f8bbdedb3|
|web/src/styles/settings.css|31199|190951e15f7546013804ffca62ee47579a442567f3d31d1a9aacfcbe31ba5cc1|
|web/src/styles/shortcut-help.css|1885|b7404c3ace5239272454c8ae5cfae6766be60dd5f11c58b6b397025aa488c9c9|
|web/src/styles/spawn-confirm.css|9555|a0ddf5087b85693dc391fb06de13a06dc11fa9119f5ef06f21c67d20fb6a42a0|
|web/src/styles/spawn.css|29138|ef4c1d7cd6ad1de2b94c07b7214ba2dbe35d7edae45dca5dfb48857b31c154af|
|web/src/styles/subscriptions.css|3507|c2e94dc00c520df13782dee00f19233fa9945902f6c71bff29333bda8620425c|
|web/src/styles/tabs.css|24299|a8d76d70c7b3bb973d16f1c37f31525316202f1deda705c6d357883f2b71c524|
|web/src/styles/terminal.css|48235|d8e6201a8c19fa696f8343282cf637bc0d09a998afdc343fefbebfa548e2103a|
|web/src/styles/token-statusbar.css|18862|0706821d359e4bab4804f4ed0a716d6d3cea1e6d3aef3a57a5216e89329eb187|
|web/src/styles/voice.css|6003|2180c304adf0cdc11326cc67ac5be178180e93b39bbfdde6b586a7c1efe6e503|
|web/src/styles/workflow-progress.css|12158|574d61bc8ac373be3170dc62fdea219cf308f6bfc0e3d1f65b26fc9d6bda49c8|
|web/src/styles/zero-session.css|5976|8e6763574e340134c107d7a6e18a5a15b6a1685cd798deed40e92bc61011690b|
