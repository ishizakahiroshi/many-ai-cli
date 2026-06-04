package config

import (
	"crypto/rand"
	"encoding/hex"
	"fmt"
	"log/slog"
	"os"
	"path/filepath"

	"gopkg.in/yaml.v3"

	"any-ai-cli/internal/wslutil"
)

const DirMode os.FileMode = 0o700

// Dir は ~/.any-ai-cli ディレクトリのパスを返す。
func Dir() (string, error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return "", fmt.Errorf("home dir: %w", err)
	}
	return filepath.Join(home, ".any-ai-cli"), nil
}

// Path は ~/.any-ai-cli/config.yaml のパスを返す。
func Path() (string, error) {
	dir, err := Dir()
	if err != nil {
		return "", err
	}
	return filepath.Join(dir, "config.yaml"), nil
}

// LogConfig はファイルローテーションロギングの設定。
type LogConfig struct {
	Enabled              bool `yaml:"enabled" json:"enabled"`
	MaxSizeMB            int  `yaml:"max_size_mb" json:"max_size_mb"`
	MaxBackups           int  `yaml:"max_backups" json:"max_backups"`
	Compress             bool `yaml:"compress" json:"compress"`
	SessionRetentionDays int  `yaml:"session_retention_days" json:"session_retention_days"`
	SessionMaxSizeMB     int  `yaml:"session_max_size_mb" json:"session_max_size_mb"`
}

// ApprovalConfig は Hub 承認ボタン機能の設定。
type ApprovalConfig struct {
	Enabled          bool `yaml:"enabled"`
	FirstLaunchShown bool `yaml:"first_launch_shown"`
}

// SlashCmdSources は provider ごとのスラッシュコマンド取得元。
// URL またはローカルの .md/.txt パスを指定できる。
type SlashCmdSources struct {
	Claude      string `yaml:"claude"  json:"claude"`
	Codex       string `yaml:"codex"   json:"codex"`
	Copilot     string `yaml:"copilot" json:"copilot"`
	CursorAgent string `yaml:"cursor-agent" json:"cursor-agent"`
}

const (
	LegacyClaudeSlashCmdSource       = "https://code.claude.com/docs/en/commands.md"
	DefaultClaudeSlashCmdSource      = "https://raw.githubusercontent.com/ishizakahiroshi/any-ai-cli/main/resources/slash-commands/claude.md"
	DefaultCodexSlashCmdSource       = "https://raw.githubusercontent.com/ishizakahiroshi/any-ai-cli/main/resources/slash-commands/codex.md"
	DefaultCopilotSlashCmdSource     = "https://raw.githubusercontent.com/ishizakahiroshi/any-ai-cli/main/resources/slash-commands/copilot.md"
	DefaultCursorAgentSlashCmdSource = "https://raw.githubusercontent.com/ishizakahiroshi/any-ai-cli/main/resources/slash-commands/cursor-agent.md"
)

const DefaultUsageLinkSource = "https://raw.githubusercontent.com/ishizakahiroshi/any-ai-cli/main/resources/usage-links/defaults.json"

const DefaultModelsSource = "https://raw.githubusercontent.com/ishizakahiroshi/any-ai-cli/main/resources/models/defaults.json"

func DefaultSlashCmdSources() SlashCmdSources {
	return SlashCmdSources{
		Claude:      DefaultClaudeSlashCmdSource,
		Codex:       DefaultCodexSlashCmdSource,
		Copilot:     DefaultCopilotSlashCmdSource,
		CursorAgent: DefaultCursorAgentSlashCmdSource,
	}
}

func EffectiveSlashCmdSources(src SlashCmdSources) SlashCmdSources {
	defaults := DefaultSlashCmdSources()
	if src.Claude == "" {
		src.Claude = defaults.Claude
	} else if src.Claude == LegacyClaudeSlashCmdSource {
		// Migrate historical default to GitHub-backed source.
		src.Claude = defaults.Claude
	}
	if src.Codex == "" {
		src.Codex = defaults.Codex
	}
	if src.Copilot == "" {
		src.Copilot = defaults.Copilot
	}
	if src.CursorAgent == "" {
		src.CursorAgent = defaults.CursorAgent
	}
	return src
}

