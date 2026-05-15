package config

import (
	"crypto/rand"
	"encoding/hex"
	"fmt"
	"os"
	"path/filepath"

	"gopkg.in/yaml.v3"
)

// LogConfig はファイルローテーションロギングの設定。
type LogConfig struct {
	Enabled    bool `yaml:"enabled" json:"enabled"`
	MaxSizeMB  int  `yaml:"max_size_mb" json:"max_size_mb"`
	MaxBackups int  `yaml:"max_backups" json:"max_backups"`
	Compress   bool `yaml:"compress" json:"compress"`
}

// ApprovalConfig は Hub 承認ボタン機能の設定。
type ApprovalConfig struct {
	Enabled          bool `yaml:"enabled"`
	FirstLaunchShown bool `yaml:"first_launch_shown"`
}

// SlashCmdSources は provider ごとのスラッシュコマンド取得元。
// URL またはローカルの .md/.txt パスを指定できる。
type SlashCmdSources struct {
	Claude string `yaml:"claude" json:"claude"`
	Codex  string `yaml:"codex"  json:"codex"`
}

const (
	LegacyClaudeSlashCmdSource  = "https://code.claude.com/docs/en/commands.md"
	DefaultClaudeSlashCmdSource = "https://raw.githubusercontent.com/ishizakahiroshi/any-ai-cli/main/resources/slash-commands/claude.md"
	DefaultCodexSlashCmdSource  = "https://raw.githubusercontent.com/ishizakahiroshi/any-ai-cli/main/resources/slash-commands/codex.md"
)

func DefaultSlashCmdSources() SlashCmdSources {
	return SlashCmdSources{
		Claude: DefaultClaudeSlashCmdSource,
		Codex:  DefaultCodexSlashCmdSource,
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
	return src
}

// ApprovalPatternSources は provider ごとのリモート md 取得元。
// SlashCmdSources と同じ構造で、空文字なら Default*ApprovalPatternSource を使う。
type ApprovalPatternSources struct {
	Claude string `yaml:"claude,omitempty" json:"claude,omitempty"`
	Codex  string `yaml:"codex,omitempty"  json:"codex,omitempty"`
	Common string `yaml:"common,omitempty" json:"common,omitempty"`
}

const (
	DefaultClaudeApprovalPatternSource = "https://raw.githubusercontent.com/ishizakahiroshi/any-ai-cli/main/resources/approval-patterns/claude.md"
	DefaultCodexApprovalPatternSource  = "https://raw.githubusercontent.com/ishizakahiroshi/any-ai-cli/main/resources/approval-patterns/codex.md"
	DefaultCommonApprovalPatternSource = "https://raw.githubusercontent.com/ishizakahiroshi/any-ai-cli/main/resources/approval-patterns/common.md"
)

func DefaultApprovalPatternSources() ApprovalPatternSources {
	return ApprovalPatternSources{
		Claude: DefaultClaudeApprovalPatternSource,
		Codex:  DefaultCodexApprovalPatternSource,
		Common: DefaultCommonApprovalPatternSource,
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
	Claude ApprovalProfileName `yaml:"claude,omitempty" json:"claude,omitempty"`
	Codex  ApprovalProfileName `yaml:"codex,omitempty"  json:"codex,omitempty"`
	Common ApprovalProfileName `yaml:"common,omitempty" json:"common,omitempty"`
}

// DefaultApprovalProfiles は新規ユーザー向けデフォルト（全 provider official）。
func DefaultApprovalProfiles() ApprovalProfiles {
	return ApprovalProfiles{
		Claude: ApprovalProfileOfficial,
		Codex:  ApprovalProfileOfficial,
		Common: ApprovalProfileOfficial,
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
	case "common":
		p.Common = name
	}
	return p
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
	Approval               ApprovalConfig         `yaml:"approval,omitempty"`
	SlashCmdSources        SlashCmdSources        `yaml:"slash_cmd_sources,omitempty" json:"slash_cmd_sources,omitempty"`
	ApprovalPatternSources ApprovalPatternSources `yaml:"approval_pattern_sources,omitempty" json:"approval_pattern_sources,omitempty"`
	ApprovalProfiles       ApprovalProfiles       `yaml:"approval_profiles,omitempty"        json:"approval_profiles,omitempty"`
	FileOpenApp            string                 `yaml:"file_open_app,omitempty"`
	TerminalApp            string                 `yaml:"terminal_app,omitempty"`
	Token                  string                 `yaml:"token"`
}

func LoadOrCreate() (*Config, error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return nil, fmt.Errorf("home dir: %w", err)
	}
	dir := filepath.Join(home, ".any-ai-cli")
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return nil, err
	}
	path := filepath.Join(dir, "config.yaml")
	cfg := defaultConfig(home)
	if b, err := os.ReadFile(path); err == nil {
		if err := yaml.Unmarshal(b, cfg); err != nil {
			return nil, err
		}
	} else {
		out, _ := yaml.Marshal(cfg)
		if err := os.WriteFile(path, out, 0o600); err != nil {
			return nil, err
		}
	}
	if cfg.Token == "" {
		cfg.Token = randomToken()
	}
	if cfg.Spawn.LastModel == nil {
		cfg.Spawn.LastModel = map[string]string{}
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
	cfg.Hub.LogDir = filepath.Join(home, ".any-ai-cli", "logs")
	cfg.Hub.IdleTimeoutMin = 60
	cfg.Hub.WrapperReconnectGraceSec = 3600
	cfg.Log.Enabled = true
	cfg.Log.MaxSizeMB = 10
	cfg.Log.MaxBackups = 3
	cfg.Log.Compress = false
	cfg.Spawn.LastModel = map[string]string{}
	cfg.SlashCmdSources = DefaultSlashCmdSources()
	cfg.ApprovalPatternSources = DefaultApprovalPatternSources()
	cfg.ApprovalProfiles = DefaultApprovalProfiles()
	cfg.Token = randomToken()
	return cfg
}

func Save(cfg *Config) error {
	home, err := os.UserHomeDir()
	if err != nil {
		return fmt.Errorf("home dir: %w", err)
	}
	path := filepath.Join(home, ".any-ai-cli", "config.yaml")
	out, err := yaml.Marshal(cfg)
	if err != nil {
		return err
	}
	return os.WriteFile(path, out, 0o600)
}

func randomToken() string {
	buf := make([]byte, 32)
	_, _ = rand.Read(buf)
	return hex.EncodeToString(buf)
}
