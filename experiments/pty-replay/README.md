# #1 Rust化構想: PTY replay 最小比較キット

`ptyReplayBuffer.append` / `snapshot` だけを独立した Go / Rust EXE で比較する準備用キット。**Draft: ビルド・正しさ検査・Windows 実測は未実施。性能改善はまだ確認していない。**

## 対象と出典

- 基準 develop: [`21d0bc7935a2c4696fb89ccff2e324157a528c2d`](https://github.com/ishizakahiroshi/many-ai-cli/commit/21d0bc7935a2c4696fb89ccff2e324157a528c2d)。実装開始時点の develop と同一で、差分混入なし
- Go 基準: [`internal/wrapper/pty_replay.go`](https://github.com/ishizakahiroshi/many-ai-cli/blob/21d0bc7935a2c4696fb89ccff2e324157a528c2d/internal/wrapper/pty_replay.go)。出典 blob / SHA-256 と独立化の変更を [provenance.json](provenance.json) に記録
- 2 MiB 末尾保持、捨てた分も含む `total`、Mutex、snapshot の独立コピーを両版で維持。UTF-8 / ANSI を解釈しない
- Go は原処理をコピーし、package / import / 定数の参照だけを変更。Rust は safe Rust の Mutex と連続領域＋先頭 offset。ring、SIMD、unsafe、独自 allocator、ロック除去は使わない
- **ここが製品のボトルネックだという証拠はない。** PTY / ConPTY、Hub、HTTP、WebSocket、VT / UI、承認判定、AI CLI は対象外

Go の `bytes.Buffer` と Rust の `Vec` は容量拡張・コピー・allocator・runtime が同一ではない。比較は「この 2 実装」の比較であり、純粋な言語差とも製品全体の改善とも呼ばない。FFI では Go runtime が残り、別 EXE 化では IPC と追加 process が必要になる。

## 分離

すべてこのディレクトリだけに置く。Go は独立 `go.mod`、Rust は独立 Cargo project で、製品の source / imports / probe / endpoint / Makefile / CI / 配布設定は変更しない。既存の製品ビルド・root `go test ./...` はこの Go module を含まない。製品へ観測点を仕込む変更でもない。

`bin/`、`fixtures/generated/`、`results/`、`rust/target/` は Git 対象外。生成 EXE、測定ログ、PC 情報、実セッション本文を PR に入れない。製品への組込みや製品内の観測点追加は別の合意と既存の instrumentation / purge 規約を要する。

## 実行前に

[LOCAL_HANDOFF.md](LOCAL_HANDOFF.md) をローカル AI と確認する。リポジトリの `AGENTS.md` / `CLAUDE.md` とローカル補足が優先。**ビルド、検査 EXE の実行、計測はユーザー担当。AI はコード準備・静的レビューと手順案内を行う。**

必要: 同じ native 64-bit Windows PC、PowerShell 7、Go 1.26.8、明示的に版を固定した Rust stable / Cargo と対応 Windows linker。ツールは自動インストールしない。WSL EXE と Windows EXE は混ぜない。製品の通常ビルドは `make build` だが、本キットの独立 EXE は下記の専用スクリプトでビルドする。

AC 電源・同じ電源モード・同じ architecture を使う。Defender やセキュリティ設定を無効化しない。他アプリを勝手に終了しない。計測中のビルド、AI 推論、大量のブラウザ操作、更新、sleep を避ける。不要な調整を片側だけに加えない。

## 比較仕様

[scenario.json](scenario.json) が初回の固定条件。入力は合成データで、日常の実測 trace ではない。安全な実運用 trace に替えるなら別の実験として hash と境界を記録する。

| 条件 | 仕事量 |
| --- | --- |
| paced | 1 process / 1 buffer / 1 writer、1500 chunks、50 chunks/s、30 秒。64 B:60% / 512 B:30% / 4096 B:10%。t=0 から絶対投入予定時刻を使い、遅れても入力を捨てず最大遅延を記録 |
| saturated | 同じ mixed fixture を待機なしで N 回反復。両版同じ N・bytes・ops。速い方も 15 秒以上となる N を pilot で確認して本測定前に固定 |

各 process は 3 秒以上の warmup 後、新しい buffer を 2 MiB に満たして開始する。毎 chunk snapshot はせず、最後の 1 回だけを計時に含める。warmup は同じ列を使うが速度により反復数は異なり、測定 bytes に含めない。

### 正しさの gate

両版が同じ生成済み fixture を読み、checkpoint ごとに全 snapshot bytes と total を単純な連結→末尾切出し oracle に照合する。詳細は [fixtures/README.md](fixtures/README.md)。空・境界・超過・UTF-8/ANSI/NUL・固定乱数・入力/返却値の独立性・並行 append/snapshot を含む。

検査が両版とも成功し、hash・入力・仕事量が一致してから性能値を採用する。不一致・クラッシュ・欠落を速度の勝ち負けで相殺しない。FNV-1a64 `output_hash` は処理結果の補助照合で、SHA-256 でも全 bytes correctness 検査の代わりでもない。fixture / EXE / source の証拠には SHA-256 を使う。

### process 制御と境界

`READY → WARMUP → WARMED → START → MEASURED → CHECK → DONE → EXIT`

READY / WARMED / MEASURED / DONE は EXE の stdout JSONL、WARMUP / START / CHECK / EXIT は controller から stdin へ送る command。

1. fixture 読込・入力検証・期待末尾準備後に READY。起動→READY を別記録
2. WARMUP で warmup、buffer 再初期化後 WARMED
3. START で append と最終 snapshot を EXE の単調時計で測る。JSON / disk / checksum はこの内部 wall の外
4. MEASURED で controller が PID CPU と外部 wall の終点を採り、CHECK を送る
5. 全 bytes 照合と hash 計算後に DONE。採取が終わってから EXIT。終点採取前に process を消さない

PID CPU / external wall には START の送受信と MEASURED の小さい JSON・scheduler 遅延が含まれる。CHECK 後の oracle / hash は含まない。EXE 内部 wall と外部 wall を同じ列に混ぜない。起動測定は warm-cache 条件であり、cold boot の測定ではない。

### pair と汚染

- 各条件 10 pairs、Go→Rust / Rust→Go を各 5 pairs、seed 固定で順番を混ぜる。各走行は新 process、Go/Rust 同時実行なし
- pilot は両版・両条件各 2 回。main は pilot の source / fixture / EXE / 条件と照合する
- process 250 ms / machine 1 秒を目標に採取。実際の時刻・遅延・欠測理由を残す
- 開始前 10 秒の CPU median <10%、available RAM >20%。実行中 RAM <15%、概算の他負荷 >15% が 3 秒以上、counter 欠落、仕事量不一致等は pair を無効にする。電源変化・別作業・更新等の手動除外も記録する
- 片側だけを捨てず、元の pair と理由を残し、両版を新 attempt として再測定する。10 有効 pairs がなければ結論保留。勝つまで入力や条件を変えない

閾値は初回の測定品質ルールで、Rust 採用の基準ではない。採用閾値は実測前に別途合意する。主要 operation latency の診断や複数 EXE 群は初回キットの対象外であり、この結果から未測定の p95 latency を主張しない。

## 記録と読み方

各実験の `metadata.json`、`schedule.json`、`correctness.json`、`runs.csv`、`samples.csv`、未加工 stdout/stderr、`summary.md` を保存する。取得不能な値は 0 と見なさず null / 空欄と理由を残す。

- PID CPU: user＋kernel の CPU time。CPU 秒/GiB、平均 core 数、1 core 基準%、PC 全体基準%を区別
- Working Set: shared を含む常駐ページ。Private Bytes: 非共有割当量。互いを同じ RAM と呼ばない
- process RAM は runtime / allocator と、共通の入力・prefill・期待値・snapshot の保持領域も含む。Go の chunk slice と Rust の範囲配列にも表現差がある。対象 buffer だけの RAM と呼ばず、差を製品へ外挿しない
- sampled memory median / p95 / max は採取点の統計。OS の PeakWorkingSet は起動・warmup を含む lifetime 最大で、定常 peak と別
- machine CPU / available RAM と controller CPU は別負荷。対象 EXE の消費と断定しない
- summary は条件別 median / run 間 nearest-rank p95 と pair ごとの Rust/Go 比を出す。10 pairs の p95 は粗い参考値。run p95 と memory sample p95 と operation p95 は別物
- 改善は % だけでなく平均 core と MiB の絶対量も確認。境界的な差は同条件の別測定日で確認

共有する前に metadata と stderr の PC 名・ユーザー名・パス・環境情報を確認して匿名化する。生ログはまずローカル保存し、公開 PR へ自動送信しない。

## 検証状態

この Draft は静的読解・出典差分・入出力契約のレビュー段階。Go / Rust / PowerShell の formatter/parser は準備環境に無く、構文ツールの合格も主張しない。ビルド、correctness、pilot、本測定、製品 regression、Windows 実機は未実施。root CI が成功しても独立キットのビルド・動作を保証しない。