// ApprovalPatternSources は provider ごとのリモート md 取得元。
// SlashCmdSources と同じ構造で、空文字なら Default*ApprovalPatternSource を使う。
type ApprovalPatternSources struct {
	Claude      string `yaml:"claude,omitempty"  json:"claude,omitempty"`
	Codex       string `yaml:"codex,omitempty"   json:"codex,omitempty"`
	Copilot     string `yaml:"copilot,omitempty" json:"copilot,omitempty"`
	CursorAgent string `yaml:"cursor-agent,omitempty" json:"cursor-agent,omitempty"`
	Common      string `yaml:"common,omitempty"  json:"common,omitempty"`
}

const (
	DefaultClaudeApprovalPatternSource      = "https://raw.githubusercontent.com/ishizakahiroshi/any-ai-cli/main/resources/approval-patterns/claude.md"
	DefaultCodexApprovalPatternSource       = "https://raw.githubusercontent.com/ishizakahiroshi/any-ai-cli/main/resources/approval-patterns/codex.md"
	DefaultCopilotApprovalPatternSource     = "https://raw.githubusercontent.com/ishizakahiroshi/any-ai-cli/main/resources/approval-patterns/copilot.md"
	DefaultCursorAgentApprovalPatternSource = "https://raw.githubusercontent.com/ishizakahiroshi/any-ai-cli/main/resources/approval-patterns/cursor-agent.md"
	DefaultCommonApprovalPatternSource      = "https://raw.githubusercontent.com/ishizakahiroshi/any-ai-cli/main/resources/approval-patterns/common.md"
)

func DefaultApprovalPatternSources() ApprovalPatternSources {
	return ApprovalPatternSources{
		Claude:      DefaultClaudeApprovalPatternSource,
		Codex:       DefaultCodexApprovalPatternSource,
		Copilot:     DefaultCopilotApprovalPatternSource,
		CursorAgent: DefaultCursorAgentApprovalPatternSource,
		Common:      DefaultCommonApprovalPatternSource,
	}
}

func EffectiveApprovalPatternSources(src ApprovalPatternSources) ApprovalPatternSources {
	defaults := DefaultApprovalPatternSources()
	if src.Claude == "" {
		src.Claude = defaults.Claude
	}
	if src.Codex == "" {
		src.Codex = defaults.Codex
	}
	if src.Copilot == "" {
		src.Copilot = defaults.Copilot
	}
	if src.CursorAgent == "" {
		src.CursorAgent = defaults.CursorAgent
	}
	if src.Common == "" {
		src.Common = defaults.Common
	}
	return src
}

// ApprovalProfileName は "official" | "custom"
type ApprovalProfileName string

const (
	ApprovalProfileOfficial ApprovalProfileName = "official"
	ApprovalProfileCustom   ApprovalProfileName = "custom"
)

// ApprovalProfiles は provider ごとのアクティブプロファイル。
type ApprovalProfiles struct {
	Claude      ApprovalProfileName `yaml:"claude,omitempty"  json:"claude,omitempty"`
	Codex       ApprovalProfileName `yaml:"codex,omitempty"   json:"codex,omitempty"`
	Copilot     ApprovalProfileName `yaml:"copilot,omitempty" json:"copilot,omitempty"`
	CursorAgent ApprovalProfileName `yaml:"cursor-agent,omitempty" json:"cursor-agent,omitempty"`
	Common      ApprovalProfileName `yaml:"common,omitempty"  json:"common,omitempty"`
}

// DefaultApprovalProfiles は新規ユーザー向けデフォルト（全 provider official）。
func DefaultApprovalProfiles() ApprovalProfiles {
	return ApprovalProfiles{
		Claude:      ApprovalProfileOfficial,
		Codex:       ApprovalProfileOfficial,
		Copilot:     ApprovalProfileOfficial,
		CursorAgent: ApprovalProfileOfficial,
		Common:      ApprovalProfileOfficial,
	}
}

// EffectiveApprovalProfiles は空フィールドを official で埋めて返す。
func EffectiveApprovalProfiles(p ApprovalProfiles) ApprovalProfiles {
	if p.Claude == "" {
		p.Claude = ApprovalProfileOfficial
	}
	if p.Codex == "" {
		p.Codex = ApprovalProfileOfficial
	}
	if p.Copilot == "" {
		p.Copilot = ApprovalProfileOfficial
	}
	if p.CursorAgent == "" {
		p.CursorAgent = ApprovalProfileOfficial
	}
	if p.Common == "" {
		p.Common = ApprovalProfileOfficial
	}
	return p
}

