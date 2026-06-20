package config

import (
	"crypto/rand"
	"encoding/hex"
	"fmt"
	"log/slog"
	"net"
	neturl "net/url"
	"os"
	"path/filepath"
	"strings"

	"gopkg.in/yaml.v3"

	"many-ai-cli/internal/wslutil"
)

const DirMode os.FileMode = 0o700

// Dir は ~/.many-ai-cli ディレクトリのパスを返す。
func Dir() (string, error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return "", fmt.Errorf("home dir: %w", err)
	}
	return filepath.Join(home, ".many-ai-cli"), nil
}

// Path は ~/.many-ai-cli/config.yaml のパスを返す。
func Path() (string, error) {
	dir, err := Dir()
	if err != nil {
		return "", err
	}
	return filepath.Join(dir, "config.yaml"), nil
}

// LogConfig はファイルローテーションロギングの設定。
type LogConfig struct {
	Enabled bool `yaml:"enabled" json:"enabled"`
	// SessionEnabled はセッションログ（.log/.jsonl/.txt + SQLite のイベント本文）の
	// 記録を有効にするかどうか。既定 false（オプトイン）。生 PTY ログ（.log）には
	// API キー・トークン・パスワード等がマスクされずに残り得るため、リスクを理解した
	// 利用者だけが有効化する。Enabled（hub.log の診断ログ）とは別物。
	SessionEnabled bool `yaml:"session_enabled" json:"session_enabled"`
	// LegacyLogsNoticeShown は「旧バージョンで既定 ON だった頃のセッションログが
	// 残っているので削除を推奨する」一回限りの通知を既に出したかどうか。
	// 旧バージョンからの設定ファイルにはこのキーが無く false 起点になるため、
	// 旧ログが残っている利用者にだけ初回起動時に通知が出る。
	LegacyLogsNoticeShown bool `yaml:"legacy_logs_notice_shown" json:"legacy_logs_notice_shown"`
	MaxSizeMB             int  `yaml:"max_size_mb" json:"max_size_mb"`
	MaxBackups            int  `yaml:"max_backups" json:"max_backups"`
	Compress              bool `yaml:"compress" json:"compress"`
	SessionRetentionDays  int  `yaml:"session_retention_days" json:"session_retention_days"`
	SessionMaxSizeMB      int  `yaml:"session_max_size_mb" json:"session_max_size_mb"`
	// AttachmentRetentionDays は添付ファイル（~/.many-ai-cli/attachments）の保持日数。
	// この日数より古い添付を定期削除する。0 で日数ベースの削除を無効化。
	AttachmentRetentionDays int `yaml:"attachment_retention_days" json:"attachment_retention_days"`
	// AttachmentMaxTotalMB は添付ファイル全体の合計容量上限(MB)。これを超えた分を
	// 古い mtime のファイルから削除して上限内に収める。0 で容量ベースの削除を無効化。
	AttachmentMaxTotalMB int `yaml:"attachment_max_total_mb" json:"attachment_max_total_mb"`
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
	Opencode    string `yaml:"opencode" json:"opencode"`
	Grok        string `yaml:"grok" json:"grok"`
}

const (
	LegacyClaudeSlashCmdSource       = "https://code.claude.com/docs/en/commands.md"
	DefaultClaudeSlashCmdSource      = "https://raw.githubusercontent.com/ishizakahiroshi/many-ai-cli/main/resources/slash-commands/claude.md"
	DefaultCodexSlashCmdSource       = "https://raw.githubusercontent.com/ishizakahiroshi/many-ai-cli/main/resources/slash-commands/codex.md"
	DefaultCopilotSlashCmdSource     = "https://raw.githubusercontent.com/ishizakahiroshi/many-ai-cli/main/resources/slash-commands/copilot.md"
	DefaultCursorAgentSlashCmdSource = "https://raw.githubusercontent.com/ishizakahiroshi/many-ai-cli/main/resources/slash-commands/cursor-agent.md"
	DefaultOpenCodeSlashCmdSource    = "https://raw.githubusercontent.com/ishizakahiroshi/many-ai-cli/main/resources/slash-commands/opencode.md"
	DefaultGrokSlashCmdSource        = "https://raw.githubusercontent.com/ishizakahiroshi/many-ai-cli/main/resources/slash-commands/grok.md"
)

const DefaultUsageLinkSource = "https://raw.githubusercontent.com/ishizakahiroshi/many-ai-cli/main/resources/usage-links/defaults.json"

const DefaultModelsSource = "https://raw.githubusercontent.com/ishizakahiroshi/many-ai-cli/main/resources/models/defaults.json"

const DefaultOllamaBaseURL = "http://localhost:11434"

