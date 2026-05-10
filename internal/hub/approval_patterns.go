package hub

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
)

// 承認 trigger phrase のデフォルト値。Hub 初回起動時にユーザー設定ディレクトリ
// (~/.ai-cli-hub/approval-patterns/) に書き出す。既存ファイルがあれば
// ユーザー編集を尊重して上書きしない。リセットしたい場合はファイルを削除すれば
// 次回起動で再生成される。
//
// 各 provider の承認 UI 文言は基本的に英語ハードコード（Anthropic / OpenAI 側で
// 国際化されない）なので、claude / codex は英語固定。common は Hub マーカー周辺の
// ユーザー会話言語に追従する文言を多言語混在で持つ。
var defaultApprovalPatterns = map[string][]string{
	"claude": {
		"do you want to",
		"esc to cancel",
		"press enter to confirm or esc to go back",
	},
	"codex": {
		"approve?",
		"continue?",
		"proceed?",
		"select model and effort",
		"select model",
		"press enter to confirm",
		"esc to go back",
		"↑/↓ to change",
		"arrow keys",
		"permission",
	},
	"common": {
		"would you like to",
		"この操作を許可",
		"続行しますか",
		"承認しますか",
		"実行しますか",
	},
}

func approvalPatternsDir() string {
	home, _ := os.UserHomeDir()
	return filepath.Join(home, ".ai-cli-hub", "approval-patterns")
}

// SyncApprovalPatterns はデフォルトパターンをユーザー設定ディレクトリに展開する。
// 既存ファイルは上書きしない（ユーザー編集を尊重）。
func SyncApprovalPatterns() error {
	dir := approvalPatternsDir()
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return fmt.Errorf("mkdir %s: %w", dir, err)
	}
	for name, patterns := range defaultApprovalPatterns {
		path := filepath.Join(dir, name+".json")
		if _, err := os.Stat(path); err == nil {
			continue
		}
		if err := writePatternFile(path, patterns); err != nil {
			return err
		}
	}
	return nil
}

// IsKnownApprovalProvider は provider 名がデフォルトに含まれるか判定する
func IsKnownApprovalProvider(provider string) bool {
	_, ok := defaultApprovalPatterns[provider]
	return ok
}

// ReadApprovalPatterns はユーザー設定ディレクトリから全 provider のパターンを読み込む
func ReadApprovalPatterns() (map[string][]string, error) {
	dir := approvalPatternsDir()
	result := map[string][]string{}
	for name := range defaultApprovalPatterns {
		path := filepath.Join(dir, name+".json")
		data, err := os.ReadFile(path)
		if err != nil {
			if os.IsNotExist(err) {
				result[name] = []string{}
				continue
			}
			return nil, fmt.Errorf("read %s: %w", path, err)
		}
		var list []string
		if len(data) > 0 {
			if err := json.Unmarshal(data, &list); err != nil {
				return nil, fmt.Errorf("parse %s: %w", path, err)
			}
		}
		if list == nil {
			list = []string{}
		}
		result[name] = list
	}
	return result, nil
}

// WriteApprovalPatterns はユーザー設定ディレクトリの指定 provider に書き込む
func WriteApprovalPatterns(provider string, patterns []string) error {
	if !IsKnownApprovalProvider(provider) {
		return fmt.Errorf("unknown provider: %s", provider)
	}
	dir := approvalPatternsDir()
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return fmt.Errorf("mkdir %s: %w", dir, err)
	}
	if patterns == nil {
		patterns = []string{}
	}
	path := filepath.Join(dir, provider+".json")
	return writePatternFile(path, patterns)
}

// ResetApprovalPatterns は指定 provider をデフォルト値で上書きする
func ResetApprovalPatterns(provider string) error {
	patterns, ok := defaultApprovalPatterns[provider]
	if !ok {
		return fmt.Errorf("unknown provider: %s", provider)
	}
	return WriteApprovalPatterns(provider, patterns)
}

func writePatternFile(path string, patterns []string) error {
	data, err := json.MarshalIndent(patterns, "", "  ")
	if err != nil {
		return fmt.Errorf("marshal %s: %w", path, err)
	}
	if err := os.WriteFile(path, append(data, '\n'), 0o644); err != nil {
		return fmt.Errorf("write %s: %w", path, err)
	}
	return nil
}
