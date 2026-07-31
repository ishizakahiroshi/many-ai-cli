# [廃止] ~/.claude 常時ロードの棚卸し（機密 @import ＋ MCP 定義 ＋ メモリ肥大の削減）

> 最終更新: 2026-06-22(月) 12:03:43

## 廃止理由（2026-06-22 追記）

スコープが [`plan_claude-md-slimming.md`](plan_claude-md-slimming.md) に実質継承され、本 plan を実行する余地が無くなったため [廃止]。具体的には:

- **C1（常時ロード量の棚卸し）**: `plan_claude-md-slimming.md` の C1 で実施完了。棚卸し対象は Memory files / System tools / Skills の全カテゴリに広がり、本 plan の対象（@import / MCP）はその一部として包含された
- **C2（機密 @import の常時ロード廃止）**: 既に達成済み。2026-06-22 の `/context` 実測で `serverpass.toml` / `serverpass.ishiz.toml` / family CSV はすでに Memory files に出ていない（@import から外されている）
- **C3（MCP ツール定義の常時ロード削減）**: 同上。MCP ツール（Google Drive / meijie-mcp 等）も常駐から消えている

つまり本 plan が想定していた介入は **別経路で既に達成されていた**ことが、今日の `/context` 実測で判明した。新規の介入余地は無いため [廃止]。

新しい context スリム化作業は [`plan_claude-md-slimming.md`](plan_claude-md-slimming.md) に集約し、こちらは archive 候補とする。

## context配分

| C | 種別 | 内容 | 並列 |
|---|---|---|---|
| C1 | plan | 常時ロード量の棚卸し（何が毎セッション載っているか・サイズ・機密性を一覧化） | — |
| C2 | plan | 機密 @import（serverpass.toml 等）の常時ロード廃止・オンデマンド化 | — |
| C3 | plan | MCP ツール定義の常時ロード削減（不要サーバの無効化／遅延ロード） | [並列OK with C2] |

実行順序: `C1 → (C2, C3)`

---

## 概要

毎セッションのプロンプトに、グローバル `~/.claude/CLAUDE.md` の `@import` 経由で機密ファイル（`serverpass.toml` / `serverpass.ishiz.toml` / family CSV 等）と MCP ツール定義が常時ロードされている。これには2つの問題がある:

1. **機密漏れリスク**: SSH 鍵パス・sudo / DB / メール / GitHub PAT などの実値が、使う使わないに関わらず毎回 AI プロバイダへ送られる（キャッシュ・インデックスされうる）。
2. **context 浪費**: `/context` 実測でメモリファイル計 32.6k トークン（うち `serverpass.toml` 4.1k）、MCP ツール 4.1k が、何もしなくても毎セッション消費される。

この plan は「常時ロードを必要時オンデマンドに寄せ、機密と無駄を毎セッションから外す」ための棚卸し＋方針出し。**実装は別 context で着手**する。本書は軽い計画（棚卸しと方針のみ）で、確定した実装手順は着手時に各 C で詰める。

スコープ外: many-ai-cli 本体のコード。これはユーザーのグローバル Claude 環境（`~/.claude/`）の構成見直し。

## 現状と問題

`/context`（2026-06-20 実測）での常時ロード内訳:

- Memory files 計 32.6k tokens
  - `~/.claude/CLAUDE.md` 11.6k
  - `~/.claude/.ssh/serverpass.toml` 4.1k ← 機密（SSH/sudo/DB/mail/onlyoffice 等）
  - `~/.claude/.ssh/serverpass.ishiz.toml` 599 ← 機密（VPS パス・GitHub PAT）
  - `~/.claude/family/facts.csv` 2.4k ＋ `people.csv` 553 ← 個人情報（tier 混在）
  - `local-toolset.md` 2k / `local-accounts.md` 603 / `approval-rules.md` 3.3k / 各 MEMORY.md ほか
- MCP tools 4.1k tokens
  - `mcp__claude_ai_Google_Drive__*`（10 ツール ≈ 4.4k 合計）
  - `mcp__claude_ai_meijie-mcp__*`（authenticate / complete_authentication）

問題の本質: これらは「いざ必要なときに引ければよい」情報なのに、`@import` と MCP 常時接続で**毎ターン全文がプロンプトに載る**。特に serverpass は使用頻度が低い割に最も機微。

## 方針

大枠の方向性（詳細は各 C 着手時に確定）:

- **機密は @import から外し、必要時に取得する経路へ寄せる**。候補:
  - 必要なときだけ Read するルールに変える（@import を消し、CLAUDE.md には「接続情報は `D:\dev\.ssh\serverpass.toml` を Read」と場所だけ書く）
  - もしくは専用 MCP / ローカルツール越しに「必要なキーだけ」引く
- **MCP は使用頻度で判定**。常用しない Google Drive / meijie-mcp は既定で無効化し、使うセッションだけ有効化（遅延ロード）する設定にできないか調べる。
- **family CSV は tier=high をコミット禁止の本則は維持**しつつ、常時 @import の必要性を再評価（オンデマンド or family MCP 経由に寄せる）。
- 採用しない案: 機密をそのまま残してサイズだけ削る（機微リスクが消えないため不可）。

---

## C1: 常時ロード量の棚卸し

### 作業内容

- `~/.claude/CLAUDE.md` の全 `@import` を列挙し、各ファイルの (a) サイズ (b) 機密性 (c) 実使用頻度 を表にする
- MCP 接続（claude.ai 設定側）の一覧と各ツール定義サイズ、実使用頻度を確認
- 「常時必要 / 時々必要 / ほぼ不要」に三分類し、C2・C3 の対象を確定

### 完了条件

- 常時ロード対象の分類表ができ、外す候補が確定している

---

## C2: 機密 @import の常時ロード廃止

### 作業内容

- `serverpass.toml` / `serverpass.ishiz.toml` の `@import` を CLAUDE.md から除去し、「必要時に Read する」運用へ置換（場所のポインタだけ残す）
- 既存の振る舞いルール（「リポは git remote から判定」等）で @import 前提のものが壊れないか確認・追従
- ジャンクション（`~/.claude/.ssh` → `D:\dev\.ssh`）はそのまま。@import 行のみ撤去

### 完了条件

- 新規セッションの `/context` から serverpass 系が消え、必要時は Read で引けることを確認

---

## C3: MCP ツール定義の常時ロード削減

### 作業内容

- Google Drive / meijie-mcp を「常時接続」から「使うときだけ有効化」へ変更できるか（claude.ai / Claude Code の MCP 設定の遅延ロード可否）を確認
- 不要なら既定無効化。必要セッションでの有効化手順を CLAUDE.md にメモ

### 完了条件

- 新規セッションの `/context` から不要 MCP ツール定義が消えている（or 削減できない場合は理由を本書へ記録）