// For は provider 名から対応するプロファイルを返す。未知の provider は official。
func (p ApprovalProfiles) For(provider string) ApprovalProfileName {
	switch provider {
	case "claude":
		if p.Claude != "" {
			return p.Claude
		}
	case "codex":
		if p.Codex != "" {
			return p.Codex
		}
	case "copilot":
		if p.Copilot != "" {
			return p.Copilot
		}
	case "cursor-agent":
		if p.CursorAgent != "" {
			return p.CursorAgent
		}
	case "common":
		if p.Common != "" {
			return p.Common
		}
	}
	return ApprovalProfileOfficial
}

// WithProvider は指定 provider のプロファイルを差し替えた新しい構造体を返す。
func (p ApprovalProfiles) WithProvider(provider string, name ApprovalProfileName) ApprovalProfiles {
	switch provider {
	case "claude":
		p.Claude = name
	case "codex":
		p.Codex = name
	case "copilot":
		p.Copilot = name
	case "cursor-agent":
		p.CursorAgent = name
	case "common":
		p.Common = name
	}
	return p
}

// UserPrefsNotifySound は通知音の設定。
type UserPrefsNotifySound struct {
	Enabled    bool   `yaml:"enabled,omitempty"     json:"enabled,omitempty"`
	Type       string `yaml:"type,omitempty"        json:"type,omitempty"`
	CustomFile string `yaml:"custom_file,omitempty" json:"custom_file,omitempty"`
	CustomMime string `yaml:"custom_mime,omitempty" json:"custom_mime,omitempty"`
}

// UserPrefsTrigger はトリガーフレーズの設定。
type UserPrefsTrigger struct {
	Enabled bool   `yaml:"enabled,omitempty" json:"enabled,omitempty"`
	Phrase  string `yaml:"phrase,omitempty"  json:"phrase,omitempty"`
}

// UserPrefsApproval は承認関連のユーザー設定。
type UserPrefsApproval struct {
	AutoSwitch bool `yaml:"auto_switch,omitempty" json:"auto_switch,omitempty"`
}

// UserPrefsQuickCmds はクイックコマンドの設定。
type UserPrefsQuickCmds struct {
	Cmd1 string `yaml:"cmd1,omitempty" json:"cmd1,omitempty"`
	Cmd2 string `yaml:"cmd2,omitempty" json:"cmd2,omitempty"`
}

// UserPrefsUsageLinks は使用量リンクの設定。
type UserPrefsUsageLinks struct {
	Claude      string `yaml:"claude,omitempty"  json:"claude,omitempty"`
	Codex       string `yaml:"codex,omitempty"   json:"codex,omitempty"`
	Copilot     string `yaml:"copilot,omitempty" json:"copilot,omitempty"`
	CursorAgent string `yaml:"cursor-agent,omitempty" json:"cursor-agent,omitempty"`
}

// UserPrefsVoice は音声入力の設定。
type UserPrefsVoice struct {
	GraceSeconds    int    `yaml:"grace_seconds,omitempty"    json:"grace_seconds,omitempty"`
	WakeWordEnabled bool   `yaml:"wake_word_enabled,omitempty" json:"wake_word_enabled,omitempty"`
	WakeWordPhrase  string `yaml:"wake_word_phrase,omitempty"  json:"wake_word_phrase,omitempty"`
}

// UserPrefsSpawn はセッション起動のデフォルト設定。
type UserPrefsSpawn struct {
	Defaults  map[string]string `yaml:"defaults,omitempty"   json:"defaults,omitempty"`
	LastModel map[string]string `yaml:"last_model,omitempty" json:"last_model,omitempty"`
}

// UserPrefsDisplay は表示まわりのユーザー設定（端末横断で共有する分）。
// theme / font_size / lang は従来 localStorage のみだったがサーバへ移行した。
// locked_mode はクライアントが従来から送っていたが収容先が無く取りこぼしていた分。
type UserPrefsDisplay struct {
	Theme      string `yaml:"theme,omitempty"       json:"theme,omitempty"`
	FontSize   string `yaml:"font_size,omitempty"   json:"font_size,omitempty"`
	Lang       string `yaml:"lang,omitempty"        json:"lang,omitempty"`
	LockedMode string `yaml:"locked_mode,omitempty" json:"locked_mode,omitempty"`
}

