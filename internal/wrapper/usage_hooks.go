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

	claudeSettingsLocalFilename = "settings.local.json"
	claudeSettingsLocalDir      = ".claude"
)

// UsageHookParams は注入時に埋め込む接続パラメータ。
type UsageHookParams struct {
	HubURL    string
	Token     string
	SessionID int
	ExePath   string // any-ai-cli バイナリのフルパス（os.Executable() で解決済み）
}

// ---------------------------------------------------------------------------
// Claude: .claude/settings.local.json への statusLine 冪等注入
// ---------------------------------------------------------------------------
//
// Claude Code の statusLine スキーマ（実機確認済み）:
//
//	"statusLine": {
//	  "type": "command",
//	  "command": "<cmd>",
//	  "padding": 0
//	}
//
// 既存の statusLine があれば "__aac_orig_statusLine" キーにバックアップして
// 上書きする。全セッション終了時に RemoveClaudeStatusLine で復元する。

const claudeStatusLineKey = "statusLine"
const claudeStatusLineBackupKey = "__aac_orig_statusLine"

// claudeStatusLineCmd は relay コマンド文字列を組み立てる。
func claudeStatusLineCmd(p UsageHookParams) string {
	return fmt.Sprintf("%s usage-relay --provider claude --hub %s --token %s --session %d",
		p.ExePath, p.HubURL, p.Token, p.SessionID)
}

// claudeSettingsLocalPath は <cwd>/.claude/settings.local.json のパスを返す。
func claudeSettingsLocalPath(cwd string) string {
	return filepath.Join(cwd, claudeSettingsLocalDir, claudeSettingsLocalFilename)
}

// InjectClaudeStatusLine は cwd の .claude/settings.local.json に statusLine を注入する。
// 既存の statusLine があれば "__aac_orig_statusLine" にバックアップして上書きする。
// 冪等（already 注入済みなら無操作）。
func InjectClaudeStatusLine(cwd string, p UsageHookParams) error {
	path := claudeSettingsLocalPath(cwd)

	// 既存ファイルを読み込む（無ければ空 map）。
	obj := map[string]json.RawMessage{}
	if data, err := os.ReadFile(path); err == nil {
		if jErr := json.Unmarshal(data, &obj); jErr != nil {
			// 破損していたら空 map で続行（上書きは危険なためスキップ）
			return fmt.Errorf("parse %s: %w", path, jErr)
		}
	} else if !os.IsNotExist(err) {
		return fmt.Errorf("read %s: %w", path, err)
	}

	// already 注入済み（backup キーが存在）かどうか確認。
	if _, exists := obj[claudeStatusLineBackupKey]; exists {
		// backup キーがあれば注入済みとみなし、command だけ更新する。
		newSL, err := buildClaudeStatusLineJSON(p)
		if err != nil {
			return err
		}
		obj[claudeStatusLineKey] = newSL
		return writeSettingsLocalJSON(path, obj)
	}

	// 既存 statusLine があればバックアップ。
	if existing, exists := obj[claudeStatusLineKey]; exists {
		obj[claudeStatusLineBackupKey] = existing
	} else {
		// 無い場合も null でバックアップキーを立てて「注入済み」マーカーにする。
		obj[claudeStatusLineBackupKey] = json.RawMessage("null")
	}

	newSL, err := buildClaudeStatusLineJSON(p)
	if err != nil {
		return err
	}
	obj[claudeStatusLineKey] = newSL

	return writeSettingsLocalJSON(path, obj)
}

// buildClaudeStatusLineJSON は statusLine フィールドの JSON を構築する。
func buildClaudeStatusLineJSON(p UsageHookParams) (json.RawMessage, error) {
	type statusLineValue struct {
		Type    string `json:"type"`
		Command string `json:"command"`
		Padding int    `json:"padding"`
	}
	v := statusLineValue{
		Type:    "command",
		Command: claudeStatusLineCmd(p),
		Padding: 0,
	}
	b, err := json.Marshal(v)
	if err != nil {
		return nil, fmt.Errorf("marshal statusLine: %w", err)
	}
	return json.RawMessage(b), nil
}

// RemoveClaudeStatusLine は注入した statusLine を除去し、バックアップを復元する。
func RemoveClaudeStatusLine(cwd string) error {
	path := claudeSettingsLocalPath(cwd)

	data, err := os.ReadFile(path)
	if os.IsNotExist(err) {
		return nil
	}
	if err != nil {
		return fmt.Errorf("read %s: %w", path, err)
	}

	obj := map[string]json.RawMessage{}
	if jErr := json.Unmarshal(data, &obj); jErr != nil {
		return fmt.Errorf("parse %s: %w", path, jErr)
	}

	backup, hasBackup := obj[claudeStatusLineBackupKey]
	if !hasBackup {
		// 注入されていない
		return nil
	}

	delete(obj, claudeStatusLineBackupKey)

	// backup が null なら statusLine ごと削除、それ以外なら復元。
	if string(backup) == "null" {
		delete(obj, claudeStatusLineKey)
	} else {
		obj[claudeStatusLineKey] = backup
	}

	return writeSettingsLocalJSON(path, obj)
}

// ScanClaudeStatusLineInjected は注入済みかどうかを確認する。
func ScanClaudeStatusLineInjected(cwd string) (bool, error) {
	path := claudeSettingsLocalPath(cwd)
	data, err := os.ReadFile(path)
	if os.IsNotExist(err) {
		return false, nil
	}
	if err != nil {
		return false, fmt.Errorf("read %s: %w", path, err)
	}
	obj := map[string]json.RawMessage{}
	if jErr := json.Unmarshal(data, &obj); jErr != nil {
		return false, nil
	}
	_, exists := obj[claudeStatusLineBackupKey]
	return exists, nil
}

func writeSettingsLocalJSON(path string, obj map[string]json.RawMessage) error {
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil { // #nosec G301 -- .claude/ は他ツール共有ディレクトリ
		return fmt.Errorf("mkdir %s: %w", filepath.Dir(path), err)
	}
	out, err := json.MarshalIndent(obj, "", "  ")
	if err != nil {
		return fmt.Errorf("marshal settings.local.json: %w", err)
	}
	return os.WriteFile(path, append(out, '\n'), 0o644) // #nosec G306 -- settings.local.json は他ツール共有（秘密情報なし）
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
