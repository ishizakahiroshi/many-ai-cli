# [完了] スラッシュコマンド鮮度チェックを release 前提に組み込む

> 最終更新: 2026-06-19(金) 08:06:57 — C1〜C5 を実装。skill・スクリプト・release preflight・docs を整備し動作検証済み

## 実行記録（2026-06-19）

- C1: 入出力契約を `skills/slash-commands-update/references/provider-sources.md` に集約（provider source-of-truth・md 形式制約・レポート形式・自動/人間確認の分離）。
- C2: `skills/slash-commands-update/SKILL.md` を作成（provider 動的検出・report/apply/preflight・unknown 扱い）。`SKILLS.md` ルーティング表と `how-to-use.md` に追加。
- C3: `scripts/freshness-report.ps1`（ローカルインベントリ抽出＋レポート雛形・commit/push なし）。実機 `copilot help commands` のみ自動 diff、docs 系と claude/opencode は誤検出回避のため unknown で AI 委譲。動作検証で `docs/local/slash-command-freshness_2026-06-19.md` を生成。
- C4: `skills/release/SKILL.md` 前提チェックに `repo == many-ai-cli` 限定 preflight を追加。`scripts/freshness-preflight.ps1`（exit 0=可 / 2=未判断差分ブロック / 3=stale）で検証。未判断のみブロック・unknown は既定警告。
- C5: `docs/manual_slash_commands_update.md`（スキル連携手順）と `docs/manual_release.md`（freshness preflight 組込み・drift ループに cursor-agent/opencode 追加）を更新。

> 着手判断は人間が差分だけ確認する半自動運用。`main` への自動 push はしない設計のまま。

## context配分

| C | 種別 | 内容 | 並列 |
|---|---|---|---|
| C1 | plan | 現行運用と更新対象を整理し、`slash-commands-update` skill の入出力契約を作る | — |
| C2 | plan | `C:\dev\workshop\skills\slash-commands-update` を作り、差分検出・正規化・レポート作成の手順を入れる | C1 後 |
| C3 | plan | 日次バッチ用の実行入口を作り、差分がある時だけ人間確認用レポートを残す | C2 後 |
| C4 | plan | `release` skill に many-ai-cli 専用 preflight を追加し、リリース前に鮮度確認済みか検査する | C2 後 |
| C5 | plan | many-ai-cli 側の手順書を更新し、release md に鮮度確認結果を残す運用に揃える | C3, C4 後 |

実行順序: `C1 → C2 → (C3, C4) → C5`

## 概要

many-ai-cli のスラッシュコマンドピッカーは、`resources/slash-commands/*.md` を GitHub `main` の raw URL から実行時取得する。これはリビルド不要で更新できる一方、各 AI CLI の本家 `/` コマンドが頻繁に変わるため、リリース直前に手作業で追う運用だと漏れや差異が出やすい。

本計画では、`C:\dev\workshop\skills` に専用 skill を追加し、調査・差分抽出・md 正規化・レポート作成を半自動化する。自動で `main` へ push するのではなく、日次バッチは差分レポートまでに留める。採用判断は人間が差分だけ確認し、release skill は「鮮度確認済みか」を preflight として扱う。

## 背景

現行手順は `docs/manual_slash_commands_update.md` にまとまっている。

- 正本ファイルは `resources/slash-commands/claude.md` / `codex.md` / `copilot.md` / `cursor-agent.md`
- Hub は `main` の raw URL から取得し、provider ごとに 24 時間キャッシュする
- `develop` で更新しただけではユーザーには反映されず、`main` へ入った時点で live 配信される
- `docs/manual_release.md` では、release 前の差分確認は drift 確認であり、本家コマンドに対する freshness は別オペとされている

一方、`C:\dev\workshop\skills\release\SKILL.md` の preflight には、`CHANGELOG.md` や `repo-consistency` はあるが、スラッシュコマンド鮮度チェックはまだ入っていない。

## 方針

