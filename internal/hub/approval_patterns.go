package hub

import (
	"encoding/json"
	"fmt"
	"net/http"
	"os"
	"path/filepath"
	"strings"

	"many-ai-cli/internal/config"
)

// 承認 trigger phrase のハードコードフォールバック値。
// 通常は GitHub の resources/approval-patterns/*.md からリモート取得した内容で
// ~/.many-ai-cli/approval-patterns/<provider>.official.json が更新される。
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
		"permission",
	},
	"copilot": {
		"permission required",
		"permissions required",
		"requires permission",
		"requires confirmation",
		"prompts for user confirmation",
		"allow all similar",
		"deny all similar",
	},
	// cursor-agent 実機 UI（コマンド allowlist 確認ダイアログ）の文言を正とする。
	"cursor-agent": {
		"run this command?",
		"not in allowlist",
		"allowlist",
		"auto-run everything",
		"skip (esc or n)",
		"permission required",
		"requires permission",
		"requires confirmation",
	},
	// opencode 実機 UI（Build agent 承認ダイアログ）の文言を正とする。
	// 選択は左右矢印 + Enter（数字キー不使用）。
	"opencode": {
		"permission required",
		"allow once",
		"allow always",
		"reject",
	},
	// grok (Grok Build) は Claude Code 互換 harness（--permission-mode / CLAUDE.md）。
	// 承認 UI も Claude Code 系と推定し、暫定で Claude 系 + 汎用文言を採用する。
	// 実機 TUI の承認プロンプトで確定する（plan_grok-build-provider-integration.md C3）。
	"grok": {
		"do you want to",
		"esc to cancel",
		"press enter to confirm",
		"allow this command",
		"permission required",
		"approve?",
		"proceed?",
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
	return []string{"claude", "codex", "copilot", "cursor-agent", "opencode", "grok", "common"}
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
	return filepath.Join(home, ".many-ai-cli", "approval-patterns")
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

func ensureApprovalPatternsDir() (string, error) {
	dir := approvalPatternsDir()
	if err := os.MkdirAll(dir, config.DirMode); err != nil {
		return "", fmt.Errorf("mkdir %s: %w", dir, err)
	}
	if err := os.Chmod(filepath.Dir(dir), config.DirMode); err != nil {
		return "", fmt.Errorf("chmod %s: %w", filepath.Dir(dir), err)
	}
	if err := os.Chmod(dir, config.DirMode); err != nil {
		return "", fmt.Errorf("chmod %s: %w", dir, err)
	}
	return dir, nil
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
	if _, err := ensureApprovalPatternsDir(); err != nil {
		return err
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
	if _, err := ensureApprovalPatternsDir(); err != nil {
		return err
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
	if _, err := ensureApprovalPatternsDir(); err != nil {
		return err
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
	if _, err := ensureApprovalPatternsDir(); err != nil {
		return err
	}
	if err := os.WriteFile(path, append(data, '\n'), 0o600); err != nil {
		return fmt.Errorf("write %s: %w", path, err)
	}
	return nil
}

func validApprovalPatternAssetName(name string) bool {
	name = strings.TrimSpace(name)
	if name == "" || strings.HasPrefix(name, ".") || strings.ContainsAny(name, `/\`) {
		return false
	}
	if !strings.HasSuffix(name, ".json") {
		return false
	}
	base := strings.TrimSuffix(name, ".json")
	for _, suffix := range []string{".official", ".custom"} {
		if strings.HasSuffix(base, suffix) {
			base = strings.TrimSuffix(base, suffix)
			break
		}
	}
	return IsKnownApprovalProvider(base)
}

func (s *Server) handleApprovalPatternAsset(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	name := strings.TrimPrefix(r.URL.Path, "/approval-patterns/")
	if !validApprovalPatternAssetName(name) {
		writeJSONError(w, http.StatusNotFound, "not_found", "not found")
		return
	}
	dir := approvalPatternsDir()
	path := filepath.Join(dir, name)
	if ok, _ := isPathUnderAllowedRoots(path, dir); !ok {
		writeJSONError(w, http.StatusForbidden, "forbidden", "forbidden")
		return
	}
	w.Header().Set("Content-Type", "application/json; charset=utf-8")
	w.Header().Set("X-Content-Type-Options", "nosniff")
	http.ServeFile(w, r, path) // #nosec G703 -- validApprovalPatternAssetName + isPathUnderAllowedRoots で検証済み
}

func (s *Server) handleApprovalPatterns(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	s.cfgMu.Lock()
	profiles := s.cfg.ApprovalProfiles
	s.cfgMu.Unlock()
	patterns, err := ReadActiveApprovalPatterns(profiles)
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "read_failed", err.Error())
		return
	}
	writeJSON(w, patterns)
}

func (s *Server) handleApprovalPatternsItem(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r) {
		return
	}
	rest := strings.TrimPrefix(r.URL.Path, "/api/approval-patterns/")
	switch rest {
	case "profile":
		s.handleApprovalProfile(w, r)
		return
	case "copy-official":
		s.handleApprovalCopyOfficial(w, r)
		return
	}
	parts := strings.SplitN(rest, "/", 2)
	provider := parts[0]
	if provider == "" || !IsKnownApprovalProvider(provider) {
		writeJSONError(w, http.StatusNotFound, "not_found", "unknown provider")
		return
	}
	if !requireMethod(w, r, http.MethodPut) {
		return
	}
	var list []string
	if !decodeJSON(w, r, &list) {
		return
	}
	cleaned := make([]string, 0, len(list))
	for _, item := range list {
		v := strings.TrimSpace(item)
		if v != "" {
			cleaned = append(cleaned, v)
		}
	}
	if err := WriteCustomApprovalPatterns(provider, cleaned); err != nil {
		writeJSONError(w, http.StatusInternalServerError, "write_failed", err.Error())
		return
	}
	s.refreshActiveMirror()
	w.WriteHeader(http.StatusNoContent)
}

// handleApprovalProfile は GET でアクティブプロファイル一覧、POST で切替を行う。
// POST body: {"provider":"claude","profile":"official"|"custom"}
func (s *Server) handleApprovalProfile(w http.ResponseWriter, r *http.Request) {
	switch r.Method {
	case http.MethodGet:
		s.cfgMu.Lock()
		profiles := config.EffectiveApprovalProfiles(s.cfg.ApprovalProfiles)
		s.cfgMu.Unlock()
		writeJSON(w, profiles)
	case http.MethodPost:
		var body struct {
			Provider string `json:"provider"`
			Profile  string `json:"profile"`
		}
		if !decodeJSON(w, r, &body) {
			return
		}
		if !IsKnownApprovalProvider(body.Provider) {
			writeJSONError(w, http.StatusBadRequest, "bad_request", "unknown provider")
			return
		}
		profile := config.ApprovalProfileName(body.Profile)
		if !IsValidApprovalProfile(profile) {
			writeJSONError(w, http.StatusBadRequest, "bad_request", "invalid profile")
			return
		}
		s.cfgMu.Lock()
		s.cfg.ApprovalProfiles = config.EffectiveApprovalProfiles(s.cfg.ApprovalProfiles).WithProvider(body.Provider, profile)
		s.cfgMu.Unlock()
		if err := s.persistConfig(); err != nil {
			s.logger.Warn("save config failed", "err", err)
		}
		s.refreshActiveMirror()
		w.WriteHeader(http.StatusNoContent)
	default:
		writeJSONError(w, http.StatusMethodNotAllowed, "method_not_allowed", "method not allowed")
	}
}

// handleApprovalCopyOfficial は official → custom コピーを行う。
// POST body: {"provider":"claude"}
func (s *Server) handleApprovalCopyOfficial(w http.ResponseWriter, r *http.Request) {
	if !requireMethod(w, r, http.MethodPost) {
		return
	}
	var body struct {
		Provider string `json:"provider"`
	}
	if !decodeJSON(w, r, &body) {
		return
	}
	if !IsKnownApprovalProvider(body.Provider) {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "unknown provider")
		return
	}
	if err := CopyOfficialToCustom(body.Provider); err != nil {
		writeJSONError(w, http.StatusInternalServerError, "copy_failed", err.Error())
		return
	}
	s.refreshActiveMirror()
	w.WriteHeader(http.StatusNoContent)
}

// refreshActiveMirror はアクティブプロファイル内容を <provider>.json に再書き出しする。
// プロファイル切替・custom 上書き・official 更新後に呼ぶ。失敗時は warn ログのみ。
func (s *Server) refreshActiveMirror() {
	s.cfgMu.Lock()
	profiles := s.cfg.ApprovalProfiles
	s.cfgMu.Unlock()
	if err := RefreshActiveMirrors(profiles); err != nil {
		s.logger.Warn("refresh approval pattern mirrors failed", "err", err)
	}
}
