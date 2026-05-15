# Windows Git Bash 検証手順

> 最終更新: 2026-05-13(水) 20:15:00

## 前提

- Git for Windows がインストール済み（`C:\Program Files\Git\`）
- `any-ai-cli.exe` がビルド済み（`C:\dev\any-ai-cli\any-ai-cli.exe`）

## 手順

### 1. Git Bash を起動する

PowerShell から起動する場合（スペースを含むパスに `&` が必要）：

```powershell
& "C:\Program Files\Git\bin\bash.exe"
```

> `bash` コマンドをそのまま実行すると WSL にルーティングされるため、フルパス指定が必要。

### 2. PATH を通す

```bash
export PATH="$PATH:/c/dev/any-ai-cli"
which any-ai-cli  # → /c/dev/any-ai-cli/any-ai-cli と表示されればOK
```

### 3. shell-init の出力確認

```bash
any-ai-cli shell-init
```

POSIX bash 構文のスクリプトが出力されることを確認：

```bash
if [ "${ANY_AI_CLI_AUTO:-0}" = "1" ]; then
  claude(){ any-ai-cli claude "$@"; }
  ...
fi
```

### 4. Hub 経由で claude を起動する

```bash
export ANY_AI_CLI_AUTO=1
eval "$(any-ai-cli shell-init)"
claude
```

## 確認ポイント

| 確認項目 | 期待値 | 結果（2026-05-09） |
|---|---|---|
| Hub UI のシェル表示 | `Terminal Output › bash` | ✓ |
| PTY 出力のリアルタイム表示 | Hub UI に流れる | ✓ |
| 承認フロー（Yes/No）表示 | action-bar が出る | ✓ |

## 既知の注意点

- `bash` コマンドは WSL に奪われるため、Git Bash の起動は必ずフルパスで行う
- `C:\dev\any-ai-cli` は Git Bash の PATH に含まれないため、毎回 `export PATH` が必要（永続化する場合は `~/.bashrc` に追記）
- `ANY_AI_CLI_AUTO=1` を設定しないと shell-init の関数定義がスキップされ、`claude` がラッパーを通らずに直接起動する

## AI エージェントが Git Bash 上で承認チェックする時の落とし穴

Git Bash 配下で AI エージェント（Claude Code 等）が `~/.any-ai-cli/approval-rules.md` に従って `ANY_AI_CLI` を確認するとき、**ツール選択と構文を揃えないと exit 127 で必ず落ちる**。実例：

```
Bash($env:ANY_AI_CLI)
  Error: Exit code 127
  /usr/bin/bash: line 1: :ANY_AI_CLI: command not found
```

原因：bash では `$env` が空展開され、残った `:ANY_AI_CLI` をコマンドとして実行しようとして失敗する。

| 実行ツール | 正しい構文 | 失敗例 |
|---|---|---|
| `Bash`（Git Bash / WSL / Cygwin） | `echo "$ANY_AI_CLI"` | `$env:ANY_AI_CLI` → exit 127 |
| `PowerShell`（ネイティブ） | `$env:ANY_AI_CLI` | `echo $ANY_AI_CLI` → 空文字（未定義扱い） |

> Git Bash セッション内で AI が動いている場合は Bash ツール側の構文で確認すること。失敗したら approval-rules.md の指示通りツールを切り替えて再試行する（セッションを止めない）。
