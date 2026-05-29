package wrapper

import (
	"bufio"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"strings"
)

const claudeImportLine = "@~/.any-ai-cli/approval-rules.md"
const codexBlockStart  = "<!-- any-ai-cli:approval-rules -->"
const codexBlockEnd    = "<!-- /any-ai-cli:approval-rules -->"
const rulesVersion     = "4"

var rulesFileContent = strings.Join([]string{
	fmt.Sprintf("<!-- version: %s -->", rulesVersion),
	"## any-ai-cli Approval Format",
	"",
	"**このセッションが any-ai-cli Hub 管理下かどうかを最初に確認してください。**",
	"",
	"確認方法（**OS と使用するツールに応じて構文を選ぶ**）：",
	"",
	"- macOS / Linux: `Bash` ツールで `echo \"$ANY_AI_CLI\"`",
	"- Windows (PowerShell ネイティブ): `PowerShell` ツールで `$env:ANY_AI_CLI`",
	"- Windows (Git Bash / WSL / Cygwin): `Bash` ツールで `echo \"$ANY_AI_CLI\"`",
	"",
	"⚠️ **取り違え注意**：",
	"- `Bash` ツールに `$env:ANY_AI_CLI` を渡すと `:ANY_AI_CLI: command not found`（exit 127）で失敗する。bash では `$env` が空に展開され、残った `:ANY_AI_CLI` がコマンドとして実行されるため。",
	"- `PowerShell` ツールに `echo $ANY_AI_CLI` を渡すと、`$ANY_AI_CLI` は PowerShell では未定義の変数として空文字に展開され、値が取得できない。",
	"- macOS / Linux には PowerShell が標準で入っていないので `PowerShell` ツールは選ばない。",
	"- 失敗したらツールを切り替えて再試行すること（落としてセッションを止めない）。",
	"",
	"`ANY_AI_CLI=1` の場合のみ、以下の [ANY-AI-CLI] マーカーを使用してください。",
	"未設定・空の場合はマーカーなしでプレーンテキストで出力してください。",
	"（このチェックはセッション内で1回だけ行い、結果を記憶してください）",
	"",
	"- YES/NO:",
	"  [ANY-AI-CLI]",
	"  question? (Y:1/N:0)",
	"  [/ANY-AI-CLI]",
	"",
	"- 番号付き選択肢:",
	"  [ANY-AI-CLI]",
	"  question?",
	"  1. Option A (Recommended)",
	"  2. Option B",
	"  N. User specifies",
	"  [/ANY-AI-CLI]",
	"",
	"  - Option 1 は必ず推奨選択肢とし (Recommended) を末尾に付ける",
	"  - 最後の選択肢は必ず「N. User specifies」",
	"",
	"- 複数質問（一括確認、上限 N=8 推奨）:",
	"  [ANY-AI-CLI]",
	"  1 question1?",
	"   1. Option A (Recommended)",
	"   2. Option B",
	"   N. User specifies",
	"  2 question2?",
	"   1. Option A (Recommended)",
	"   2. Option B",
	"   N. User specifies",
	"  [/ANY-AI-CLI]",
	"",
	"  - 1 ブロックに 2 件以上の質問を並べる場合のみこの形式を使う",
	"  - 質問の見出し番号は 1, 2, 3 ... の連番。プレフィックス（Q1: / C1: 等）は付けない",
	"  - 各質問の選択肢行は 1 文字以上インデントする（見出し番号と区別するため）",
	"  - ユーザーの回答は空白区切りの数字列で返ってくる（例: 1 2 1 3）。各数値は質問順に対応する",
	"",
	"- [ANY-AI-CLI] マーカーは確認・承認の質問にのみ使用する",
	"",
}, "\n")

// centralRulesPath は ~/.any-ai-cli/approval-rules.md のパスを返す
func centralRulesPath() string {
	home, _ := os.UserHomeDir()
	return filepath.Join(home, ".any-ai-cli", "approval-rules.md")
}

// CentralRulesDir は ~/.any-ai-cli/ ディレクトリのパスを返す
func CentralRulesDir() string {
	home, _ := os.UserHomeDir()
	return filepath.Join(home, ".any-ai-cli")
}

