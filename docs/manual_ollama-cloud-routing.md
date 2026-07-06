# Ollama Cloud 経由で Codex / Claude Code を使う運用手順

> 最終更新: 2026-06-19(金) 07:51:46

## 概要

Codex CLI と Claude Code を、ローカル Ollama を経由して Ollama Cloud モデルへ接続するための運用手順をまとめる。Anthropic / OpenAI の API key を保有していない環境でも、`ollama signin` 済みであれば cloud モデルが利用できる。

`plan_spawn-model-picker-ollama.md` の実装により、many-ai-cli v0.3.0+ では **Hub 画面の spawn フォームから Ollama Cloud / Local モデルを直接選択して起動できる**。env や profile を親 shell で設定する必要はない（spawn 時に Hub が必要な env を自動注入する）。

旧来の「shell で env を手動設定する」手順は本書末尾の [付録: 親 shell で env を設定する方式](#付録-親-shell-で-env-を設定する方式) に降格した。Hub 画面方式が使えない場合（CLI 直接起動など）の fallback として残置。

## 前提

- Ollama 0.15 以降（`ollama launch` サブコマンドが必要）
- `ollama signin` 済みであること（cloud モデル利用権が紐付くアカウント）
- `claude` / `codex` バイナリが PATH 上にあること
- many-ai-cli は v0.3.0 以降

## 接続経路と API key の責務

| 経路 | 必要なもの | Anthropic / OpenAI 本物 key |
|---|---|---|
| ローカル daemon 経由（推奨） | `ollama signin` 済みのローカル Ollama | 不要 |
| ollama.com 直結 | `OLLAMA_API_KEY`（ollama.com 発行） | 不要 |
| 純正 API 直結 | Anthropic / OpenAI の本物 key | 必要 |

ローカル daemon 経由では、Codex の `OPENAI_API_KEY` / Claude Code の `ANTHROPIC_AUTH_TOKEN` ともに **ダミー文字列 `ollama` で構わない**（API は required だが値は無視される）。

## Codex × Ollama Cloud

### Codex の base URL と接続形式

- base URL: `http://localhost:11434/v1`
- 認証: 不要（profile 経由が推奨。CLI 引数で直接指定もできる）

### `~/.codex/config.toml` の profile 例

```toml
[model_providers.ollama-launch]
name = "Ollama"
base_url = "http://localhost:11434/v1"
wire_api = "responses"

[profiles.ollama-cloud]
model = "gpt-oss:120b-cloud"
model_provider = "ollama-launch"
```

### 起動方法

profile 経由:

```powershell
codex --profile ollama-cloud
```

CLI 引数で直接指定:

```powershell
codex --oss -m gpt-oss:120b-cloud
```

`ollama launch` サブコマンド経由（Ollama 0.15+）:

```powershell
ollama launch codex
```

### Codex で使える cloud モデル名（命名形式 `<name>:<size>-cloud`）

- `gpt-oss:120b-cloud`
- `gpt-oss:20b-cloud`
- `deepseek-v3.1:671-cloud`
- `qwen3-coder:480b-cloud`

最新一覧は `https://ollama.com/search?c=cloud` を参照。

## Claude Code × Ollama Cloud

### Claude Code の base URL と接続形式

- base URL: `http://localhost:11434`（**`/v1` を付けない**。SDK 側で `/v1/messages` が付与される）
- 認証: `ANTHROPIC_AUTH_TOKEN` にダミー文字列、`ANTHROPIC_API_KEY` を空文字に明示設定する必要あり

### 環境変数（PowerShell の例）

```powershell
$env:ANTHROPIC_AUTH_TOKEN = "ollama"
$env:ANTHROPIC_API_KEY = ""
$env:ANTHROPIC_BASE_URL = "http://localhost:11434"
claude --model kimi-k2.5:cloud
```

`ANTHROPIC_API_KEY` を空文字に明示しないと、Claude Code が純正 Anthropic 接続へフォールバックする場合がある（要注意）。

### `ollama launch` サブコマンド経由（Ollama 0.15+）

```powershell
ollama launch claude
ollama launch claude --model kimi-k2.5:cloud
```

`ollama launch claude` は環境変数の設定を自動で行うため、手動 env より安全。

### モデル別名（必要な場合）

Claude Code 側コードが既定モデル名 `claude-3-5-sonnet` を要求する場合、Ollama 側でエイリアスを切れる:

```powershell
ollama cp qwen3-coder claude-3-5-sonnet
```

### Claude Code で使える cloud モデル名（命名形式 `<name>:cloud`）

公式 example で確認済み:

- `kimi-k2.5:cloud`
- `glm-5:cloud`
- `qwen3.5:cloud`
- `qwen3-coder:cloud`
- `deepseek-v3.2:cloud`
- `minimax-m2.5:cloud`

最新一覧は `https://ollama.com/search?c=cloud` を参照。

## many-ai-cli 経由での起動（推奨: Hub 画面方式）

### 仕組み

- Hub UI の spawn フォームで「Model」コンボボックスを開くと `[Anthropic]` / `[OpenAI]` / `[Ollama Cloud]` / `[Ollama Local]` の各グループからモデルを選択できる
- バックエンド `/api/models` が以下を集約して返す:
  - Anthropic / OpenAI: ハードコード一覧
  - Ollama Cloud: local daemon `/api/tags` の `remote_host` 付き alias
  - Ollama Local: `ollama.base_url` の `/api/tags`（既定 `http://localhost:11434/api/tags`、60s キャッシュ） + `~/.many-ai-cli/config.yaml` の `local_models:` 手書きを merge
- 選択したモデルから route を自動判定し、spawn payload の `route` フィールドに付与
- Hub が route に応じた env preset を子プロセスに注入:
  - `route=ollama` × claude → `ANTHROPIC_AUTH_TOKEN=ollama` / `ANTHROPIC_API_KEY=` (空文字) / `ANTHROPIC_BASE_URL=<ollama.base_url>`
  - `route=ollama` × codex → `OPENAI_API_KEY=ollama` / `OPENAI_BASE_URL=<ollama.base_url>/v1`
  - `route=anthropic` / `route=openai` → 既存 shell env をそのまま継承（env 注入なし）

### Codex を Ollama Cloud で起動する手順（Hub 画面）

1. `many-ai-cli serve` で Hub を起動（ブラウザ自動起動）
2. spawn フォームを開き、provider を `codex` に設定
3. Model 欄をクリック → `[Ollama Cloud]` グループから `gpt-oss:120b-cloud` 等を選ぶ
4. Launch
5. 子プロセスに `OPENAI_BASE_URL=http://localhost:11434/v1` 等の env が自動注入され、Ollama 経由で起動する

### Claude Code を Ollama Cloud で起動する手順（Hub 画面）

1. provider を `claude` に設定
2. Model 欄から `[Ollama Cloud]` グループの `kimi-k2.5:cloud` 等を選ぶ
3. Launch
4. `ANTHROPIC_BASE_URL=http://localhost:11434` 等が自動注入される

### Ollama daemon が別ホストにある場合

Hub を Hyper-V ゲスト内で動かし、Ollama と GPU をホスト Windows 側で動かす場合などは、`~/.many-ai-cli/config.yaml` に接続先を設定する:

```yaml
ollama:
  base_url: "http://192.168.11.50:11434"
```

Default Switch 側を使う場合の例:

```yaml
ollama:
  base_url: "http://172.20.224.1:11434"
```

この値は `/api/models` の `/api/tags` 取得と、spawn 時に注入する `ANTHROPIC_BASE_URL` / `OPENAI_BASE_URL` の両方に使われる。`base_url` には `/v1` や `/api/tags` を付けない。

### Local LLM（pull 済み）を使う

- Ollama daemon を起動しておけば、`[Ollama Local]` グループに `/api/tags` で取れたモデルが並ぶ
- daemon が落ちていても `~/.many-ai-cli/config.yaml` に手書きで以下を追記すれば一覧に出る:

```yaml
local_models:
  - id: "llama3.2:3b"
    label: "Llama 3.2 (軽量)"
  - id: "qwen3:14b"
```

### リフレッシュ

Model 欄の隣の `↻` ボタンで `/api/models` を強制再取得できる。`ollama pull <model>` した直後にこのボタンを押せば一覧に反映される。

### Hub プロセス再起動の必要性

env 注入は **spawn 時に子プロセス単位で適用** されるため、Hub プロセス自体の再起動は不要。route を切り替えて新しいセッションを spawn し直すだけでよい。

## 表示メタデータの方針（将来仕様）

将来、provider 汎化が完了した段階で Hub UI / ログに以下のフィールドを表示することを想定する（v0.3.0 時点では未実装、設計メモとして記録）。

| フィールド | 例 |
|---|---|
| `provider` | `codex`, `claude` |
| `route` | `ollama`, `anthropic`, `openai` |
| `model` | `gpt-oss:120b-cloud`, `kimi-k2.5:cloud` |
| `displayTitle` | `Codex (Ollama)`, `Claude Code (Ollama)` |
| `displaySubtitle` | `gpt-oss:120b-cloud` |

基本表示（広い領域）:

```text
Codex (Ollama)
gpt-oss:120b-cloud
```

```text
Claude Code (Ollama)
kimi-k2.5:cloud
```

狭い領域での短縮表示:

```text
Codex · Ollama · gpt-oss
Claude · Ollama · kimi-k2.5
```

ログ・セッション詳細の完全表示:

```text
Provider: Codex
Route: Ollama
Model: gpt-oss:120b-cloud
Command: codex --profile ollama-cloud
```

model 名取得の優先順位:

1. spawn UI または config で明示された `model`
2. CLI 引数の `--model` / `-m`
3. 既知の profile 設定から安全に解決できる model
4. 不明な場合は空にして route まで表示（例: `Codex (Ollama)` のみ）

## 通常接続へ戻す手順

### Codex を本来の OpenAI 接続に戻す

profile を切り替えるだけでよい:

```powershell
codex --profile default        # ~/.codex/config.toml に通常 profile があれば
# または引数なしで起動（既定 profile を使う）
codex
```

`OPENAI_API_KEY` を別途設定している場合、それは OpenAI 直結の認証として有効。

### Claude Code を本来の Anthropic 接続に戻す

env を解除する:

```powershell
Remove-Item Env:ANTHROPIC_AUTH_TOKEN
Remove-Item Env:ANTHROPIC_BASE_URL
$env:ANTHROPIC_API_KEY = "<本物の Anthropic key>"   # 純正接続を使う場合
claude
```

env が残ったままだと Claude Code がローカル Ollama に向かい続けるため、Hub の再起動と shell の env 整理を必ずセットで行う。

## 検証手順

実機検証はユーザー実行前提。以下 6 観点を順に確認する。

### 1. Ollama Cloud daemon オフライン時

- 状況: ローカル daemon は止まっているが、`ollama.com/api/tags` には到達可
- 期待: Hub UI の Model 欄に `[Anthropic]` / `[OpenAI]` / `[Ollama Cloud]` のみ表示される
- 確認: `/api/models?token=...` の response の `warnings` に `ollama_daemon_unreachable` が出る（config に `local_models` が無い場合）

### 2. Ollama daemon 未起動時 + Cloud 到達不可

- 状況: daemon も Cloud API も両方使えない
- 期待: `[Anthropic]` / `[OpenAI]` のみ表示。`[Ollama Cloud]` / `[Ollama Local]` 両方が省かれる
- `warnings` に `ollama_cloud_fetch_failed` と `ollama_daemon_unreachable` 両方が出る

### 3. 両方起動時

- 状況: ローカル daemon + Cloud API 両方 OK
- 期待: 全グループが表示される。選択 → Launch で env preset が注入されて起動する
- 子 PTY 内で `echo $env:ANTHROPIC_BASE_URL` 等を打つと注入された値が見える（PowerShell の場合）

### 4. 手入力時の自動 route 判定

- datalist に無い `something:cloud` のような名前を打って Launch
- 期待: `:cloud` を含むため自動で `route=ollama` 推定 → Ollama env が注入される
- `claude-opus-4-7` 等の Anthropic モデル名 → `route=anthropic`（env 注入なし）

### 5. 既存の通常起動

- 何も選ばず空欄、または `claude-sonnet-4-5` を入れて Launch
- 期待: env 注入なし。既存の Anthropic / OpenAI 接続経路で起動（既存挙動の互換性）

### 6. refresh ボタン

- `ollama pull <model>` した後に Model 欄の `↻` ボタンをクリック
- 期待: `[Ollama Local]` グループに pull したモデルが新しく出る

## トラブル時の切り分け

| 症状 | 原因候補 | 確認 |
|---|---|---|
| Claude Code が Anthropic 純正へ繋ぎに行く | `ANTHROPIC_API_KEY` 未空化 / `ANTHROPIC_BASE_URL` 未設定 | `Get-ChildItem Env:ANTHROPIC_*` |
| Codex が `gpt-4o-mini` 等の OpenAI モデルを返す | profile 未適用 / `--profile` 指定漏れ | `codex --profile ollama-cloud --help` |
| `ollama: command not found` | Ollama 0.15+ 未インストール | `ollama --version` |
| cloud モデルが pull できない | `ollama signin` 未実施 / cloud 利用権なし | `ollama whoami` |
| Hub 経由だと env が効かない | 既存 Hub プロセスの env が古い | `many-ai-cli stop && many-ai-cli serve` |
| `ollama launch` が無い | Ollama < 0.15 | バージョン更新 |

## 関連

- 親計画: [plan_ollama-cloud-codex-claude.md](plan_ollama-cloud-codex-claude.md)
- 実装計画: [plan_spawn-model-picker-ollama.md](plan_spawn-model-picker-ollama.md)
- Provider 汎化計画: `docs/local/archive/v0.1.3/plan_provider-extensibility_20260510.md`
- Ollama 公式 cloud モデル一覧: `https://ollama.com/search?c=cloud`
- Ollama 公式統合ドキュメント: `https://docs.ollama.com/integrations/codex` / `https://docs.ollama.com/integrations/claude-code`

---

## 付録: 親 shell で env を設定する方式

many-ai-cli v0.3.0+ では Hub 画面方式が主だが、以下の場合は親 shell で env を設定する旧方式が使える。

- `many-ai-cli` を経由せず直接 `claude` / `codex` を起動するケース
- Hub の env preset 注入が未対応のセットアップに遭遇した場合の fallback
- `ollama launch <provider>` サブコマンド経由（Ollama 0.15+）

### Codex を Ollama Cloud で起動（shell 方式）

```powershell
codex --profile ollama-cloud
# または
codex --oss -m gpt-oss:120b-cloud
# または
ollama launch codex
```

### Claude Code を Ollama Cloud で起動（shell 方式）

```powershell
$env:ANTHROPIC_AUTH_TOKEN = "ollama"
$env:ANTHROPIC_API_KEY = ""
$env:ANTHROPIC_BASE_URL = "http://localhost:11434"
claude --model kimi-k2.5:cloud
# または
ollama launch claude --model kimi-k2.5:cloud
```

`ollama launch claude` は環境変数の設定を自動で行うため、手動 env より安全。

### many-ai-cli serve を shell env 込みで起動

shell 方式で many-ai-cli 全体を Ollama 化するには、env を export 済みの shell から Hub を起動する:

```powershell
$env:ANTHROPIC_AUTH_TOKEN = "ollama"
$env:ANTHROPIC_API_KEY = ""
$env:ANTHROPIC_BASE_URL = "http://localhost:11434"
many-ai-cli serve
```

この方式は Hub 全体を Ollama 一択に固定するため、別の route のセッションを混在させたい場合は Hub 画面方式（spawn ごとに route を切替）を推奨。