- 完全自動更新ではなく、半自動にする。
- 日次バッチは「本家との差分検出」「many-ai-cli md 形式への正規化案」「人間確認用レポート作成」まで行う。
- 自動 commit / 自動 main push はしない。`main` raw URL は全ユーザーへ配信されるため、誤検出を自動公開しない。
- release skill は `repo == many-ai-cli` の時だけ、鮮度確認レポートが存在し、期限内で、未処理差分がないことを確認する。
- 差分がある場合は release を止めるのではなく、まず「採用する / 見送る / 調査中」を release md に明示させる。未判断の差分だけをブロック扱いにする。

## C1: 入出力契約の整理

### 対象ファイル

- `docs/manual_slash_commands_update.md`
- `docs/manual_release.md`
- `C:\dev\workshop\skills\release\SKILL.md`
- `C:\dev\workshop\skills\SKILLS.md`

### 作業内容

- provider ごとの取得元を明文化する。
  - Claude Code: 現行手順どおり、信頼できる実機出力または claude-code-guide 相当の調査結果を使う。
  - Codex CLI: 公式 Codex CLI slash commands docs を正本にする。
  - GitHub Copilot CLI: `copilot help commands` の実機出力を優先する。
  - Cursor Agent CLI: 実機 `/help` を優先し、無い場合は Cursor 公式 docs を使う。
- `resources/slash-commands/*.md` の形式制約を skill へ移す。
  - コマンド列は引数なしの `/cmd` のみ。
  - 説明文に `|` を入れない。
  - 説明文に丸括弧・角括弧で重要情報を書かない。
  - 並び順はコマンド名 ABC 順。
- freshness レポートの保存先と形式を決める。
  - 推奨: `docs/local/slash-command-freshness_YYYY-MM-DD.md`
  - レポートには provider / source / checked_at / detected diff / decision を持たせる。

### 完了条件

- skill が読むべき既存文書と、書き出すべき成果物が明確になっている。
- 自動で公開してよい処理と、人間確認が必要な処理が分離されている。

## C2: slash-commands-update skill を作る

### 対象ファイル

- `C:\dev\workshop\skills\slash-commands-update\SKILL.md`
- 必要なら `C:\dev\workshop\skills\slash-commands-update\scripts\*.ps1`
- 必要なら `C:\dev\workshop\skills\slash-commands-update\references\*.md`
- `C:\dev\workshop\skills\SKILLS.md`

### 作業内容

- skill frontmatter を作る。
  - `name: slash-commands-update`
  - description には「many-ai-cli の `resources/slash-commands/*.md` を本家 AI CLI の最新 `/` コマンドに追従させる。日次差分確認、release 前 freshness preflight、正規化、レポート作成で使う」旨を入れる。
- `SKILL.md` に通常フローを記載する。
  - repo root を特定する。
  - 現在の `resources/slash-commands/*.md` を読む。
  - provider ごとの本家 source を取得またはユーザー提供出力から読む。
  - 差分を command 単位で出す。
  - many-ai-cli の md テーブル形式に正規化案を作る。
  - `docs/local/slash-command-freshness_YYYY-MM-DD.md` にレポートを作る。
- 実行モードを分ける。
  - `report`: 差分レポートのみ。
  - `apply`: 人間が承認した差分だけ `resources/slash-commands/*.md` へ反映。
  - `preflight`: release skill から呼ばれ、期限内レポートと未判断差分を確認。
- provider source が取れない場合の扱いを決める。
  - ネットワーク失敗や CLI 未インストールは `unknown` として記録し、勝手に「差分なし」にしない。
  - release preflight では `unknown` が残る provider を警告またはブロックにする。ブロック条件は release md で明示的に override できるようにする。

### 完了条件

- Claude Code から明示起動できる skill として `C:\dev\workshop\skills` に追加されている。
- `SKILLS.md` のルーティング表に `slash-commands-update` が追加されている。
- Codex でも使う場合の配置方針がメモされている。現状 `C:\dev\workshop\skills` は Claude 用 junction の実体なので、Codex 用には `~\.codex\skills` 側の junction またはコピーが別途必要。

## C3: 日次バッチ入口を作る

### 対象ファイル