const DefaultLMStudioBaseURL = "http://localhost:1234"

func DefaultSlashCmdSources() SlashCmdSources {
	return SlashCmdSources{
		Claude:      DefaultClaudeSlashCmdSource,
		Codex:       DefaultCodexSlashCmdSource,
		Copilot:     DefaultCopilotSlashCmdSource,
		CursorAgent: DefaultCursorAgentSlashCmdSource,
		Opencode:    DefaultOpenCodeSlashCmdSource,
		Grok:        DefaultGrokSlashCmdSource,
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
	if src.Opencode == "" {
		src.Opencode = defaults.Opencode
	}
	if src.Grok == "" {
		src.Grok = defaults.Grok
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
	Opencode    string `yaml:"opencode,omitempty" json:"opencode,omitempty"`
	Grok        string `yaml:"grok,omitempty" json:"grok,omitempty"`
	Common      string `yaml:"common,omitempty"  json:"common,omitempty"`
}

const (
	DefaultClaudeApprovalPatternSource      = "https://raw.githubusercontent.com/ishizakahiroshi/many-ai-cli/main/resources/approval-patterns/claude.md"
	DefaultCodexApprovalPatternSource       = "https://raw.githubusercontent.com/ishizakahiroshi/many-ai-cli/main/resources/approval-patterns/codex.md"
	DefaultCopilotApprovalPatternSource     = "https://raw.githubusercontent.com/ishizakahiroshi/many-ai-cli/main/resources/approval-patterns/copilot.md"
	DefaultCursorAgentApprovalPatternSource = "https://raw.githubusercontent.com/ishizakahiroshi/many-ai-cli/main/resources/approval-patterns/cursor-agent.md"
	DefaultCommonApprovalPatternSource      = "https://raw.githubusercontent.com/ishizakahiroshi/many-ai-cli/main/resources/approval-patterns/common.md"
	DefaultOpenCodeApprovalPatternSource    = "https://raw.githubusercontent.com/ishizakahiroshi/many-ai-cli/main/resources/approval-patterns/opencode.md"
	DefaultGrokApprovalPatternSource        = "https://raw.githubusercontent.com/ishizakahiroshi/many-ai-cli/main/resources/approval-patterns/grok.md"
)

func DefaultApprovalPatternSources() ApprovalPatternSources {
	return ApprovalPatternSources{
		Claude:      DefaultClaudeApprovalPatternSource,
		Codex:       DefaultCodexApprovalPatternSource,
		Copilot:     DefaultCopilotApprovalPatternSource,
		CursorAgent: DefaultCursorAgentApprovalPatternSource,
		Opencode:    DefaultOpenCodeApprovalPatternSource,
		Grok:        DefaultGrokApprovalPatternSource,
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
	if src.Opencode == "" {
		src.Opencode = defaults.Opencode
	}
	if src.Grok == "" {
		src.Grok = defaults.Grok
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
	Opencode    ApprovalProfileName `yaml:"opencode,omitempty" json:"opencode,omitempty"`
	Grok        ApprovalProfileName `yaml:"grok,omitempty" json:"grok,omitempty"`
	Common      ApprovalProfileName `yaml:"common,omitempty"  json:"common,omitempty"`
}

// DefaultApprovalProfiles は新規ユーザー向けデフォルト（全 provider official）。
func DefaultApprovalProfiles() ApprovalProfiles {
	return ApprovalProfiles{
		Claude:      ApprovalProfileOfficial,
		Codex:       ApprovalProfileOfficial,
		Copilot:     ApprovalProfileOfficial,
		CursorAgent: ApprovalProfileOfficial,
		Opencode:    ApprovalProfileOfficial,
		Grok:        ApprovalProfileOfficial,
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
	if p.Opencode == "" {
		p.Opencode = ApprovalProfileOfficial
	}
	if p.Grok == "" {
		p.Grok = ApprovalProfileOfficial
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
	case "opencode":
		if p.Opencode != "" {
			return p.Opencode
		}
	case "grok":
		if p.Grok != "" {
			return p.Grok
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
	case "opencode":
		p.Opencode = name
	case "grok":
		p.Grok = name
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

// UserPrefsDesktopNotifications はページ内 Notification API の設定。
type UserPrefsDesktopNotifications struct {
	Enabled bool `yaml:"enabled,omitempty" json:"enabled,omitempty"`
}

// UserPrefsPushNotifications は Service Worker / Web Push の設定。
type UserPrefsPushNotifications struct {
	Enabled bool `yaml:"enabled,omitempty" json:"enabled,omitempty"`
}

// UserPrefsQuickCmds はクイックコマンドの設定（スロット 1〜5）。
// ShowN はボタンの表示/非表示。既定は表示のため、未設定(nil)は表示として扱う。
// false を omitempty で落とさないようポインタにする（非表示設定の永続化のため）。
type UserPrefsQuickCmds struct {
	Cmd1  string `yaml:"cmd1,omitempty"  json:"cmd1,omitempty"`
	Cmd2  string `yaml:"cmd2,omitempty"  json:"cmd2,omitempty"`
	Cmd3  string `yaml:"cmd3,omitempty"  json:"cmd3,omitempty"`
	Cmd4  string `yaml:"cmd4,omitempty"  json:"cmd4,omitempty"`
	Cmd5  string `yaml:"cmd5,omitempty"  json:"cmd5,omitempty"`
	Show1 *bool  `yaml:"show1,omitempty" json:"show1,omitempty"`
	Show2 *bool  `yaml:"show2,omitempty" json:"show2,omitempty"`
	Show3 *bool  `yaml:"show3,omitempty" json:"show3,omitempty"`
	Show4 *bool  `yaml:"show4,omitempty" json:"show4,omitempty"`
	Show5 *bool  `yaml:"show5,omitempty" json:"show5,omitempty"`
}

// UserPrefsUsageLinks は使用量リンクの設定。
type UserPrefsUsageLinks struct {
	Claude      string `yaml:"claude,omitempty"  json:"claude,omitempty"`
	Codex       string `yaml:"codex,omitempty"   json:"codex,omitempty"`
	Copilot     string `yaml:"copilot,omitempty" json:"copilot,omitempty"`
	CursorAgent string `yaml:"cursor-agent,omitempty" json:"cursor-agent,omitempty"`
	Opencode    string `yaml:"opencode,omitempty" json:"opencode,omitempty"`
	Grok        string `yaml:"grok,omitempty" json:"grok,omitempty"`
}

// UserPrefsVoice は音声入力の設定。
type UserPrefsVoice struct {
	GraceSeconds    int    `yaml:"grace_seconds,omitempty"    json:"grace_seconds,omitempty"`
	WakeWordEnabled bool   `yaml:"wake_word_enabled,omitempty" json:"wake_word_enabled,omitempty"`
	WakeWordPhrase  string `yaml:"wake_word_phrase,omitempty"  json:"wake_word_phrase,omitempty"`
	// InputDisabled は音声入力（🎤 ボタン・Alt+V）を無効化する。
	// 既定 false（有効）。default-true を omitempty で扱えないため否定形フィールドにしている。
	InputDisabled bool `yaml:"input_disabled,omitempty" json:"input_disabled,omitempty"`
}

// UserPrefsSpawn はセッション起動のデフォルト設定。
type UserPrefsSpawn struct {
	Defaults  map[string]string `yaml:"defaults,omitempty"   json:"defaults,omitempty"`
	LastModel map[string]string `yaml:"last_model,omitempty" json:"last_model,omitempty"`
}

// UserPrefsDoneSummaryNotify はタスク完了サマリー通知の設定。
type UserPrefsDoneSummaryNotify struct {
	Enabled bool `yaml:"enabled,omitempty" json:"enabled,omitempty"`
}

// UserPrefsTokenStatusbar はトークンコスト常時表示バーの設定。
// Enabled が nil（未設定）または true のときに表示する（既定 ON）。
// bool のゼロ値が false なため *bool ポインタで三値（nil=未設定/true/false）を表現する。
type UserPrefsTokenStatusbar struct {
	Enabled *bool `yaml:"enabled,omitempty" json:"enabled,omitempty"`
	// Segments: セグメント名(short, 例 "ctx"/"ratelimit"/"ailines") → 表示するか。
	// キー未設定 = デフォルト表示。明示的に false のセグメントだけ UI 側で非表示にする。
	Segments map[string]bool `yaml:"segments,omitempty" json:"segments,omitempty"`
}

// IsEnabled は Enabled が nil（未設定）または true のとき true を返す（既定 ON）。
func (u UserPrefsTokenStatusbar) IsEnabled() bool {
	return u.Enabled == nil || *u.Enabled
}

// UserPrefsDisplay は表示まわりのユーザー設定（端末横断で共有する分）。
// theme / font_size / lang は従来 localStorage のみだったがサーバへ移行した。
// locked_mode はクライアントが従来から送っていたが収容先が無く取りこぼしていた分。
type UserPrefsDisplay struct {
	Theme      string `yaml:"theme,omitempty"       json:"theme,omitempty"`
	FontSize   string `yaml:"font_size,omitempty"   json:"font_size,omitempty"`
	Lang       string `yaml:"lang,omitempty"        json:"lang,omitempty"`
	LockedMode string `yaml:"locked_mode,omitempty" json:"locked_mode,omitempty"`
	// ライブステータス帯（「実行中」バー）のユーザー配色（hex 文字列）。空＝既定。
	LiveStatusBg string `yaml:"live_status_bg,omitempty" json:"live_status_bg,omitempty"`
	LiveStatusFg string `yaml:"live_status_fg,omitempty" json:"live_status_fg,omitempty"`
}

type VoiceWhisperConfig struct {
	ServerURL      string `yaml:"server_url,omitempty" json:"server_url,omitempty"`
	RequestPath    string `yaml:"request_path,omitempty" json:"request_path,omitempty"`
	Language       string `yaml:"language,omitempty" json:"language,omitempty"`
	TimeoutSeconds int    `yaml:"timeout_seconds,omitempty" json:"timeout_seconds,omitempty"`
	Managed        bool   `yaml:"managed,omitempty" json:"managed,omitempty"`
	Model          string `yaml:"model,omitempty" json:"model,omitempty"`
	ServerPort     int    `yaml:"server_port,omitempty" json:"server_port,omitempty"`
	// HallucinationPhrases は認識結果がこれらと（正規化後に）完全一致したら破棄する幻聴フィルタ。
	// 未設定（nil）なら既定リストを適用。空リスト（[]）を明示するとフィルタ無効。
	HallucinationPhrases []string `yaml:"hallucination_phrases,omitempty" json:"hallucination_phrases,omitempty"`
}

// DefaultWhisperHallucinationPhrases は Whisper が無音・微小音に対して出力しがちな
// 定型句（幻聴）の既定リスト。config.yaml の voice.whisper.hallucination_phrases で上書きできる。
var DefaultWhisperHallucinationPhrases = []string{
	"ご視聴ありがとうございました",
	"ご清聴ありがとうございました",
	"チャンネル登録をお願いします",
	"チャンネル登録よろしくお願いします",
	"字幕視聴ありがとうございました",
	"thanks for watching",
	"thank you for watching",
	"please subscribe",
	"like and subscribe",
	"don't forget to subscribe",
}

type VoiceConfig struct {
	Whisper VoiceWhisperConfig `yaml:"whisper,omitempty" json:"whisper,omitempty"`
}

// UserPrefs はサーバ側（config.yaml: user_prefs:）に保存するユーザー機能設定。
// 端末・ポート横断で共有する D2 分類の設定を全て保持する。
type UserPrefs struct {
	Trigger                  UserPrefsTrigger              `yaml:"trigger,omitempty"      json:"trigger,omitempty"`
	NotifySound              UserPrefsNotifySound          `yaml:"notify_sound,omitempty" json:"notify_sound,omitempty"`
	DesktopNotifications     UserPrefsDesktopNotifications `yaml:"desktop_notifications,omitempty" json:"desktop_notifications,omitempty"`
	PushNotifications        UserPrefsPushNotifications    `yaml:"push_notifications,omitempty" json:"push_notifications,omitempty"`
	Approval                 UserPrefsApproval             `yaml:"approval,omitempty"     json:"approval,omitempty"`
	QuickCmds                UserPrefsQuickCmds            `yaml:"quick_cmds,omitempty"   json:"quick_cmds,omitempty"`
	UsageLinks               UserPrefsUsageLinks           `yaml:"usage_links,omitempty"  json:"usage_links,omitempty"`
	Voice                    UserPrefsVoice                `yaml:"voice,omitempty"        json:"voice,omitempty"`
	Favorites                []string                      `yaml:"favorites,omitempty"        json:"favorites,omitempty"`
	SessionOrder             []string                      `yaml:"session_order,omitempty"    json:"session_order,omitempty"`
	GroupOrder               []string                      `yaml:"group_order,omitempty"      json:"group_order,omitempty"`
	ProjectFavorites         []string                      `yaml:"project_favorites,omitempty" json:"project_favorites,omitempty"`
	CwdHistory               []string                      `yaml:"cwd_history,omitempty"      json:"cwd_history,omitempty"`
	CwdFavorites             []string                      `yaml:"cwd_favorites,omitempty"    json:"cwd_favorites,omitempty"`
	Spawn                    UserPrefsSpawn                `yaml:"spawn,omitempty"            json:"spawn,omitempty"`
	Display                  UserPrefsDisplay              `yaml:"display,omitempty"          json:"display,omitempty"`
	MigratedFromLocalstorage bool                          `yaml:"migrated_from_localstorage,omitempty" json:"migrated_from_localstorage,omitempty"`
	Avatar                   string                        `yaml:"avatar,omitempty"       json:"avatar,omitempty"`
	DisplayName              string                        `yaml:"display_name,omitempty" json:"display_name,omitempty"`
	TokenStatusbar           UserPrefsTokenStatusbar       `yaml:"token_statusbar,omitempty" json:"token_statusbar,omitempty"`
	DoneSummaryNotify        UserPrefsDoneSummaryNotify    `yaml:"done_summary_notify,omitempty" json:"done_summary_notify,omitempty"`
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
	c.CwdFavorites = cloneStringSlice(p.CwdFavorites)
	c.Spawn.Defaults = cloneStringMap(p.Spawn.Defaults)
	c.Spawn.LastModel = cloneStringMap(p.Spawn.LastModel)
	if p.TokenStatusbar.Segments != nil {
		m := make(map[string]bool, len(p.TokenStatusbar.Segments))
		for k, v := range p.TokenStatusbar.Segments {
			m[k] = v
		}
		c.TokenStatusbar.Segments = m
	}
	return c
}

// LocalModel は config.yaml の local_models セクションに手書きで追記される
// ローカル LLM の 1 件。Ollama daemon `/api/tags` で取得した一覧と merge して
// /api/models の "Ollama Local" グループに表示する。
type LocalModel struct {
	ID    string `yaml:"id"             json:"id"`
	Label string `yaml:"label,omitempty" json:"label,omitempty"`
}

// OllamaConfig は Ollama daemon への接続先設定。
// 空なら DefaultOllamaBaseURL を使う。
type OllamaConfig struct {
	BaseURL string `yaml:"base_url,omitempty" json:"base_url,omitempty"`
}

func EffectiveOllamaBaseURL(baseURL string) string {
	baseURL = strings.TrimRight(strings.TrimSpace(baseURL), "/")
	if baseURL == "" {
		return DefaultOllamaBaseURL
	}
	return baseURL
}

// LMStudioConfig は LM Studio ローカルサーバーへの接続先設定。
// 空なら DefaultLMStudioBaseURL を使う。base_url に /v1 や path は付けない。
type LMStudioConfig struct {
	BaseURL string `yaml:"base_url,omitempty" json:"base_url,omitempty"`
}

func EffectiveLMStudioBaseURL(baseURL string) string {
	baseURL = strings.TrimRight(strings.TrimSpace(baseURL), "/")
	if baseURL == "" {
		return DefaultLMStudioBaseURL
	}
	return baseURL
}

// NotifyBackendConfig は通知バックエンド 1 件の設定。
type NotifyBackendConfig struct {
	Type  string `yaml:"type"  json:"type"` // "ntfy" | "webhook"
	URL   string `yaml:"url"   json:"url"`
	Topic string `yaml:"topic,omitempty" json:"topic,omitempty"` // ntfy のみ有効
}

// NotifyConfig は config.yaml の notify: セクションに対応する。
// 未設定時は全イベントで通知しない（オプトイン）。
type NotifyConfig struct {
	Backends []NotifyBackendConfig `yaml:"backends,omitempty" json:"backends,omitempty"`
	Events   []string              `yaml:"events,omitempty"   json:"events,omitempty"`
}

type Config struct {
	Hub struct {
		Port         int  `yaml:"port"`
		OpenBrowser  bool `yaml:"open_browser"`
		AutoShutdown bool `yaml:"auto_shutdown"`
		// StaleBinaryAutoRestart: 既定 true。`many-ai-cli claude` 等の起動時に
		// 「ディスクの exe ≠ 稼働中 Hub のバイナリ」かつアクティブセッション 0 の
		// とき、古い Hub を自動で停止→再起動して新バイナリに載せ替える。
		// false にすると載せ替えず警告ログのみ（手動再起動に委ねる）。
		StaleBinaryAutoRestart    bool     `yaml:"stale_binary_auto_restart"`
		LogDir                    string   `yaml:"log_dir"`
		IdleTimeoutMin            int      `yaml:"idle_timeout_min"`
		WrapperReconnectGraceSec  int      `yaml:"wrapper_reconnect_grace_sec"`
		AllowLoopbackWithoutToken bool     `yaml:"allow_loopback_without_token,omitempty" json:"allow_loopback_without_token,omitempty"`
		TrustedNetworks           []string `yaml:"trusted_networks,omitempty" json:"trusted_networks,omitempty"`
		AllowedHosts              []string `yaml:"allowed_hosts,omitempty" json:"allowed_hosts,omitempty"`
		EnvKind                   string   `yaml:"env_kind,omitempty" json:"env_kind,omitempty"`
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
	TerminalApp            string                 `yaml:"terminal_app,omitempty"`
	Token                  string                 `yaml:"token"`
	// AuthCookieSecret は将来のリモート認証 cookie（PIN セッション）の HMAC 署名鍵。
	// 「全アクセス失効」で Token と共にローテーションして既存セッションを無効化する。
	// API レスポンスには出さない（json:"-"）。
	AuthCookieSecret string `yaml:"auth_cookie_secret,omitempty" json:"-"`
	// RemotePINHash は任意リモート PIN（既定 OFF）の bcrypt ハッシュ。空なら PIN 無効。
	// 非 loopback アクセス時のみ PIN ログインを要求する追加の扉（plan_hub-remote-auth.md / A）。
	// 平文 PIN は決して保存しない。API レスポンスにも出さない（json:"-"）。
	RemotePINHash string       `yaml:"remote_pin_hash,omitempty" json:"-"`
	Ollama        OllamaConfig   `yaml:"ollama,omitempty" json:"ollama,omitempty"`
	LMStudio      LMStudioConfig `yaml:"lm_studio,omitempty" json:"lm_studio,omitempty"`
	LocalModels   []LocalModel   `yaml:"local_models,omitempty" json:"local_models,omitempty"`
	UserPrefs     UserPrefs    `yaml:"user_prefs,omitempty" json:"user_prefs,omitempty"`
	Voice         VoiceConfig  `yaml:"voice,omitempty" json:"voice,omitempty"`
	Notify        NotifyConfig `yaml:"notify,omitempty" json:"notify,omitempty"`
}

func LoadOrCreate() (*Config, error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return nil, fmt.Errorf("home dir: %w", err)
	}
	migrateLegacyDir(home)
	dir := filepath.Join(home, ".many-ai-cli")
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
			if bakErr := writeConfigAtomic(dir, bak, b); bakErr != nil { // #nosec G703 -- ~/.many-ai-cli/config.yaml.bak 固定パス（ユーザー入力なし）
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
			if err := writeConfigAtomic(dir, path, out); err != nil {
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
		if err := writeConfigAtomic(dir, path, out); err != nil {
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
	cfg.applyDefaults()
	if err := cfg.Validate(); err != nil {
		return nil, err
	}
	return cfg, nil
}

// migrateLegacyDir は旧名 ~/.any-ai-cli を新名 ~/.many-ai-cli へ一度だけ移行する。
// any-ai-cli → many-ai-cli 改名（v0.3.0）の後方互換。新ディレクトリが既に存在する
// 場合は何もしない。コピーではなく os.Rename（移動）で stale ディレクトリを残さない。
// 失敗しても起動は継続する（新規ディレクトリ作成にフォールバックする）。
func migrateLegacyDir(home string) {
	newDir := filepath.Join(home, ".many-ai-cli")
	oldDir := filepath.Join(home, ".any-ai-cli")
	if _, err := os.Stat(newDir); err == nil {
		return // 新ディレクトリが既にある: 移行不要
	} else if !os.IsNotExist(err) {
		return // stat エラー（権限等）: 触らない
	}
	if info, err := os.Stat(oldDir); err != nil || !info.IsDir() {
		return // 旧ディレクトリが無い: 新規ユーザー
	}
	if err := os.Rename(oldDir, newDir); err != nil {
		slog.Warn("legacy config dir migration failed; starting fresh",
			"from", oldDir, "to", newDir, "err", err)
		return
	}
	slog.Info("migrated legacy config dir", "from", oldDir, "to", newDir)
}

func defaultConfig(home string) *Config {
	cfg := &Config{}
	cfg.Hub.Port = 47777
	cfg.Hub.OpenBrowser = true
	cfg.Hub.AutoShutdown = true
	cfg.Hub.StaleBinaryAutoRestart = true
	// When invoked via the many-ai-cli-launcher.exe Windows launcher's WSL
	// profile (and only then — not for plain `many-ai-cli serve` inside a WSL
	// shell), place logs under the
	// Windows %USERPROFILE% so the Hub UI's open-folder button resolves to a
	// plain C:\Users\... path that Windows Explorer can open directly. A bare
	// WSL session is treated as pure-Linux and keeps logs under Linux $HOME.
	logHome := home
	if wslutil.IsWindowsLauncherMode() {
		if winHome := wslutil.WindowsHomeAsUnix(); winHome != "" {
			logHome = winHome
		}
	}
	cfg.Hub.LogDir = filepath.Join(logHome, ".many-ai-cli", "logs")
	cfg.Hub.IdleTimeoutMin = 60
	cfg.Hub.WrapperReconnectGraceSec = 3600
	cfg.Log.Enabled = true
	// セッションログは既定で無効（オプトイン）。.log に秘密情報が平文で残るリスクのため。
	cfg.Log.SessionEnabled = false
	cfg.Log.MaxSizeMB = 10
	cfg.Log.MaxBackups = 3
	cfg.Log.Compress = false
	cfg.Log.SessionRetentionDays = 7
	cfg.Log.SessionMaxSizeMB = 50
	cfg.Log.AttachmentRetentionDays = 7
	cfg.Log.AttachmentMaxTotalMB = 500
	cfg.UserPrefs = UserPrefs{}
	cfg.Voice.Whisper.Language = "ja"
	cfg.Voice.Whisper.TimeoutSeconds = 60
	cfg.SlashCmdSources = DefaultSlashCmdSources()
	cfg.ApprovalPatternSources = DefaultApprovalPatternSources()
	cfg.ApprovalProfiles = DefaultApprovalProfiles()
	return cfg
}

func Save(cfg *Config) error {
	cfg.applyDefaults()
	if err := cfg.Validate(); err != nil {
		return err
	}
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
	return writeConfigAtomic(dir, path, out)
}

// writeConfigAtomic は out を path へ原子的に書き込む。
// temp ファイルを同一ディレクトリ dir に作成し、Sync + 0o600 設定後に Rename で
// 差し替える。同一ボリューム内の Rename はほぼ atomic であり、書き込み中断
// （クラッシュ・電源断）でも path を部分書き込み（破損）状態で残さない。
// パーミッション 0o600・同一ディレクトリ内 temp・戻り値エラーの挙動は Save の
// 旧実装と同一。LoadOrCreate のデフォルト/再生成/バックアップ書き込みでも共用する。
func writeConfigAtomic(dir, path string, out []byte) error {
	tmp, err := os.CreateTemp(dir, "config-*.yaml.tmp")
	if err != nil {
		return fmt.Errorf("create temp config: %w", err)
	}
	tmpName := tmp.Name()
	defer os.Remove(tmpName) // Rename 成功後は no-op
	if _, err := tmp.Write(out); err != nil {
		_ = tmp.Close()
		return fmt.Errorf("write temp config: %w", err)
	}
	if err := tmp.Sync(); err != nil {
		_ = tmp.Close()
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
	c.Hub.TrustedNetworks = cloneStringSlice(cfg.Hub.TrustedNetworks)
	c.Hub.AllowedHosts = cloneStringSlice(cfg.Hub.AllowedHosts)

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
	if cfg.Notify.Backends != nil {
		bs := make([]NotifyBackendConfig, len(cfg.Notify.Backends))
		copy(bs, cfg.Notify.Backends)
		c.Notify.Backends = bs
	}
	if cfg.Notify.Events != nil {
		c.Notify.Events = cloneStringSlice(cfg.Notify.Events)
	}
	if cfg.Voice.Whisper.HallucinationPhrases != nil {
		c.Voice.Whisper.HallucinationPhrases = cloneStringSlice(cfg.Voice.Whisper.HallucinationPhrases)
	}
	return &c
}

func (cfg *Config) applyDefaults() {
	if cfg == nil {
		return
	}
	if strings.TrimSpace(cfg.Voice.Whisper.Language) == "" {
		cfg.Voice.Whisper.Language = "ja"
	}
	if cfg.Voice.Whisper.TimeoutSeconds <= 0 {
		cfg.Voice.Whisper.TimeoutSeconds = 60
	}
	if strings.TrimSpace(cfg.Voice.Whisper.Model) == "" {
		cfg.Voice.Whisper.Model = "small"
	}
	if cfg.Voice.Whisper.HallucinationPhrases == nil {
		cfg.Voice.Whisper.HallucinationPhrases = cloneStringSlice(DefaultWhisperHallucinationPhrases)
	}
}

func (cfg *Config) Validate() error {
	if cfg == nil {
		return nil
	}
	if err := validateTrustedNetworks(cfg.Hub.TrustedNetworks); err != nil {
		return err
	}
	if err := validateAllowedHosts(cfg.Hub.AllowedHosts); err != nil {
		return err
	}
	if err := validateVoiceWhisper(cfg.Voice.Whisper); err != nil {
		return err
	}
	if err := validateOllama(cfg.Ollama); err != nil {
		return err
	}
	if err := validateLMStudio(cfg.LMStudio); err != nil {
		return err
	}
	return nil
}

func validateOllama(ollama OllamaConfig) error {
	baseURL := strings.TrimSpace(ollama.BaseURL)
	if baseURL == "" {
		return nil
	}
	u, err := neturl.Parse(baseURL)
	if err != nil || u.Scheme == "" || u.Host == "" {
		return fmt.Errorf("ollama.base_url must be an http or https base URL")
	}
	if u.Scheme != "http" && u.Scheme != "https" {
		return fmt.Errorf("ollama.base_url must use http or https")
	}
	if u.User != nil || u.RawQuery != "" || u.Fragment != "" {
		return fmt.Errorf("ollama.base_url must not include credentials, query, or fragment")
	}
	if u.Path != "" && u.Path != "/" {
		return fmt.Errorf("ollama.base_url must not include a path")
	}
	return nil
}

func validateLMStudio(lms LMStudioConfig) error {
	baseURL := strings.TrimSpace(lms.BaseURL)
	if baseURL == "" {
		return nil
	}
	u, err := neturl.Parse(baseURL)
	if err != nil || u.Scheme == "" || u.Host == "" {
		return fmt.Errorf("lm_studio.base_url must be an http or https base URL")
	}
	if u.Scheme != "http" && u.Scheme != "https" {
		return fmt.Errorf("lm_studio.base_url must use http or https")
	}
	if u.User != nil || u.RawQuery != "" || u.Fragment != "" {
		return fmt.Errorf("lm_studio.base_url must not include credentials, query, or fragment")
	}
	if u.Path != "" && u.Path != "/" {
		return fmt.Errorf("lm_studio.base_url must not include a path")
	}
	return nil
}

func validateVoiceWhisper(whisper VoiceWhisperConfig) error {
	serverURL := strings.TrimSpace(whisper.ServerURL)
	if serverURL != "" {
		u, err := neturl.Parse(serverURL)
		if err != nil || u.Scheme != "http" || u.Host == "" {
			return fmt.Errorf("voice.whisper.server_url must be a localhost http URL")
		}
		host := strings.TrimSuffix(strings.ToLower(strings.Trim(u.Hostname(), "[]")), ".")
		if host != "127.0.0.1" && host != "localhost" && host != "::1" {
			return fmt.Errorf("voice.whisper.server_url must point to localhost")
		}
	}
	requestPath := strings.TrimSpace(whisper.RequestPath)
	if requestPath != "" && (!strings.HasPrefix(requestPath, "/") || strings.Contains(requestPath, "://")) {
		return fmt.Errorf("voice.whisper.request_path must be empty or start with /")
	}
	if whisper.TimeoutSeconds < 1 || whisper.TimeoutSeconds > 300 {
		return fmt.Errorf("voice.whisper.timeout_seconds must be between 1 and 300")
	}
	if whisper.ServerPort != 0 && (whisper.ServerPort < 1024 || whisper.ServerPort > 65535) {
		return fmt.Errorf("voice.whisper.server_port must be between 1024 and 65535")
	}
	return nil
}

func validateTrustedNetworks(networks []string) error {
	for _, raw := range networks {
		value := strings.TrimSpace(raw)
		if value == "" {
			return fmt.Errorf("hub.trusted_networks contains empty CIDR")
		}
		_, cidr, err := net.ParseCIDR(value)
		if err != nil {
			return fmt.Errorf("hub.trusted_networks %q: %w", raw, err)
		}
		ones, bits := cidr.Mask.Size()
		if bits == 0 || ones == 0 {
			return fmt.Errorf("hub.trusted_networks %q is too broad", raw)
		}
	}
	return nil
}

func validateAllowedHosts(hosts []string) error {
	for _, raw := range hosts {
		host := strings.TrimSuffix(strings.ToLower(strings.TrimSpace(raw)), ".")
		if host == "" {
			return fmt.Errorf("hub.allowed_hosts contains empty host")
		}
		if _, _, err := net.SplitHostPort(host); err == nil {
			return fmt.Errorf("hub.allowed_hosts %q must not include a port", raw)
		}
		if strings.Contains(host, "*") || strings.Contains(host, "://") || strings.ContainsAny(host, "/\\") {
			return fmt.Errorf("hub.allowed_hosts %q is not a host name or IP literal", raw)
		}
		if ip := net.ParseIP(strings.Trim(host, "[]")); ip != nil {
			continue
		}
		if !isValidAllowedHostname(host) {
			return fmt.Errorf("hub.allowed_hosts %q is not a valid host name", raw)
		}
	}
	return nil
}

func isValidAllowedHostname(host string) bool {
	if len(host) > 253 {
		return false
	}
	labels := strings.Split(host, ".")
	for _, label := range labels {
		if label == "" || len(label) > 63 {
			return false
		}
		for i, r := range label {
			isAlphaNum := (r >= 'a' && r <= 'z') || (r >= '0' && r <= '9')
			if isAlphaNum {
				continue
			}
			if r == '-' && i > 0 && i < len(label)-1 {
				continue
			}
			return false
		}
	}
	return true
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
