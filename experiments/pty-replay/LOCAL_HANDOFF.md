# #1: ローカル AI / ユーザーへの引継ぎ

## ローカル AI が先に確認すること

1. `AGENTS.md`、`CLAUDE.md`、関連 `CLAUDE/*.md`、存在するローカル補足・関連 skills を読む。HEAD / branch / dirty diff を確認し、利用者の checkout を勝手に切り替え・reset・破棄しない。このキットの Draft PR の専用 checkout を利用者と決める
2. 基準は develop `21d0bc7935a2c4696fb89ccff2e324157a528c2d`。`provenance.json` と Go 原処理の差分を確認。後続 develop の変更を黙って混ぜない
3. 初回は formatter / parser も未実行。Go は対象ファイルを名指して `gofmt -e`、Rust は `cargo fmt --check`、PowerShell は `System.Management.Automation.Language.Parser.ParseFile` などの構文確認だけを行い、指摘があれば修正案を示す。構文成功とビルド・動作成功は別
4. リポジトリ規約どおり、**ビルド・EXE 実行・計測はユーザーが行う**。AI は下のコマンドを案内して結果を確認する。追加の build/run 権限が必要なら利用者の指示を待つ
5. 本体統合・Hub 起動・本体 `make build`・インストール・security 設定変更・merge・release は含めない

公開差分はこのキットだけ。入力は合成データのみ。Draft の静的レビューを済ませてから、以下の順で進む。

## ユーザーが実行する順序

native 64-bit Windows の PowerShell 7 を、この `experiments/pty-replay` ディレクトリで開く。AC 電源、電源モード、他作業停止を確認。未導入ツールがあれば自動インストールせず、公式導入方法を確認する。

### 1. fixture と optimized EXE

```powershell
.\prepare.ps1
$rustVersion = ((& rustc --version) -split ' ')[1]
.\build.ps1 -RustVersion $rustVersion
```

Go は `1.26.8`。Rust はこのコマンドで取得した完全な stable 版を `build.ps1` に明示して記録する。以後、同じ実験の途中で toolchain / flags / 環境変数 / source を変えない。root 製品とは別 module のビルドであり、製品 EXE は生成しない。fixture / 計測の出力ディレクトリが既にある場合は既存結果を消さず、新しい場所を選ぶ。build は bin 内の生成 EXE と build metadata を更新するので、既存実験の再現用 EXE が必要なら再ビルド前にローカルで保管する。

### 2. correctness

```powershell
.\measure.ps1 -Stage verify -OutputDir results/verify-001
```

`correctness.json` と両版の未加工ログを確認する。全 bytes / total / 所有権 / 並行整合性が両版とも成功しない限り、ここで停止してソース修正へ戻る。性能測定へ進まない。

### 3. pilot で同じ仕事量を決める

```powershell
.\measure.ps1 -Stage pilot -OutputDir results/pilot-001 -Repetitions 100 -ConditionsConfirmed
```

`-ConditionsConfirmed` は AC 電源・固定電源モード・他作業を控える条件を人が確認した印。計測器が PC の全状態を保証する flag ではない。

pilot は Go/Rust の paced/saturated を各 2 回。`pilot.json` の `advised_repetitions` を読む。速い方も saturated 15 秒以上、counter 正常、汚染なしとなるまで、示された反復数で **新しい結果ディレクトリ**に pilot 全体を再実行する。例えば次の形式（N は実測 pilot が返した値へ置き換える）。

```powershell
.\measure.ps1 -Stage pilot -OutputDir results/pilot-002 -Repetitions N -ConditionsConfirmed
```

N を本測定前に固定する。短すぎる pilot を本測定値に数えない。数値が改善するまで条件を変える進め方にしない。

### 4. main paired 測定

受理された pilot のディレクトリ名と N を指定する。

```powershell
.\measure.ps1 -Stage measure -OutputDir results/main-001 -PilotDir results/pilot-002 -Repetitions N -ConditionsConfirmed
.\summarize.ps1 -ResultDir results/main-001
```

両条件各 10 pairs、Go-first / Rust-first は各 5。数十分を見込み、実行中に PC で別作業をしない。中断・counter 不足・汚染・一部失敗は未完了であり、成功へ読み替えない。元ログを残す。片側だけを消して再試行せず、この初期版では **新しい output dir に 10 pairs 全体を再測定**する。

### 5. dot へ戻すもの

- `summary.md`、`runs.csv`、`samples.csv`
- `metadata.json`、`schedule.json`、`correctness.json`、受理した `pilot.json`
- 不一致 / 失敗があれば該当 stdout / stderr と実行したコマンド
- 計測中の別作業・更新・sleep・電源変更があれば時刻と理由

まずローカルで内容を確認し、PC 名・ユーザー名・フルパス・秘密があれば匿名化する。GitHub に生ログを commit しない。dot はログの品質と正しさを先に確認し、その後 Go 継続 / Go 改善 / Rust 追加検証の材料をまとめる。Rust 採用、全移植、製品の節約量はこの段階で決めない。

## 期待する完了報告

- 変更ファイルと source / EXE / fixture hash
- 静的確認 / build / correctness / pilot / main / CI / Windows 実機を別々に記録
- 実際に実行したものだけ成功・失敗と書く。未実行は未実行
- 10 有効 pairs が揃わない条件や測定限界、再測定の要否