- `C:\dev\workshop\skills\slash-commands-update\scripts\*.ps1`
- 必要なら `C:\dev\workshop\scheduled-tasks\*.ps1`
- 必要なら `C:\dev\workshop\release-registry.json`

### 作業内容

- Windows 日次実行を想定し、PowerShell 入口を用意する。
- バッチは many-ai-cli repo を対象に `report` モードだけを実行する。
- 差分が無い場合は最新確認時刻だけを更新する。
- 差分がある場合はレポートに次を残す。
  - 追加候補
  - 削除候補
  - 説明文変更候補
  - source URL または実機コマンド出力の取得元
  - 推奨 decision: `pending`
- バッチは commit / push しない。
- 通知が必要なら、最初はファイル出力だけにする。通知や GitHub Issue/PR 作成は後続拡張に回す。

### 完了条件

- 1日1回実行しても repo を勝手に変更しない。
- 差分がある時だけ、人間が見るべきレポートが明確に残る。
- バッチ失敗時も release 前に `unknown` として検出できる。

## C4: release skill preflight に組み込む

### 対象ファイル

- `C:\dev\workshop\skills\release\SKILL.md`
- 必要なら `C:\dev\workshop\skills\release\references\*.md`

### 作業内容

- `release` skill の前提チェックに many-ai-cli 専用項目を追加する。
- 条件:
  - `repo == many-ai-cli` の場合、`slash-commands-update preflight` を実施する。
  - 最新 freshness レポートが存在しない、または古い場合は release 実行前に `report` を促す。
  - 未判断の差分がある場合は release md に採用判断を書くまで止める。
  - `accepted` の差分は `resources/slash-commands/*.md` に反映済みであることを確認する。
  - `deferred` の差分は release md の `notes` または `申し送り` に理由を書く。
- release md の「リリース引数」または「実行計画」に、freshness 結果を残す欄を追加する。

### 完了条件

- many-ai-cli release で、スラッシュコマンド鮮度チェックの抜けが preflight で検出される。
- other repo の release には影響しない。
- 未判断差分だけがブロックされ、明示的な見送りはブロックしない。

## C5: many-ai-cli 文書と運用を揃える

### 対象ファイル

- `docs/manual_slash_commands_update.md`
- `docs/manual_release.md`
- 必要なら `docs/local/manual_release-vX.Y.Z_YYYY-MM-DD.md` のテンプレ

### 作業内容

- `manual_slash_commands_update.md` に `slash-commands-update` skill を使う新手順を追記する。
- `manual_release.md` の runtime-served resources 節を更新し、freshness preflight が release skill に組み込まれていることを書く。
- release md に残す freshness 記録の例を追加する。

例:

```markdown
### Slash command freshness

| provider | checked_at | source | decision |
|---|---|---|---|
| claude | 2026-06-19 07:55 | claude-code-guide /実機出力 | accepted |
| codex | 2026-06-19 07:55 | OpenAI official docs | no diff |
| copilot | 2026-06-19 07:55 | `copilot help commands` | deferred: local CLI unavailable |
| cursor-agent | 2026-06-19 07:55 | Cursor official docs | no diff |
```

### 完了条件

- リリース直前に何を確認すればよいか、release skill と repo docs の記述が一致している。
- 「差分確認」と「本家 freshness 確認」が混同されない。

## 判断ログ

- `resources/slash-commands/*.md` は `main` raw URL で live 配信されるため、日次バッチによる自動 main push は避ける。
- provider ごとに正本の強さが違うため、完全自動採用ではなく人間が差分だけ承認する。
- `C:\dev\workshop\skills` は現状 Claude Code 用 junction の実体として運用されている。Codex でも同じ skill を使うなら、Codex skill 探索パスへの展開を別途決める。
- release skill へ直接巨大な provider 別調査手順を入れると肥大化するため、専用 `slash-commands-update` skill に分離する。

## 非対象

- Hub の `/api/slash-commands` 実装変更は本計画の主対象ではない。
- 自動で GitHub PR を作る仕組みは初期対象外。必要なら C3 の後続拡張として扱う。
- 各 provider CLI のインストールやログイン自動化はしない。
- GitHub `main` への自動 push はしない。

