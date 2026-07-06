# [完了] Opus 4.8 1M コンテキストと statusbar の 200k 分母表示（調査結果）

> 最終更新: 2026-06-20(土) 10:36:42

## context配分

| C | 種別 | 内容 | 並列 |
|---|---|---|---|
| C1 | fix | Web ダッシュボード statusbar の ctx 分母が 200k になる原因調査 | — |

実行順序: `C1`

---

## 概要

Opus 4.8 を使っているセッションで、many-ai-cli の Web ダッシュボード statusbar のツールチップが「~85.4k / 200.0k tokens（43% used / 残り 57%）」と表示され、「Opus 4.8 は 1M のはずなのに分母が 200k でおかしい」と見えた件の調査記録。**結論：many-ai-cli のバグではなく、そのセッションが実際に 200k 窓で動いている実態を正確に映しているだけ**。コード変更は不要。

スコープ外：1M を常時既定にする運用変更や、statusbar 側で分母を底上げする実装（後述の経緯のとおり、底上げは過去に不具合となり廃止済みのため採用しない）。

## 現状と問題（症状）

- statusbar ツールチップ: `43% used (Claude statusLine) ~85.4k / 200.0k tokens 残り 57%`
- 85.4k / 200k ≒ 43% で内部整合は取れている。問題視されたのは「分母が 200k」という点のみ。

## 調査結果（根本原因・データフロー）

ctx の分母・使用率は many-ai-cli が独自計算しているのではなく、**Claude Code 自身が statusLine の stdin JSON で申告する値をそのまま中継**している。

- `internal/usagerelay/usagerelay.go:198-199`
  - `CtxWindow` ← `context_window.context_window_size`
  - `CtxUsedPct` ← `context_window.used_percentage`
  - 自前計算なし。Hub へ POST してフロントへ渡すだけ。
  - 同ファイル 128-130 行コメント：UsedPercentage は「Claude Code が実窓(200k/1M)を判定済みで算出した値」。
- `web/src/app/token-statusbar.ts:146-155`（`resolveEffectiveCtxLimit`）
  - `const relayLimit = entry && entry.ctxWindow > 0 ? entry.ctxWindow : null;`
  - relay が申告した実窓を **最優先**で分母採用。relay が 0/未送のときだけモデル名テーブル推定にフォールバック。
  - 147-151 行コメントに経緯：旧実装はモデル名テーブル（Opus 4.8→1M）と `Math.max` で分母を底上げしていたが、**1M ベータが効いていない 200k セッションで分母が過大→ctx% 過小表示**になる不具合だったため、申告値優先に修正済み。
- ツールチップ文字列は `token-statusbar.ts:478`、緑の「1M」バッジは同 472 行で `entry.ctxWindow >= 1_000_000` のときのみ表示。

つまり「200.0k と出ている＝そのセッションの Claude Code が `context_window_size=200000` を申告している＝実際に 200k 窓で動いている」が正しい読み。1M が効いていれば Claude が 1000000 を申告し、statusbar は自動で `/1.0M` ＋「1M」バッジに切り替わる。

### なぜ 1M になっていなかったか

- Opus 4.8 は 1M に「対応」しているが、1M は**自動では効かない**。窓を別途選ぶ必要がある。
- 現行 Claude Code では `/model` の **「Opus 4.8 (1M context)」変種**を選ぶと有効化（有効時の model id は `claude-opus-4-8[1m]`）。**セッション単位**の選択で、既定にしても効くのは新規セッションのみ。
- 検証（2026-06-20）：`~/.claude/settings.json` に env ブロック無し、環境変数 `ANTHROPIC_BETAS` / `ANTHROPIC_MODEL` とも空。→ 過去に検討した「settings.json env に ANTHROPIC_BETAS」経路は**未使用**と確定。
- スクショのセッションは「Opus 4.8 (1M context)」を既定にする前に起動していたため、200k のままだった。

## 結論と対処

- many-ai-cli 側の不具合ではない。statusbar は実態（Claude 申告の実窓）を正しく表示している。コード修正なし。
- 1M を使いたいセッションでは `/model` → 「Opus 4.8 (1M context)」を選び直す（既定化済みなら起動し直しでも可）。次の API 応答以降、statusbar が自動で `/1.0M` ＋「1M」バッジに切り替わる。
- 確認方法：当該セッションで `/context` を実行し、分母が 1,000,000 になっていれば 1M が有効。
