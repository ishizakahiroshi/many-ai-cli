# #3 many-ai-cli rust: execution instructions

> 最終更新: 2026-10-03(土) 12:44:27

このディレクトリを開始指示として受け取ったdotsは、現行Go版の全機能をRustへ移植し、両binaryが使える状態まで実装・検証し、develop向けPRを作る。作業の途中で [PROGRESS.md](PROGRESS.md) を更新する。Codexが実差分と証跡を独立レビューするまで、全体完了やmergeを宣言しない。

## Goal and baseline

Goの基準SHAは `21d0bc7935a2c4696fb89ccff2e324157a528c2d`。実装branchは `feat/rust-migration`、PRのbaseは `develop`。default branch mainへ直接変更を入れない。instruction-only commitは基準Go sourceを変更していない。実装前にこのSHAのREADME/source/testsとAGENTS.md/CLAUDE.mdを読む。

本体 `many-ai-cli` と `many-ai-cli-launcher` の両方をRustにする。既存Web TypeScriptは継続使用し、Go sourceと既存Web機能を凍結する。RustからGo本体へ処理を渡す実装、少数のAPIだけを備えたdemo、成功を返すstubを完成品にしない。全15契約のentry/caller/wire/persistence/OS境界を移植する。

利用者の操作互換、データ保全、認証/プロセス境界、配布/切り戻しの4軸を守る。Goで現在修正済みの挙動も固定SHAのfixturesから保持し、意図的な変更は利用者への影響と回帰testをPRに書く。

## Read and execute in order

1. [00-contracts.md](00-contracts.md): 全工程の不変条件と追跡表。
2. [01-foundation.md](01-foundation.md): 共通型・CLI・隔離root・dependency/build基盤。共有APIを固定する。
3. 固定後に [02-core.md](02-core.md)、[03-services.md](03-services.md)、[04-launcher-delivery.md](04-launcher-delivery.md) を排他的所有で並列実装する。
4. [05-integration-review.md](05-integration-review.md): 全機能のwiring、検証、PR、Codex独立レビュー。
5. [06-cutover.md](06-cutover.md): 複製データの復元と実機受入後、利用者が切り替える。

実装する担当は各指示書のsource oracle/fixturesを実際に読む。指示が実コードと食い違えばコードを正とし、進捗に食い違いを書いて親に伝える。架空のfile/function/APIを作業の根拠にしない。API変更や機能省略をREADMEだけで覆わない。

## Ownership and execution

共有Cargo/lock/lib/build.rs/binary entry/proto/config/process interface、Git、dependency更新、統合buildは親の単独所有。C2はterminal/storage/approval/transcript/wrapper/orchestration/headless/handoff、C3はhub/files/profile/update/routine/memo/notify/voice/tray、C4はlauncher/deliveryを所有する。同じfileを複数agentが編集しない。変更要求は共有担当へ返し、他laneの編集中compile errorを勝手に直さない。

この指示の実装・ローカルの隔離build/tests・PR作成は利用者が承認済み。稼働中Hubの停止/再起動、実credential/実データの利用、課金AI probes、外部通知、定期Usage、ソフトの新規install、破壊的schema移行、release公開を実装工程へ自動で追加しない。実機環境が無い項目は未実施とし、利用者へ必要操作を具体的に渡す。

WindowsではPowerShell7またはフルパス指定のGit Bashを使い、WSLを起動しない。私有config/認証/環境変数を全文列挙しない。secret、端末名、実ホームパス、実顧客データをcommit/PR/logへ写さない。fixturesは合成データ。

## Instruction revisions and acknowledgment

Initial instruction commit: `6b0fb8e198750245ef8b4b475fbac9070db0ac3e`. The Go behavior oracle remains `21d0bc7935a2c4696fb89ccff2e324157a528c2d`. Use commit-pinned instruction links in the start request; use the branch URL for current PROGRESS. Later instruction supplements must be identified separately and do not restart the task or erase its earlier receipts.

At acceptance, report the recognized coordination number, collision check, instruction commit read, and whether dependencies/tests are executable. Before recording the next checkpoint, check for published instruction/progress supplements, incorporate them without overwriting concurrent work, and state which revision was read. Preserve the original implementation scope and any separately approved environment decisions.

A successful send, a read-back of the posted text, dots acknowledgment, dependency fetch, test success and real acceptance are distinct evidence states. Record only what was observed, with its source; label bot/environment reports as self-reported until verified. The operator verifies message body and acknowledgment in the same conversation. Keep private conversation links in the operator's private record rather than public GitHub documents.

## Progress and PR contract

各工程の開始・停止・実装完了時とcommitを作るときにPROGRESSを更新する。記録するのは担当、状態、具体的changed paths、実行commandとexit code、結果を証明するcommit/CI/artifact、未実施の受入、次の一手。本文やsecretを記録しない。

状態は `pending` → `in_progress` → `implementation_complete` → `reviewed` → `accepted`。`blocked` には原因と解除に必要な入力を書く。子の完了、test pass、PR作成、手動受入、切り替え、安定稼働を同じ状態にしない。

指示書SHA、実装コードSHA、レビュー対象SHA、看板更新commitを別々に記録する。看板自身の更新commitを同じcommitの本文に埋め込もうとせず、GitHub履歴から読むか後続のreceiptで記録する。

PRはdevelop向けdraftから開始し、全Kと確認方法、意図的な互換差、未実施実機項目、データ復元方法を自己完結で説明する。Codex独立レビューで指摘された欠陥を実装と回帰testで直し、差分と再検証receiptを更新する。レビュー前のmerge/tag/releaseは禁止。release v1.0.0は実機受入・復元実演・安定稼働の確認後に扱う。
