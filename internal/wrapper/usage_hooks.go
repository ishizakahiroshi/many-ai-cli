package wrapper

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"strings"
)

// ---------------------------------------------------------------------------
// 共通定数・型
// ---------------------------------------------------------------------------

const (
	usageHookBlockStart = "# any-ai-cli:usage-hook-start"
	usageHookBlockEnd   = "# any-ai-cli:usage-hook-end"
)

// UsageHookParams は注入時に埋め込む接続パラメータ。
type UsageHookParams struct {
	HubURL    string
	Token     string
	SessionID int
	ExePath   string // any-ai-cli バイナリのフルパス（os.Executable() で解決済み）
}

// ---------------------------------------------------------------------------
// Claude: wrapper 所有の一時 settings ファイル経由で statusLine を渡す
// ---------------------------------------------------------------------------
//
// 共有ファイル .claude/settings.local.json には一切書き込まない（重要）。
// Claude Code 本体も権限承認のたびに同ファイルを書き換え、ユーザーも手編集する
// ため、後付け注入は衝突・手編集ミスで壊れ続ける（過去に単一バックスラッシュの
// Windows パスで JSON 全体が不正化し、Claude が settings を読めず statusLine が
// 無効化された。さらに旧実装は破損ファイルを安全のためスキップする設計だったので
// 一度壊れると永久に再注入できず「トークンが流れてこない」状態に固定された）。
//
// 代わりに claude 起動時に `claude --settings <temp>` を渡す。--settings は設定
// 階層のうちコマンドライン引数（local/project/user より上・managed の下）として
// マージされるため、temp に statusLine だけ書けば有効になる。temp は wrapper だけ
// が所有し起動ごとに作り直すので、共有衝突が原理的に発生せず、万一壊れても次回
// 起動で上書きされる。

// claudeStatusLineCmd は relay コマンド文字列を組み立てる。
func claudeStatusLineCmd(p UsageHookParams) string {
	return fmt.Sprintf("%s usage-relay --provider claude --hub %s --token %s --session %d",
		p.ExePath, p.HubURL, p.Token, p.SessionID)
}

// WriteClaudeStatuslineSettings は statusLine だけを含む wrapper 所有の一時
// settings JSON を書き出し、そのパスと後始末関数を返す。
// `claude --settings <path>` に渡して使う。
// JSON は json.Marshal で生成するため Windows パスのバックスラッシュも常に
// 正しくエスケープされ、手編集由来の破損は起こり得ない。
func WriteClaudeStatuslineSettings(p UsageHookParams) (path string, cleanup func(), err error) {
	type statusLineValue struct {
		Type    string `json:"type"`
		Command string `json:"command"`
		Padding int    `json:"padding"`
	}
	doc := map[string]any{
		"statusLine": statusLineValue{
			Type:    "command",
			Command: claudeStatusLineCmd(p),
			Padding: 0,
		},
	}
	body, err := json.MarshalIndent(doc, "", "  ")
	if err != nil {
		return "", nil, fmt.Errorf("marshal statusline settings: %w", err)
	}
	name := fmt.Sprintf("aac-claude-statusline-s%d-%d.json", p.SessionID, os.Getpid())
	path = filepath.Join(os.TempDir(), name)
	// 0600: Hub URL と token を含むため他ユーザーに読ませない。
	if err := os.WriteFile(path, append(body, '\n'), 0o600); err != nil { // #nosec G306 -- wrapper 専用 temp（token を含むため 0600 が意図）
		return "", nil, fmt.Errorf("write statusline settings: %w", err)
	}
	cleanup = func() { _ = os.Remove(path) }
	return path, cleanup, nil
}

// ---------------------------------------------------------------------------
// Codex: ~/.codex/config.toml への Stop フック冪等注入
//
// Codex の Stop フック設定形式（要実機確認: フォーマットが確定次第更新）:
//
//	[[hooks.Stop]]
//	command = "<cmd>"
//
// マーカーコメントで自前注入ブロックを識別し、全セッション終了時に除去する。
// ---------------------------------------------------------------------------

