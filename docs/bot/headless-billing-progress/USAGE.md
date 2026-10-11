# 画面なしの子実行: 払い方と進み

> 最終更新: 2026-10-11(日) 02:05:00 JST

## 対象

受け入れ済み設計は commit `83dfa79` の DESIGN.md。初回の払い方確認は
Claude Code **2.1.295 のみ**。互換性の根拠と制限は [COMPATIBILITY.md](COMPATIBILITY.md)。
現在の CLI を更新・降格する機能ではない。未確認の版、競合する認証・route・managed
設定は推論前に止める。初回のプラン判定は first-party の Pro / Max のみ。

既存の他 provider の headless 起動は残す。今回、別払いへの切替やその払い方の確認は
行わない。取得できないターン・道具・発言・費用は未取得と出す。Codex の headless
除外と権限設定はそのまま。

## 1 回ごとの選択

子 spawn は `plan-login` が既定。API を使う子だけ `--billing-mode api-key`、
`--billing-profile <登録済み ID>`、`--max-budget-usd <正の USD 数値>` を指定する。
既存の `--execution-mode headless`、`--model`、`--effort` と組み合わせる。
API 要求は起動確認を必須にし、確認ダイアログでも払い方・登録済み profile・予算を選べる。
role の記憶や前回の設定へ払い方・予算は保存しない。
確認画面で新たに API profile を選んだ場合は、現在の情報を読み直すため一度止める。
閉じて開き直し、更新された内容を確認してから承認する。別画面の古い承認は受け付けない。

API の失敗からプランへ、またはその逆へ自動で切り替えない。API の timeout は
自動再起動しない。再実行するなら新たな費用を伴う別の spawn として確認する。

## ローカルの取り出し方

利用者が `~/.many-ai-cli/headless-billing.json` に **recipe だけ**を用意する。
trial では選択した root 内。形式は version=1 と profiles の配列で、各 profile の欄は
id、display_name、provider（claude）、command。キー値の欄は存在しない。
command は既に信頼した resolver の呼出しであり、キーのリテラルを埋め込まない。

- ファイルは本人だけが読める権限。共有・symlink・不正な形式は拒否する。
- resolver の成功 stdout は値だけ、失敗は非ゼロ。stderr に秘密を書かない。
- Hub は resolver を実行・試験しない。Claude だけが apiKeyHelper として実行する。
- UI へは profile の ID・表示名・provider だけを一覧として送る。
- 確認後に recipe が変われば、起動せず再確認を求める。
- 一時 Claude settings は実行所有の private file にまとめ、終了・失敗時に回収する。
- この作業で実環境の profile 登録や resolver 実行は行っていない。

API では `--bare` を使うため、自動発見される CLAUDE.md / skills / MCP などは省かれる。
通常の対話起動と同一の読み込み環境だとは扱わない。

## 見込み額の対象

初回の価格表は `claude-sonnet-4-5-20250929`、`claude-sonnet-4-6`、
`claude-opus-4-6` の正確な serving ID のみ（2026-10-10 確認）。
未知モデルや fast / 地域 / 長文脈等の未対応条件を既知料金へ当てはめない。
その場合も CLI が有効な最終費用を返せば終了値は表示できる。

## 表示の意味

カード・左一覧・複数 pane は同じ run の snapshot を表示する。

- ターン: 途中は主実行の観測数、最後は result のターン数。
- 道具: 実行中の名前と件数。引数や tool result は表示しない。
- 経過: 実プロセスの monotonic 時間。画面の再読込でゼロに戻さず、終了で止める。
- 費用: 途中は「見込み（観測した主実行分）」、未対応価格・内訳は部分見込み/未取得。
  終端の「CLI 結果」は CLI の最終算出値であり、請求書の確定額ではない。
  プラン時は API 換算の参考額で、追加請求やプラン残量ではない。
- 最後の発言: 完結した主実行 assistant 発言の、安全化した短い抜粋。

API 予算も見込み額による停止しきい値で、進行中の呼出しにより超過しうる。
厳密な支出上限として使わない。失敗の結果費用は失敗状態とともに残す。
終了した headless カードは確認できるよう保持する。入力禁止は維持する。
Hub 再起動後の headless 子の再接続・履歴からの費用復元は対応しない。

## 安全境界と検査

API の raw stdout/stderr ログと process capture は無効。未知・壊れた JSON、init 認証、
raw error、tool 引数/結果は公開しない。完全な assistant 文字列を秘密マスクしてから
短縮・escape し、全表示で同じ安全な投影を使う。

信頼した公式 CLI と resolver が前提であり、悪意ある command の別ファイルへの書込みや
provider 自身の非公開ログまで封じるものではない。検査は合成データと隔離環境で行う。
実キー、有料 API、稼働中 Hub、Windows の実画面は検査しない。
検査結果と対象 SHA は Issue 14 と Draft PR に記録する。

## 発言と認証経路の安全側の制限

最後の発言は、同じ応答の分割 record を私有の上限付き buffer へ集め、信頼できる
完了を確認してから応答全体を mask して表示する。API/auth error の本文は表示しない。
完了を確認できない場合は前の安全な発言を保ち、それも無ければ未取得とする。

認証・設定の検査と実起動の経路を一致させるため、別の設定 root / managed settings /
認証経路や debug 出力を指定する未対応の provider 引数は、推論前に拒否する。