// UserPrefs はサーバ側（config.yaml: user_prefs:）に保存するユーザー機能設定。
// 端末・ポート横断で共有する D2 分類の設定を全て保持する。
type UserPrefs struct {
	Trigger                  UserPrefsTrigger     `yaml:"trigger,omitempty"      json:"trigger,omitempty"`
	NotifySound              UserPrefsNotifySound `yaml:"notify_sound,omitempty" json:"notify_sound,omitempty"`
	Approval                 UserPrefsApproval    `yaml:"approval,omitempty"     json:"approval,omitempty"`
	QuickCmds                UserPrefsQuickCmds   `yaml:"quick_cmds,omitempty"   json:"quick_cmds,omitempty"`
	UsageLinks               UserPrefsUsageLinks  `yaml:"usage_links,omitempty"  json:"usage_links,omitempty"`
	Voice                    UserPrefsVoice       `yaml:"voice,omitempty"        json:"voice,omitempty"`
	Favorites                []string             `yaml:"favorites,omitempty"        json:"favorites,omitempty"`
	SessionOrder             []string             `yaml:"session_order,omitempty"    json:"session_order,omitempty"`
	GroupOrder               []string             `yaml:"group_order,omitempty"      json:"group_order,omitempty"`
	ProjectFavorites         []string             `yaml:"project_favorites,omitempty" json:"project_favorites,omitempty"`
	CwdHistory               []string             `yaml:"cwd_history,omitempty"      json:"cwd_history,omitempty"`
	Spawn                    UserPrefsSpawn       `yaml:"spawn,omitempty"            json:"spawn,omitempty"`
	Display                  UserPrefsDisplay     `yaml:"display,omitempty"          json:"display,omitempty"`
	MigratedFromLocalstorage bool                 `yaml:"migrated_from_localstorage,omitempty" json:"migrated_from_localstorage,omitempty"`
	Avatar                   string               `yaml:"avatar,omitempty"       json:"avatar,omitempty"`
	DisplayName              string               `yaml:"display_name,omitempty" json:"display_name,omitempty"`
}

// Clone returns a deep copy of p. It copies slice and map fields so callers can
// marshal or mutate the result without racing with the live server config.
func (p UserPrefs) Clone() UserPrefs {
	c := p
	c.Favorites = cloneStringSlice(p.Favorites)
	c.SessionOrder = cloneStringSlice(p.SessionOrder)
	c.GroupOrder = cloneStringSlice(p.GroupOrder)
	c.ProjectFavorites = cloneStringSlice(p.ProjectFavorites)
	c.CwdHistory = cloneStringSlice(p.CwdHistory)
	c.Spawn.Defaults = cloneStringMap(p.Spawn.Defaults)
	c.Spawn.LastModel = cloneStringMap(p.Spawn.LastModel)
	return c
}

// LocalModel は config.yaml の local_models セクションに手書きで追記される
// ローカル LLM の 1 件。Ollama daemon `/api/tags` で取得した一覧と merge して
// /api/models の "Ollama Local" グループに表示する。
type LocalModel struct {
	ID    string `yaml:"id"             json:"id"`
	Label string `yaml:"label,omitempty" json:"label,omitempty"`
}

type Config struct {
	Hub struct {
		Port                     int    `yaml:"port"`
		OpenBrowser              bool   `yaml:"open_browser"`
		AutoShutdown             bool   `yaml:"auto_shutdown"`
		LogDir                   string `yaml:"log_dir"`
		IdleTimeoutMin           int    `yaml:"idle_timeout_min"`
		WrapperReconnectGraceSec int    `yaml:"wrapper_reconnect_grace_sec"`
	} `yaml:"hub"`
	Log   LogConfig `yaml:"log"`
	Spawn struct {
		LastModel map[string]string `yaml:"last_model,omitempty" json:"last_model,omitempty"`
	} `yaml:"spawn,omitempty" json:"spawn,omitempty"`
	Approval        ApprovalConfig  `yaml:"approval,omitempty"`
	SlashCmdSources SlashCmdSources `yaml:"slash_cmd_sources,omitempty" json:"slash_cmd_sources,omitempty"`
	// ModelsSource は /api/models のモデル defaults 取得元 URL を上書きする。
	// 空なら DefaultModelsSource を使う（slash_cmd_sources 等と同じく config 上書き可能にし、
	// 他の remote source だけ config 化されていない非対称を解消する）。
	ModelsSource           string                 `yaml:"models_source,omitempty" json:"models_source,omitempty"`
	ApprovalPatternSources ApprovalPatternSources `yaml:"approval_pattern_sources,omitempty" json:"approval_pattern_sources,omitempty"`
	ApprovalProfiles       ApprovalProfiles       `yaml:"approval_profiles,omitempty"        json:"approval_profiles,omitempty"`
	FileOpenApp            string                 `yaml:"file_open_app,omitempty"`
	TerminalApp            string                 `yaml:"terminal_app,omitempty"`
	Token                  string                 `yaml:"token"`
	LocalModels            []LocalModel           `yaml:"local_models,omitempty" json:"local_models,omitempty"`
	UserPrefs              UserPrefs              `yaml:"user_prefs,omitempty" json:"user_prefs,omitempty"`
}

