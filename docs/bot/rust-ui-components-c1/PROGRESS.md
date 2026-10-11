---
type: reference
status: draft
tags: [rust, ui, handoff]
owner: local-conductor
review_status: draft
related: [README.md, FACTS.md, REVIEW.md]
last_reviewed: 2026-10-11
---

# #P-20261011-005 many-ai-cli：C1資料の入口

> 最終更新: 2026-10-11(日) 19:38:10 JST

管理Issue: [#16](https://github.com/ishizakahiroshi/many-ai-cli/issues/16)。送信・受付・調査提出・独立レビュー・手元検収の状態遷移は、このIssueのコメントに集約する。

## 指示と証拠

- [作業指示](README.md): 目的、五軸の棚卸し、許可/禁止、提出形式。
- [確認済み事実](FACTS.md): 公開source SHAと手元との差、停止モーダル、既存共通部品、自動CIの範囲。
- [独立レビュー手順](REVIEW.md): sourceと集計を別担当が検証する。第1巡と修正候補を記録。
- 調査成果 [INVENTORY.md](INVENTORY.md): source棚卸し候補。再集計は [recount_ui.py](recount_ui.py)。独立レビュー第1巡の結果と修正をREVIEW.mdへ記録。新成果SHAの再レビュー待ち。
- 最初の指示公開commit: `fad045cba803e7827d31cdaddcb4f333f1f39f32`。レビューdiffの起点。管理Issue追記後の最終固定指示URLはIssue本文・送信文へ載せる。branch最新版URLと区別する。

## 完了していない受入

1. dots受付・source棚卸し候補は提出済み（最終受入とは別）。
2. 公開SHAの全画面名簿、五軸の分類、再現できる計数、停止モーダルの契約一覧。
3. 調査担当と別担当によるレビューと、修正後SHAの再確認。
4. 手元AIによる全差分・集計の照合、普段のビルド元にある追加変更の確認、非公開棚卸しへの反映。
5. 実画面の受入。sourceの相違を実画面のばらつきと断定しない。

次工程はC2の仕様資料と利用者確認。C3/C4の製品変更、C5の機械検査、ビルド、稼働版への反映は今回のC1成果では完了しない。

常設監視/自動配送の接続は今回の依頼で起動しない。手元担当のセッション中の直接読取りと、セッション終了後の監視を区別する。

## dots C1 調査候補

- 受付完了。固定指示6f24b79、source e1ab622、Linux/Python3.12.14/Git2.52.0/rg15.2.0/Node24.19.0。
- 指定215件＋入口3件=218件を固定読取。41画面群・五軸・停止契約・C2未決定事項と手元受入手順を提出。CSS集計は宣言ベクトルであり、実画面の最終種類数とは区別。
- 再集計self-check 6成功/0失敗/0skip。固定SHA再集計成功。CSSのEOF未閉鎖2 warningを明記。
- 既存wheel静的gate成功（33件/免除1件）。instrumentationは最初archiveでGit無しexit1、その後固定SHAの隔離Git checkoutで成功（台帳17/active2）。過去失敗を隠さず、後者が実施結果。
- 依存追加・製品変更・製品build・Hub操作なし。Windows/実画面/mobile/手元未公開差分は未検収。
- 別contextのsource先読み・集計一致確認済み。成果61daa7fの全差分を独立レビューし文書5項目を修正。新成果SHAの再レビューは未了。PR/CIは提出後にIssueで実行結果・skipを区別して報告。

C1提出は親計画の完了ではなく、C2仕様確認前にC3以降へ進めない。

- 指定base向けDraft PR: [#17](https://github.com/ishizakahiroshi/many-ai-cli/pull/17)。61daa7fのsecret-scan成功、Validateは別途実runを照合。CI結果の正本はIssue16。

- 第2巡7f80b94では状態SVG共有helperの名簿、launcherのclose/launch順、shortcut helpの閉じ方を補正。計数/source不変、新SHA再レビュー待ち。7f80b94のValidateにはgovulncheck失敗・release-token-scopes skipあり（残ジョブ進行中時点）。成功扱いしない。

- 追加依頼: §3.1.1に直書き色から既存変数への代表12行対照を追加。dark/light/custom、alpha、provider/状態/Git/footerの意味差を保留し、自動置換・C2採用決定は行わない。全196色の分類とは区別。
- 手元報告: 61daa7fのWindows再集計（218件、fixture34、hex673/196、rgb320/161、font878/54、button557/398、overlay241/176、input164/124、SVG37、img3、inline311、self-check6）一致。手元追加24変更pathの確認は手元担当の別証拠でありdotsの実行とはしない。
- 独立レビュー: 7f80b94まで確認済み。54c4002の最終差分読取は審査cancel、許可根拠付きの同一呼出1回もcancelで停止。追加追補を含む新SHAは独立未確認。C1 source棚卸し合格・実画面合格はまだ出していない。
