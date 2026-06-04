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
const sharedBlockStart = "<!-- any-ai-cli:approval-rules -->"
const sharedBlockEnd = "<!-- /any-ai-cli:approval-rules -->"
const rulesVersion = "7"

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
	"   3. Option C",
	"   N. User specifies",
	"  2 question2?",
	"   4. Option D (Recommended)",
	"   5. Option E",
	"   6. Option F",
	"   N. User specifies",
	"  [/ANY-AI-CLI]",
	"",
	"  - 選択肢番号は自由。上例のようなブロック全体の通し番号でも、質問ごとに 1. から振り直してもよい。ただし同一質問内で番号を重複させない",
	"  - ユーザーの回答には **画面に表示した選択肢番号がそのまま** 返ってくる。解釈時は自分が出力した番号と照合すること（1 起点に読み替えない）",
	"  - 1 ブロックに 2 件以上の質問を並べる場合のみこの形式を使う",
	"  - 質問の見出し番号は 1, 2, 3 ... の連番。プレフィックス（Q1: / C1: 等）は付けない",
	"  - 各質問の最初の選択肢を推奨とし (Recommended) を末尾に付ける。各質問の最後は必ず「N. User specifies」",
	"  - 各質問の選択肢行は 1 文字以上インデントする（見出し番号と区別するため）",
	"  - ユーザーの回答は各行「<質問見出し番号> <選択肢番号>」の複数行テキストで返ってくる（例: 上の通し番号の例なら「1 2」と「2 5」の 2 行 = 質問1 は Option B、質問2 は Option E）",
	"  - ユーザーが手入力した場合は「2 5」のような質問順の数字列 1 行のこともある。行頭の数字が質問番号として解釈できない場合はこちらとみなす",
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
	dir := filepath.Dir(path)
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return fmt.Errorf("mkdir %s: %w", dir, err)
	}
	if err := os.Chmod(dir, 0o700); err != nil {
		return fmt.Errorf("chmod %s: %w", dir, err)
	}
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
	return os.WriteFile(path, []byte(rulesFileContent), 0o644)
}

func providerUsesSharedBlock(provider string) bool {
	switch provider {
	case "codex", "copilot", "cursor-agent":
		return true
	default:
		return false
	}
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

// ScanSharedBlockConfigured は AGENTS.md 等に共有ブロックが含まれているか確認する
func ScanSharedBlockConfigured(path string) (bool, error) {
	f, err := os.Open(path)
	if err != nil {
		return false, err
	}
	defer f.Close()
	scanner := bufio.NewScanner(f)
	for scanner.Scan() {
		if strings.TrimSpace(scanner.Text()) == sharedBlockStart {
			return true, nil
		}
	}
	return false, nil
}

// ScanCodexConfigured は後方互換用。共有ブロック方式の検出を行う。
func ScanCodexConfigured(path string) (bool, error) {
	return ScanSharedBlockConfigured(path)
}

// sharedBlockIsCurrent は注入済み共有ブロック内の version が最新かを確認する
func sharedBlockIsCurrent(path string) (bool, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return false, err
	}
	blockRe := regexp.MustCompile(`(?s)` + regexp.QuoteMeta(sharedBlockStart) + `(.*?)` + regexp.QuoteMeta(sharedBlockEnd))
	m := blockRe.FindSubmatch(data)
	if m == nil {
		return false, nil
	}
	return strings.Contains(string(m[1]), fmt.Sprintf("<!-- version: %s -->", rulesVersion)), nil
}

// ScanRulesConfigured は provider に対応する注入済みマーカーを検出する。
func ScanRulesConfigured(provider, path string) (bool, error) {
	switch {
	case provider == "claude":
		return ScanClaudeConfigured(path)
	case providerUsesSharedBlock(provider):
		return ScanSharedBlockConfigured(path)
	default:
		return false, fmt.Errorf("unknown provider: %s", provider)
	}
}

// appendClaudeImport は CLAUDE.md の末尾に claudeImportLine を追記する
func appendClaudeImport(path string) error {
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return fmt.Errorf("mkdir %s: %w", filepath.Dir(path), err)
	}
	f, err := os.OpenFile(path, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0o644)
	if err != nil {
		return fmt.Errorf("open %s: %w", path, err)
	}
	defer f.Close()
	_, err = fmt.Fprintf(f, "\n%s\n", claudeImportLine)
	return err
}

// appendSharedBlock は AGENTS.md 等の末尾に中央ファイルの内容をブロックとして追記する
func appendSharedBlock(path string) error {
	centralContent, err := os.ReadFile(centralRulesPath())
	if err != nil {
		return fmt.Errorf("read central rules: %w", err)
	}
	block := strings.Join([]string{
		sharedBlockStart,
		strings.TrimSpace(string(centralContent)),
		sharedBlockEnd,
		"",
	}, "\n")
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return fmt.Errorf("mkdir %s: %w", filepath.Dir(path), err)
	}
	f, err := os.OpenFile(path, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0o644)
	if err != nil {
		return fmt.Errorf("open %s: %w", path, err)
	}
	defer f.Close()
	_, err = fmt.Fprintf(f, "\n%s", block)
	return err
}

// appendCodexBlock は後方互換用。共有ブロック方式で追記する。
func appendCodexBlock(path string) error {
	return appendSharedBlock(path)
}

// InjectRules はファイルに承認ルールを注入する
func InjectRules(provider, path string) error {
	if err := SyncRulesFile(); err != nil {
		return fmt.Errorf("sync rules file: %w", err)
	}
	already, err := ScanRulesConfigured(provider, path)
	if err != nil && !os.IsNotExist(err) {
		return fmt.Errorf("scan %s: %w", path, err)
	}
	switch {
	case provider == "claude":
		// import 行方式は中央ファイル側が SyncRulesFile で更新されるため、存在すれば何もしない
		if already {
			return nil
		}
		return appendClaudeImport(path)
	case providerUsesSharedBlock(provider):
		// 共有ブロック方式は内容を埋め込むため、version が古ければ削除して再注入する
		if already {
			current, cerr := sharedBlockIsCurrent(path)
			if cerr != nil {
				return fmt.Errorf("check shared block version %s: %w", path, cerr)
			}
			if current {
				return nil
			}
			if rerr := RemoveRules(provider, path); rerr != nil {
				return fmt.Errorf("remove stale shared block %s: %w", path, rerr)
			}
		}
		return appendSharedBlock(path)
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
	switch {
	case provider == "claude":
		var kept []string
		for _, line := range strings.Split(string(content), "\n") {
			if strings.TrimSpace(line) != claudeImportLine {
				kept = append(kept, line)
			}
		}
		newContent = strings.Join(kept, "\n")
	case providerUsesSharedBlock(provider):
		blockRe := regexp.MustCompile(`(?s)\n?` + regexp.QuoteMeta(sharedBlockStart) + `.*?` + regexp.QuoteMeta(sharedBlockEnd) + `\n?`)
		newContent = blockRe.ReplaceAllString(string(content), "")
	default:
		return fmt.Errorf("unknown provider: %s", provider)
	}
	if newContent == string(content) {
		return nil
	}
	return os.WriteFile(path, []byte(newContent), 0o644)
}
