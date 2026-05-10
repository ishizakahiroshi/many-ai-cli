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
	DefaultClaudeSlashCmdSource = "https://raw.githubusercontent.com/ishizakahiroshi/ai-cli-hub/main/resources/slash-commands/claude.md"
	DefaultCodexSlashCmdSource  = "https://raw.githubusercontent.com/ishizakahiroshi/ai-cli-hub/main/resources/slash-commands/codex.md"
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
	Token           string          `yaml:"token"`
}

func LoadOrCreate() (*Config, error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return nil, fmt.Errorf("home dir: %w", err)
	}
	dir := filepath.Join(home, ".ai-cli-hub")
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
	return cfg, nil
}

func defaultConfig(home string) *Config {
	cfg := &Config{}
	cfg.Hub.Port = 47777
	cfg.Hub.AutoShutdown = true
	cfg.Hub.LogDir = filepath.Join(home, ".ai-cli-hub", "logs")
	cfg.Hub.IdleTimeoutMin = 60
	cfg.Hub.WrapperReconnectGraceSec = 3600
	cfg.Log.Enabled = true
	cfg.Log.MaxSizeMB = 10
	cfg.Log.MaxBackups = 3
	cfg.Log.Compress = false
	cfg.Spawn.LastModel = map[string]string{}
	cfg.SlashCmdSources = DefaultSlashCmdSources()
	cfg.Token = randomToken()
	return cfg
}

func Save(cfg *Config) error {
	home, err := os.UserHomeDir()
	if err != nil {
		return fmt.Errorf("home dir: %w", err)
	}
	path := filepath.Join(home, ".ai-cli-hub", "config.yaml")
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