// SyncRulesFile はバージョンを確認し、不一致または不存在なら最新内容で上書きする
func SyncRulesFile() error {
	path := centralRulesPath()
	if data, err := os.ReadFile(path); err == nil {
		firstLine := ""
		if idx := strings.IndexByte(string(data), '\n'); idx >= 0 {
			firstLine = strings.TrimSpace(string(data[:idx]))
		} else {
			firstLine = strings.TrimSpace(string(data))
		}
		if firstLine == fmt.Sprintf("<!-- version: %s -->", rulesVersion) {
			return nil
		}
	}
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return fmt.Errorf("mkdir %s: %w", filepath.Dir(path), err)
	}
	return os.WriteFile(path, []byte(rulesFileContent), 0o644)
}

// ScanClaudeConfigured は CLAUDE.md に claudeImportLine が含まれているか確認する
func ScanClaudeConfigured(path string) (bool, error) {
	f, err := os.Open(path)
	if err != nil {
		return false, err
	}
	defer f.Close()
	scanner := bufio.NewScanner(f)
	for scanner.Scan() {
		if strings.TrimSpace(scanner.Text()) == claudeImportLine {
			return true, nil
		}
	}
	return false, nil
}

// ScanCodexConfigured は AGENTS.md に codexBlockStart が含まれているか確認する
func ScanCodexConfigured(path string) (bool, error) {
	f, err := os.Open(path)
	if err != nil {
		return false, err
	}
	defer f.Close()
	scanner := bufio.NewScanner(f)
	for scanner.Scan() {
		if strings.TrimSpace(scanner.Text()) == codexBlockStart {
			return true, nil
		}
	}
	return false, nil
}

// appendClaudeImport は CLAUDE.md の末尾に claudeImportLine を追記する
func appendClaudeImport(path string) error {
	f, err := os.OpenFile(path, os.O_APPEND|os.O_WRONLY, 0o644)
	if err != nil {
		return fmt.Errorf("open %s: %w", path, err)
	}
	defer f.Close()
	_, err = fmt.Fprintf(f, "\n%s\n", claudeImportLine)
	return err
}

// appendCodexBlock は AGENTS.md の末尾に中央ファイルの内容をブロックとして追記する
func appendCodexBlock(path string) error {
	centralContent, err := os.ReadFile(centralRulesPath())
	if err != nil {
		return fmt.Errorf("read central rules: %w", err)
	}
	block := strings.Join([]string{
		codexBlockStart,
		strings.TrimSpace(string(centralContent)),
		codexBlockEnd,
		"",
	}, "\n")
	f, err := os.OpenFile(path, os.O_APPEND|os.O_WRONLY, 0o644)
	if err != nil {
		return fmt.Errorf("open %s: %w", path, err)
	}
	defer f.Close()
	_, err = fmt.Fprintf(f, "\n%s", block)
	return err
}

// InjectRules はファイルに承認ルールを注入する
func InjectRules(provider, path string) error {
	if err := SyncRulesFile(); err != nil {
		return fmt.Errorf("sync rules file: %w", err)
	}
	switch provider {
	case "claude":
		return appendClaudeImport(path)
	case "codex":
		return appendCodexBlock(path)
	default:
		return fmt.Errorf("unknown provider: %s", provider)
	}
}

// RemoveRules はファイル内のルール設定を削除する
func RemoveRules(provider, path string) error {
	content, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return nil
		}
		return fmt.Errorf("read %s: %w", path, err)
	}
	var newContent string
	switch provider {
	case "claude":
		var kept []string
		for _, line := range strings.Split(string(content), "\n") {
			if strings.TrimSpace(line) != claudeImportLine {
				kept = append(kept, line)
			}
		}
		newContent = strings.Join(kept, "\n")
	case "codex":
		blockRe := regexp.MustCompile(`(?s)\n?` + regexp.QuoteMeta(codexBlockStart) + `.*?` + regexp.QuoteMeta(codexBlockEnd) + `\n?`)
		newContent = blockRe.ReplaceAllString(string(content), "")
	default:
		return fmt.Errorf("unknown provider: %s", provider)
	}
	return os.WriteFile(path, []byte(newContent), 0o644)
}
