# 合成 fixture

`prepare.ps1` が `generated/` に共通バイナリと明示的 chunk 境界を生成する。Go/Rust は同じファイルを読む。実セッション、秘密、個人データは使わない。生成物は Git 対象外。

- `cases.tsv`: `name<TAB>payload.bin<TAB>chunks.csv`。ヘッダーなし、相対ファイル名のみ
- `*.csv`: 1 行 1 個の十進バイト長、ヘッダーなし。空入力の case は `0` 1 行。合計は payload 長と一致
- `mixed.bin` / `chunks.csv`: 100 周期 × 10 chunks。各周期は 64 B × 6、512 B × 3、4096 B × 1。seed 42 の固定 LCG で周期内を並べ替える
- `prefill.bin`: 2 MiB の初期履歴
- `manifest.json`: 各ファイルの SHA-256 / byte 数、seed、generator と scenario の SHA-256

検査 cases は 0 / 1 / limit−1 / limit / limit＋1 / limit＋123 / 2×limit＋17、既存テストの 64 KiB 超履歴と 2 種の超過入力、1 バイト境界の UTF-8 / ANSI / NUL / 不正 UTF-8、空 append と snapshot 後の追加、同一の固定乱数列を 2 種の境界で処理する条件。実装内で入力/返却データの独立性と単一 writer・reader の並行整合性も確認する。

正しさは checkpoint ごとの全 bytes と total を、独立した連結→末尾切出し oracle と照合する。結果の hash 比較だけで正しさ合格にはしない。