func LoadOrCreate() (*Config, error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return nil, fmt.Errorf("home dir: %w", err)
	}
	dir := filepath.Join(home, ".any-ai-cli")
	if err := ensurePrivateDir(dir); err != nil {
		return nil, err
	}
	path := filepath.Join(dir, "config.yaml")
	cfg := defaultConfig(home)

	b, readErr := os.ReadFile(path)
	switch {
	case readErr == nil:
		if err := yaml.Unmarshal(b, cfg); err != nil {
			// 破損: .bak へ退避してデフォルト設定で起動継続
			bak := path + ".bak"
			if bakErr := os.WriteFile(bak, b, 0o600); bakErr != nil {
				return nil, fmt.Errorf("backup invalid config: %w", bakErr)
			}
			slog.Warn("config parse failed; backed up and regenerating",
				"bak", bak, "err", err)
			cfg = defaultConfig(home)
			if err := ensureToken(cfg); err != nil {
				return nil, err
			}
			out, mErr := yaml.Marshal(cfg)
			if mErr != nil {
				return nil, fmt.Errorf("marshal default config: %w", mErr)
			}
			if err := os.WriteFile(path, out, 0o600); err != nil {
				return nil, fmt.Errorf("write default config: %w", err)
			}
		}
	case os.IsNotExist(readErr):
		if err := ensureToken(cfg); err != nil {
			return nil, err
		}
		out, mErr := yaml.Marshal(cfg)
		if mErr != nil {
			return nil, fmt.Errorf("marshal default config: %w", mErr)
		}
		if err := os.WriteFile(path, out, 0o600); err != nil {
			return nil, fmt.Errorf("write default config: %w", err)
		}
	default:
		// 権限・I/O エラーは隠さず返す（既存設定を握り潰さない）
		return nil, fmt.Errorf("read config: %w", readErr)
	}
	if cfg.Token == "" {
		if err := ensureToken(cfg); err != nil {
			return nil, err
		}
	}
	// 旧 cfg.Spawn.LastModel → UserPrefs.Spawn.LastModel へ移行
	// 読み込み時に旧位置に値があり新位置が空なら移送し、旧位置を空にする
	if cfg.Spawn.LastModel != nil {
		if cfg.UserPrefs.Spawn.LastModel == nil {
			cfg.UserPrefs.Spawn.LastModel = map[string]string{}
		}
		for k, v := range cfg.Spawn.LastModel {
			if v != "" && cfg.UserPrefs.Spawn.LastModel[k] == "" {
				cfg.UserPrefs.Spawn.LastModel[k] = v
			}
		}
		cfg.Spawn.LastModel = nil
	}
	if cfg.UserPrefs.Spawn.LastModel == nil {
		cfg.UserPrefs.Spawn.LastModel = map[string]string{}
	}
	cfg.SlashCmdSources = EffectiveSlashCmdSources(cfg.SlashCmdSources)
	cfg.ApprovalPatternSources = EffectiveApprovalPatternSources(cfg.ApprovalPatternSources)
	cfg.ApprovalProfiles = EffectiveApprovalProfiles(cfg.ApprovalProfiles)
	return cfg, nil
}

