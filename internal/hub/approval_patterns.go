package hub

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"

	"any-ai-cli/internal/config"
)

// 承認 trigger phrase のハードコードフォールバック値。
// 通常は GitHub の resources/approval-patterns/*.md からリモート取得した内容で
// ~/.any-ai-cli/approval-patterns/<provider>.official.json が更新される。
// fetch が失敗した場合（ネット切断・リポジトリ乗っ取り等）はここの値で初期化される。
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
		"requires approval",
		"do you want to proceed?",
		"would you like to run the following command",
		"would you like to run",
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

// KnownApprovalProviders は承認パターンを管理する provider 名一覧（順序固定）。
func KnownApprovalProviders() []string {
	return []string{"claude", "codex", "common"}
}

// IsKnownApprovalProvider は provider 名が管理対象か判定する。
func IsKnownApprovalProvider(provider string) bool {
	_, ok := defaultApprovalPatterns[provider]
	return ok
}

// IsValidApprovalProfile は profile 名が有効か判定する。
func IsValidApprovalProfile(profile config.ApprovalProfileName) bool {
	return profile == config.ApprovalProfileOfficial || profile == config.ApprovalProfileCustom
}

func approvalPatternsDir() string {
	home, _ := os.UserHomeDir()
	return filepath.Join(home, ".any-ai-cli", "approval-patterns")
}

func approvalProfilePath(provider string, profile config.ApprovalProfileName) string {
	suffix := ".official.json"
	if profile == config.ApprovalProfileCustom {
		suffix = ".custom.json"
	}
	return filepath.Join(approvalPatternsDir(), provider+suffix)
}

// legacyApprovalPath は旧 <provider>.json のパスを返す。
// マイグレーション + フロント互換ミラー出力に使用する。
func legacyApprovalPath(provider string) string {
	return filepath.Join(approvalPatternsDir(), provider+".json")
}

func fileExists(path string) bool {
	_, err := os.Stat(path)
	return err == nil
}

// SyncApprovalPatterns はユーザー設定ディレクトリのファイル構造を整える。
//   - 旧 <provider>.json があり <provider>.custom.json が無ければカスタムへリネーム
//   - <provider>.custom.json が既にあるのに旧 <provider>.json も残っていれば旧を削除
//   - <provider>.official.json が無ければハードコード値で初期化
//   - アクティブプロファイルの内容を <provider>.json にミラー（フロント互換のため）
func SyncApprovalPatterns(profiles config.ApprovalProfiles) error {
	dir := approvalPatternsDir()
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return fmt.Errorf("mkdir %s: %w", dir, err)
	}
	profiles = config.EffectiveApprovalProfiles(profiles)
	for _, provider := range KnownApprovalProviders() {
		legacy := legacyApprovalPath(provider)
		official := approvalProfilePath(provider, config.ApprovalProfileOfficial)
		custom := approvalProfilePath(provider, config.ApprovalProfileCustom)

		legacyExists := fileExists(legacy)
		customExists := fileExists(custom)
		officialExists := fileExists(official)

		if legacyExists && !customExists {
			if err := os.Rename(legacy, custom); err != nil {
				return fmt.Errorf("migrate %s -> custom: %w", provider, err)
			}
			legacyExists = false
			customExists = true
		}
		if legacyExists && customExists {
			_ = os.Remove(legacy)
		}
		if !officialExists {
			if err := writePatternFile(official, defaultApprovalPatterns[provider]); err != nil {
				return err
			}
		}
		_ = customExists // 静的解析よけ（custom 不在のままでも OK）
		if err := writeActiveMirror(provider, profiles.For(provider)); err != nil {
			return err
		}
	}
	return nil
}

// writeActiveMirror はアクティブプロファイルの内容を <provider>.json に書き出す。
// フロント側の既存ロード経路（/approval-patterns/<provider>.json）と互換を保つため。
func writeActiveMirror(provider string, profile config.ApprovalProfileName) error {
	patterns, err := ReadApprovalPatternsByProfile(provider, profile)
	if err != nil {
		return err
	}
	return writePatternFile(legacyApprovalPath(provider), patterns)
}

// RefreshActiveMirrors は全 provider について <provider>.json を再生成する。
// プロファイル切替・custom 上書き・official 更新時に呼ぶ。
func RefreshActiveMirrors(profiles config.ApprovalProfiles) error {
	profiles = config.EffectiveApprovalProfiles(profiles)
	for _, provider := range KnownApprovalProviders() {
		if err := writeActiveMirror(provider, profiles.For(provider)); err != nil {
			return err
		}
	}
	return nil
}

// ReadApprovalPatternsByProfile は指定プロファイルのパターンを読み込む。
// ファイルが存在しない場合：
//   - official: ハードコード defaultApprovalPatterns を返す（初回起動・破損対策）
//   - custom : 空配列を返す
func ReadApprovalPatternsByProfile(provider string, profile config.ApprovalProfileName) ([]string, error) {
	if !IsKnownApprovalProvider(provider) {
		return nil, fmt.Errorf("unknown provider: %s", provider)
	}
	path := approvalProfilePath(provider, profile)
	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			if profile == config.ApprovalProfileOfficial {
				return append([]string{}, defaultApprovalPatterns[provider]...), nil
			}
			return []string{}, nil
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
	return list, nil
}

// ReadActiveApprovalPatterns は全 provider のアクティブプロファイル分をまとめて読み込む。
func ReadActiveApprovalPatterns(profiles config.ApprovalProfiles) (map[string][]string, error) {
	profiles = config.EffectiveApprovalProfiles(profiles)
	result := map[string][]string{}
	for _, provider := range KnownApprovalProviders() {
		list, err := ReadApprovalPatternsByProfile(provider, profiles.For(provider))
		if err != nil {
			return nil, err
		}
		result[provider] = list
	}
	return result, nil
}

// WriteOfficialApprovalPatterns は <provider>.official.json を上書きする（リモート fetch 結果反映用）。
func WriteOfficialApprovalPatterns(provider string, patterns []string) error {
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
	return writePatternFile(approvalProfilePath(provider, config.ApprovalProfileOfficial), patterns)
}

// WriteCustomApprovalPatterns は <provider>.custom.json を上書きする。
func WriteCustomApprovalPatterns(provider string, patterns []string) error {
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
	return writePatternFile(approvalProfilePath(provider, config.ApprovalProfileCustom), patterns)
}

// CopyOfficialToCustom は official プロファイルの内容を custom にコピーする。
func CopyOfficialToCustom(provider string) error {
	patterns, err := ReadApprovalPatternsByProfile(provider, config.ApprovalProfileOfficial)
	if err != nil {
		return err
	}
	return WriteCustomApprovalPatterns(provider, patterns)
}

func writePatternFile(path string, patterns []string) error {
	if patterns == nil {
		patterns = []string{}
	}
	data, err := json.MarshalIndent(patterns, "", "  ")
	if err != nil {
		return fmt.Errorf("marshal %s: %w", path, err)
	}
	if err := os.WriteFile(path, append(data, '\n'), 0o644); err != nil {
		return fmt.Errorf("write %s: %w", path, err)
	}
	return nil
}