// codexConfigPath は ~/.codex/config.toml のパスを返す。
func codexConfigPath() string {
	home, _ := os.UserHomeDir()
	codexHome := os.Getenv("CODEX_HOME")
	if codexHome == "" {
		codexHome = filepath.Join(home, ".codex")
	}
	return filepath.Join(codexHome, "config.toml")
}

// codexStopHookBlock は注入するブロックテキストを返す。
// 注意: Codex の config.toml フォーマットは要実機確認。
// 現在は OpenAI Codex CLI の [[hooks.Stop]] TOML テーブル配列形式を想定。
func codexStopHookBlock(p UsageHookParams) string {
	cmd := fmt.Sprintf("%s usage-relay --provider codex --hub %s --token %s --session %d",
		p.ExePath, p.HubURL, p.Token, p.SessionID)
	return strings.Join([]string{
		usageHookBlockStart,
		"[[hooks.Stop]]",
		fmt.Sprintf("command = %q", cmd),
		usageHookBlockEnd,
		"",
	}, "\n")
}

// InjectCodexStopHook は ~/.codex/config.toml に Stop フックを冪等注入する。
func InjectCodexStopHook(p UsageHookParams) error {
	path := codexConfigPath()

	// 既存ファイルを読む（無ければ空）。
	var content string
	if data, err := os.ReadFile(path); err == nil {
		content = string(data)
	} else if !os.IsNotExist(err) {
		return fmt.Errorf("read %s: %w", path, err)
	}

	// already 注入済みかどうか確認。
	if strings.Contains(content, usageHookBlockStart) {
		// 注入済み: コマンドだけ更新（セッション ID が変わる場合を考慮）。
		newBlock := codexStopHookBlock(p)
		blockRe := regexp.MustCompile(`(?s)` + regexp.QuoteMeta(usageHookBlockStart) + `.*?` + regexp.QuoteMeta(usageHookBlockEnd) + `\n?`)
		content = blockRe.ReplaceAllString(content, newBlock)
		return writeCodexConfig(path, content)
	}

	// 末尾に追記。
	if !strings.HasSuffix(content, "\n") && len(content) > 0 {
		content += "\n"
	}
	content += "\n" + codexStopHookBlock(p)

	return writeCodexConfig(path, content)
}

// RemoveCodexStopHook は注入した Stop フックブロックを除去する。
func RemoveCodexStopHook() error {
	path := codexConfigPath()
	data, err := os.ReadFile(path)
	if os.IsNotExist(err) {
		return nil
	}
	if err != nil {
		return fmt.Errorf("read %s: %w", path, err)
	}

	content := string(data)
	if !strings.Contains(content, usageHookBlockStart) {
		return nil
	}

	blockRe := regexp.MustCompile(`(?s)\n?` + regexp.QuoteMeta(usageHookBlockStart) + `.*?` + regexp.QuoteMeta(usageHookBlockEnd) + `\n?`)
	newContent := blockRe.ReplaceAllString(content, "")
	if newContent == content {
		return nil
	}

	return writeCodexConfig(path, newContent)
}

// ScanCodexStopHookInjected は注入済みかどうかを確認する。
func ScanCodexStopHookInjected() (bool, error) {
	path := codexConfigPath()
	data, err := os.ReadFile(path)
	if os.IsNotExist(err) {
		return false, nil
	}
	if err != nil {
		return false, fmt.Errorf("read %s: %w", path, err)
	}
	return strings.Contains(string(data), usageHookBlockStart), nil
}

func writeCodexConfig(path, content string) error {
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil { // #nosec G301 -- ~/.codex は秘密情報を持つ可能性があるため 0700
		return fmt.Errorf("mkdir %s: %w", filepath.Dir(path), err)
	}
	return os.WriteFile(path, []byte(content), 0o600) // #nosec G306 -- ~/.codex/config.toml は Codex CLI の設定ファイル（0600 が意図）
}