func defaultConfig(home string) *Config {
	cfg := &Config{}
	cfg.Hub.Port = 47777
	cfg.Hub.OpenBrowser = true
	cfg.Hub.AutoShutdown = true
	// When invoked via the any-ai-cli-wsl.exe Windows launcher (and only then —
	// not for plain `any-ai-cli serve` inside a WSL shell), place logs under the
	// Windows %USERPROFILE% so the Hub UI's open-folder button resolves to a
	// plain C:\Users\... path that Windows Explorer can open directly. A bare
	// WSL session is treated as pure-Linux and keeps logs under Linux $HOME.
	logHome := home
	if wslutil.IsWindowsLauncherMode() {
		if winHome := wslutil.WindowsHomeAsUnix(); winHome != "" {
			logHome = winHome
		}
	}
	cfg.Hub.LogDir = filepath.Join(logHome, ".any-ai-cli", "logs")
	cfg.Hub.IdleTimeoutMin = 60
	cfg.Hub.WrapperReconnectGraceSec = 3600
	cfg.Log.Enabled = true
	cfg.Log.MaxSizeMB = 10
	cfg.Log.MaxBackups = 3
	cfg.Log.Compress = false
	cfg.Log.SessionRetentionDays = 7
	cfg.Log.SessionMaxSizeMB = 50
	cfg.UserPrefs = UserPrefs{}
	cfg.SlashCmdSources = DefaultSlashCmdSources()
	cfg.ApprovalPatternSources = DefaultApprovalPatternSources()
	cfg.ApprovalProfiles = DefaultApprovalProfiles()
	return cfg
}

func Save(cfg *Config) error {
	dir, err := Dir()
	if err != nil {
		return err
	}
	if err := ensurePrivateDir(dir); err != nil {
		return err
	}
	path := filepath.Join(dir, "config.yaml")
	out, err := yaml.Marshal(cfg)
	if err != nil {
		return fmt.Errorf("marshal config: %w", err)
	}
	// atomic write: temp ファイルを同一ディレクトリに作成して Rename で差し替える。
	// 同一ボリューム内の Rename はほぼ atomic であり、書き込み中断でも既存 config.yaml を破損しない。
	tmp, err := os.CreateTemp(dir, "config-*.yaml.tmp")
	if err != nil {
		return fmt.Errorf("create temp config: %w", err)
	}
	tmpName := tmp.Name()
	defer os.Remove(tmpName) // Rename 成功後は no-op
	if _, err := tmp.Write(out); err != nil {
		tmp.Close()
		return fmt.Errorf("write temp config: %w", err)
	}
	if err := tmp.Sync(); err != nil {
		tmp.Close()
		return fmt.Errorf("sync temp config: %w", err)
	}
	if err := tmp.Close(); err != nil {
		return fmt.Errorf("close temp config: %w", err)
	}
	if err := os.Chmod(tmpName, 0o600); err != nil {
		return fmt.Errorf("chmod temp config: %w", err)
	}
	return os.Rename(tmpName, path)
}

// Clone returns a deep copy of cfg safe to pass to Save without holding s.mu.
// map and slice fields are individually copied to prevent concurrent map
// iteration/write panics during yaml.Marshal.
func (cfg *Config) Clone() *Config {
	c := *cfg // shallow copy of all scalar/struct fields
	c.UserPrefs = cfg.UserPrefs.Clone()

	// Deep-copy map fields
	if cfg.Spawn.LastModel != nil {
		c.Spawn.LastModel = cloneStringMap(cfg.Spawn.LastModel)
	}

	// Deep-copy slice fields
	if cfg.LocalModels != nil {
		s := make([]LocalModel, len(cfg.LocalModels))
		copy(s, cfg.LocalModels)
		c.LocalModels = s
	}
	return &c
}

func cloneStringSlice(in []string) []string {
	if in == nil {
		return nil
	}
	out := make([]string, len(in))
	copy(out, in)
	return out
}

func cloneStringMap(in map[string]string) map[string]string {
	if in == nil {
		return nil
	}
	out := make(map[string]string, len(in))
	for k, v := range in {
		out[k] = v
	}
	return out
}

func ensurePrivateDir(dir string) error {
	if err := os.MkdirAll(dir, DirMode); err != nil {
		return fmt.Errorf("mkdir config dir: %w", err)
	}
	if err := os.Chmod(dir, DirMode); err != nil {
		return fmt.Errorf("chmod config dir: %w", err)
	}
	return nil
}

func ensureToken(cfg *Config) error {
	token, err := randomToken()
	if err != nil {
		return err
	}
	cfg.Token = token
	return nil
}

func randomToken() (string, error) {
	buf := make([]byte, 32)
	if _, err := rand.Read(buf); err != nil {
		return "", fmt.Errorf("generate token: %w", err)
	}
	return hex.EncodeToString(buf), nil
}
