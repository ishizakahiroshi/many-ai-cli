package config

import (
	"encoding/json"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"

	"gopkg.in/yaml.v3"
)

func TestDefaultConfigOpensBrowser(t *testing.T) {
	cfg := defaultConfig(t.TempDir())
	if !cfg.Hub.OpenBrowser {
		t.Fatal("defaultConfig().Hub.OpenBrowser = false, want true")
	}
}

// TestNormalizeHandoffIntentModeDefaultsUnknownValues is the C3 completion
// criterion for the config half of 案 3 (docs/local/plan_session-handoff-board_c3_intent-layer.md
// 内部 C3): an empty or unrecognized value falls back to done-only, and the
// one recognized alternative round-trips unchanged.
func TestNormalizeHandoffIntentModeDefaultsUnknownValues(t *testing.T) {
	cases := map[string]string{
		"":             HandoffIntentModeDoneOnly,
		"done-only":    HandoffIntentModeDoneOnly,
		"turn-summary": HandoffIntentModeTurnSummary,
		"bogus":        HandoffIntentModeDoneOnly,
		"TURN-SUMMARY": HandoffIntentModeDoneOnly, // 大文字小文字は丸めない（TerminalColor と同じ厳密一致）
	}
	for in, want := range cases {
		if got := NormalizeHandoffIntentMode(in); got != want {
			t.Errorf("NormalizeHandoffIntentMode(%q) = %q, want %q", in, got, want)
		}
	}
}

func TestHandoffConfigTurnSummaryEnabled(t *testing.T) {
	if (HandoffConfig{}).TurnSummaryEnabled() {
		t.Fatal("zero-value HandoffConfig must default to done-only (turn-summary disabled)")
	}
	if !(HandoffConfig{IntentMode: HandoffIntentModeTurnSummary}).TurnSummaryEnabled() {
		t.Fatal("intent_mode=turn-summary must enable TurnSummaryEnabled")
	}
	if (HandoffConfig{IntentMode: "bogus"}).TurnSummaryEnabled() {
		t.Fatal("an unknown intent_mode must not enable turn-summary")
	}
}

// TestHandoffNoteOnThresholdDefaultsAndWarns is the 内部 C2 completion criterion
// for the config half (子 plan: docs/local/plan_derived-session-launch_c4_handoff-routes.md):
// an unset or misspelled value behaves as ask (the memo is never written
// without someone pressing the button), and a misspelling is reported instead
// of silently doing nothing.
func TestHandoffNoteOnThresholdDefaultsAndWarns(t *testing.T) {
	cases := map[string]string{
		"":     HandoffNoteOnThresholdAsk,
		"ask":  HandoffNoteOnThresholdAsk,
		"auto": HandoffNoteOnThresholdAuto,
		"off":  HandoffNoteOnThresholdOff,
		"AUTO": HandoffNoteOnThresholdAsk, // IntentMode と同じ厳密一致
		"none": HandoffNoteOnThresholdAsk,
	}
	for in, want := range cases {
		if got := (HandoffConfig{NoteOnThreshold: in}).NoteOnThresholdOrDefault(); got != want {
			t.Errorf("NoteOnThresholdOrDefault(%q) = %q, want %q", in, got, want)
		}
	}

	cfg := &Config{}
	cfg.Handoff.NoteOnThreshold = "auto"
	if got := cfg.handoffWarnings(); len(got) != 0 {
		t.Fatalf("handoffWarnings() for a valid value = %v, want none", got)
	}
	cfg.Handoff.NoteOnThreshold = "none"
	warnings := cfg.handoffWarnings()
	if len(warnings) != 1 || !strings.Contains(warnings[0], "handoff.note_on_threshold") {
		t.Fatalf("handoffWarnings() = %v, want one warning naming handoff.note_on_threshold", warnings)
	}
	// 値そのものは書き換えない（利用者が書いた綴りが残っているから警告できる）。
	if cfg.Handoff.NoteOnThreshold != "none" {
		t.Fatalf("note_on_threshold was rewritten to %q; it must stay as written", cfg.Handoff.NoteOnThreshold)
	}
}

func TestUsageProbeModelDefaultsAndRejectsUnsafeValues(t *testing.T) {
	cfg := defaultConfig(t.TempDir())
	if cfg.UserPrefs.UsageProbeModel != DefaultUsageProbeModel {
		t.Fatalf("default usage probe model = %q, want %q", cfg.UserPrefs.UsageProbeModel, DefaultUsageProbeModel)
	}

	for _, raw := range []string{"", "   ", "claude haiku", "claude;rm", "claude\tmodel"} {
		cfg.UserPrefs.UsageProbeModel = raw
		cfg.applyDefaults()
		if cfg.UserPrefs.UsageProbeModel != DefaultUsageProbeModel {
			t.Fatalf("invalid usage probe model %q became %q", raw, cfg.UserPrefs.UsageProbeModel)
		}
	}

	cfg.UserPrefs.UsageProbeModel = "claude-sonnet-4-5"
	cfg.applyDefaults()
	if cfg.UserPrefs.UsageProbeModel != "claude-sonnet-4-5" {
		t.Fatalf("valid usage probe model was not preserved: %q", cfg.UserPrefs.UsageProbeModel)
	}
}

func TestWorkflowJournalDefaultsOnAndCanBeDisabled(t *testing.T) {
	cfg := defaultConfig(t.TempDir())
	if !cfg.Workflow.JournalEnabled {
		t.Fatal("defaultConfig().Workflow.JournalEnabled = false, want true")
	}
	if err := yaml.Unmarshal([]byte("workflow:\n  journal_enabled: false\n"), cfg); err != nil {
		t.Fatal(err)
	}
	if cfg.Workflow.JournalEnabled {
		t.Fatal("explicit workflow.journal_enabled=false was ignored")
	}
	if cfg.UserPrefs.WorkflowCompletionNotify.Enabled {
		t.Fatal("workflow completion Push must remain opt-in")
	}
}

func TestBoardNotifyModeDefaultsAndValidation(t *testing.T) {
	cfg := defaultConfig(t.TempDir())
	if cfg.Orchestration.BoardNotifyMode != BoardNotifySoft {
		t.Fatalf("default board notify mode = %q, want %q", cfg.Orchestration.BoardNotifyMode, BoardNotifySoft)
	}
	for _, mode := range []BoardNotifyMode{BoardNotifySoft, BoardNotifyQueueUntilIdle, BoardNotifyInterrupt} {
		cfg.Orchestration.BoardNotifyMode = mode
		cfg.applyDefaults()
		if cfg.Orchestration.BoardNotifyMode != mode {
			t.Fatalf("applyDefaults replaced explicit mode %q with %q", mode, cfg.Orchestration.BoardNotifyMode)
		}
		if err := cfg.Validate(); err != nil {
			t.Fatalf("Validate() mode %q: %v", mode, err)
		}
	}
	cfg.Orchestration.BoardNotifyMode = "immediately"
	if err := cfg.Validate(); err == nil {
		t.Fatal("Validate() accepted invalid board notify mode")
	}
}

// TestChildStartupFastFailConfigDefaultsAndOverrides covers C1
// (plan_spawn-orchestration-backlog-closeout_c3_child-fast-fail.md): the two
// child_startup_* settings default to enabled with a 60s grace, and an
// explicit false/override in YAML survives applyDefaults.
func TestChildStartupFastFailConfigDefaultsAndOverrides(t *testing.T) {
	cfg := &Config{}
	cfg.applyDefaults()
	if cfg.Orchestration.ChildStartupGraceSeconds != 60 {
		t.Fatalf("default child_startup_grace_seconds = %d, want 60", cfg.Orchestration.ChildStartupGraceSeconds)
	}
	if !cfg.Orchestration.ChildStartupFailEnabled() {
		t.Fatal("default ChildStartupFailEnabled() = false, want true")
	}
	if !cfg.Orchestration.ChildStartupKillEnabled() {
		t.Fatal("default ChildStartupKillEnabled() = false, want true")
	}

	yamlCfg := &Config{}
	if err := yaml.Unmarshal([]byte("orchestration:\n  child_startup_fail_enabled: false\n  child_startup_kill: false\n  child_startup_grace_seconds: 30\n"), yamlCfg); err != nil {
		t.Fatal(err)
	}
	yamlCfg.applyDefaults()
	if yamlCfg.Orchestration.ChildStartupFailEnabled() {
		t.Fatal("explicit child_startup_fail_enabled=false was ignored by applyDefaults")
	}
	if yamlCfg.Orchestration.ChildStartupKillEnabled() {
		t.Fatal("explicit child_startup_kill=false was ignored by applyDefaults")
	}
	if yamlCfg.Orchestration.ChildStartupGraceSeconds != 30 {
		t.Fatalf("explicit child_startup_grace_seconds = %d, want 30 (applyDefaults must not clobber a positive value)", yamlCfg.Orchestration.ChildStartupGraceSeconds)
	}
}

// TestConfigCloneDeepCopiesChildStartupPointers covers C1's Clone requirement:
// ChildStartupFail / ChildStartupKill must not alias the source config's
// *bool after Clone (same pattern as WorktreeAuto).
func TestConfigCloneDeepCopiesChildStartupPointers(t *testing.T) {
	cfg := &Config{}
	failFalse := false
	killFalse := false
	cfg.Orchestration.ChildStartupFail = &failFalse
	cfg.Orchestration.ChildStartupKill = &killFalse

	clone := cfg.Clone()
	*cfg.Orchestration.ChildStartupFail = true
	*cfg.Orchestration.ChildStartupKill = true

	if clone.Orchestration.ChildStartupFail == nil || *clone.Orchestration.ChildStartupFail {
		t.Fatal("ChildStartupFail pointer was aliased by Clone")
	}
	if clone.Orchestration.ChildStartupKill == nil || *clone.Orchestration.ChildStartupKill {
		t.Fatal("ChildStartupKill pointer was aliased by Clone")
	}
}

func TestProviderDefaultsIncludeCopilot(t *testing.T) {
	slash := DefaultSlashCmdSources()
	if slash.Copilot == "" || !strings.Contains(slash.Copilot, "/copilot.md") {
		t.Fatalf("DefaultSlashCmdSources().Copilot = %q", slash.Copilot)
	}
	effSlash := EffectiveSlashCmdSources(SlashCmdSources{})
	if effSlash.Copilot != slash.Copilot {
		t.Fatalf("EffectiveSlashCmdSources().Copilot = %q, want %q", effSlash.Copilot, slash.Copilot)
	}

	patterns := DefaultApprovalPatternSources()
	if patterns.Copilot == "" || !strings.Contains(patterns.Copilot, "/copilot.md") {
		t.Fatalf("DefaultApprovalPatternSources().Copilot = %q", patterns.Copilot)
	}
	effPatterns := EffectiveApprovalPatternSources(ApprovalPatternSources{})
	if effPatterns.Copilot != patterns.Copilot {
		t.Fatalf("EffectiveApprovalPatternSources().Copilot = %q, want %q", effPatterns.Copilot, patterns.Copilot)
	}

	profiles := EffectiveApprovalProfiles(ApprovalProfiles{})
	if profiles.For("copilot") != ApprovalProfileOfficial {
		t.Fatalf("profiles.For(copilot) = %q", profiles.For("copilot"))
	}
	if got := profiles.WithProvider("copilot", ApprovalProfileCustom).For("copilot"); got != ApprovalProfileCustom {
		t.Fatalf("WithProvider(copilot, custom).For(copilot) = %q", got)
	}
}

func TestProviderDefaultsIncludeCursorAgent(t *testing.T) {
	slash := DefaultSlashCmdSources()
	if slash.CursorAgent == "" || !strings.Contains(slash.CursorAgent, "/cursor-agent.md") {
		t.Fatalf("DefaultSlashCmdSources().CursorAgent = %q", slash.CursorAgent)
	}
	effSlash := EffectiveSlashCmdSources(SlashCmdSources{})
	if effSlash.CursorAgent != slash.CursorAgent {
		t.Fatalf("EffectiveSlashCmdSources().CursorAgent = %q, want %q", effSlash.CursorAgent, slash.CursorAgent)
	}

	patterns := DefaultApprovalPatternSources()
	if patterns.CursorAgent == "" || !strings.Contains(patterns.CursorAgent, "/cursor-agent.md") {
		t.Fatalf("DefaultApprovalPatternSources().CursorAgent = %q", patterns.CursorAgent)
	}
	effPatterns := EffectiveApprovalPatternSources(ApprovalPatternSources{})
	if effPatterns.CursorAgent != patterns.CursorAgent {
		t.Fatalf("EffectiveApprovalPatternSources().CursorAgent = %q, want %q", effPatterns.CursorAgent, patterns.CursorAgent)
	}

	profiles := EffectiveApprovalProfiles(ApprovalProfiles{})
	if profiles.For("cursor-agent") != ApprovalProfileOfficial {
		t.Fatalf("profiles.For(cursor-agent) = %q", profiles.For("cursor-agent"))
	}
	if got := profiles.WithProvider("cursor-agent", ApprovalProfileCustom).For("cursor-agent"); got != ApprovalProfileCustom {
		t.Fatalf("WithProvider(cursor-agent, custom).For(cursor-agent) = %q", got)
	}
}

func TestProviderDefaultsIncludeGrok(t *testing.T) {
	slash := DefaultSlashCmdSources()
	if slash.Grok == "" || !strings.Contains(slash.Grok, "/grok.md") {
		t.Fatalf("DefaultSlashCmdSources().Grok = %q", slash.Grok)
	}
	effSlash := EffectiveSlashCmdSources(SlashCmdSources{})
	if effSlash.Grok != slash.Grok {
		t.Fatalf("EffectiveSlashCmdSources().Grok = %q, want %q", effSlash.Grok, slash.Grok)
	}

	patterns := DefaultApprovalPatternSources()
	if patterns.Grok == "" || !strings.Contains(patterns.Grok, "/grok.md") {
		t.Fatalf("DefaultApprovalPatternSources().Grok = %q", patterns.Grok)
	}
	effPatterns := EffectiveApprovalPatternSources(ApprovalPatternSources{})
	if effPatterns.Grok != patterns.Grok {
		t.Fatalf("EffectiveApprovalPatternSources().Grok = %q, want %q", effPatterns.Grok, patterns.Grok)
	}

	profiles := EffectiveApprovalProfiles(ApprovalProfiles{})
	if profiles.For("grok") != ApprovalProfileOfficial {
		t.Fatalf("profiles.For(grok) = %q", profiles.For("grok"))
	}
	if got := profiles.WithProvider("grok", ApprovalProfileCustom).For("grok"); got != ApprovalProfileCustom {
		t.Fatalf("WithProvider(grok, custom).For(grok) = %q", got)
	}
}

func TestProviderDefaultsIncludeCommandCode(t *testing.T) {
	slash := DefaultSlashCmdSources()
	if slash.CommandCode == "" || !strings.Contains(slash.CommandCode, "/command-code.md") {
		t.Fatalf("DefaultSlashCmdSources().CommandCode = %q", slash.CommandCode)
	}
	effSlash := EffectiveSlashCmdSources(SlashCmdSources{})
	if effSlash.CommandCode != slash.CommandCode {
		t.Fatalf("EffectiveSlashCmdSources().CommandCode = %q, want %q", effSlash.CommandCode, slash.CommandCode)
	}

	patterns := DefaultApprovalPatternSources()
	if patterns.CommandCode == "" || !strings.Contains(patterns.CommandCode, "/command-code.md") {
		t.Fatalf("DefaultApprovalPatternSources().CommandCode = %q", patterns.CommandCode)
	}
	effPatterns := EffectiveApprovalPatternSources(ApprovalPatternSources{})
	if effPatterns.CommandCode != patterns.CommandCode {
		t.Fatalf("EffectiveApprovalPatternSources().CommandCode = %q, want %q", effPatterns.CommandCode, patterns.CommandCode)
	}

	profiles := EffectiveApprovalProfiles(ApprovalProfiles{})
	if profiles.For("command-code") != ApprovalProfileOfficial {
		t.Fatalf("profiles.For(command-code) = %q", profiles.For("command-code"))
	}
	if got := profiles.WithProvider("command-code", ApprovalProfileCustom).For("command-code"); got != ApprovalProfileCustom {
		t.Fatalf("WithProvider(command-code, custom).For(command-code) = %q", got)
	}
}

// TestSaveAtomicWrite は Save が atomic write（temp + Rename）を使うことを確認する。
// 書き込み後に temp ファイルが残っていないこと、内容が一致することを検証する。
func TestSaveAtomicWrite(t *testing.T) {
	home := t.TempDir()
	dir := filepath.Join(home, ".many-ai-cli")
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatal(err)
	}

	// HOME を一時ディレクトリにすり替えて Save を実行する
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)

	cfg := defaultConfig(home)
	cfg.Token = "test-token-atomic"
	if err := Save(cfg); err != nil {
		t.Fatalf("Save: %v", err)
	}

	// config.yaml が存在すること
	path := filepath.Join(dir, "config.yaml")
	if _, err := os.Stat(path); err != nil {
		t.Fatalf("config.yaml not found: %v", err)
	}

	// temp ファイルが残っていないこと
	entries, err := os.ReadDir(dir)
	if err != nil {
		t.Fatal(err)
	}
	for _, e := range entries {
		if strings.HasSuffix(e.Name(), ".yaml.tmp") {
			t.Errorf("temp file left behind: %s", e.Name())
		}
	}

	// round-trip: 読み戻した設定が一致すること
	cfg2 := defaultConfig(home)
	b, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if err := unmarshalYAML(b, cfg2); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if cfg2.Token != cfg.Token {
		t.Errorf("token mismatch: got %q, want %q", cfg2.Token, cfg.Token)
	}
}

// TestLoadOrCreateNotExist はファイルが存在しない場合に新規作成されることを確認する。
func TestLoadOrCreateNotExist(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)

	cfg, err := LoadOrCreate()
	if err != nil {
		t.Fatalf("LoadOrCreate: %v", err)
	}
	if cfg.Token == "" {
		t.Error("token should be non-empty for new config")
	}

	// config.yaml が生成されていること
	path := filepath.Join(home, ".many-ai-cli", "config.yaml")
	if _, statErr := os.Stat(path); statErr != nil {
		t.Errorf("config.yaml not created: %v", statErr)
	}
}

func TestLoadOrCreatePrivatePermissions(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("Unix permission bits are not reliable on Windows")
	}
	home := t.TempDir()
	dir := filepath.Join(home, ".many-ai-cli")
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatal(err)
	}
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)

	if _, err := LoadOrCreate(); err != nil {
		t.Fatalf("LoadOrCreate: %v", err)
	}
	dirInfo, err := os.Stat(dir)
	if err != nil {
		t.Fatal(err)
	}
	if got := dirInfo.Mode().Perm(); got != DirMode {
		t.Fatalf("config dir mode = %o, want %o", got, DirMode)
	}
	configInfo, err := os.Stat(filepath.Join(dir, "config.yaml"))
	if err != nil {
		t.Fatal(err)
	}
	if got := configInfo.Mode().Perm(); got != 0o600 {
		t.Fatalf("config file mode = %o, want 600", got)
	}
}

func TestRandomTokenLength(t *testing.T) {
	token, err := randomToken()
	if err != nil {
		t.Fatalf("randomToken: %v", err)
	}
	if len(token) != 64 {
		t.Fatalf("token length = %d, want 64", len(token))
	}
}

// TestLoadOrCreateCorruptedFile は破損 YAML の場合に .bak が生成され、デフォルト設定で起動できることを確認する。
func TestLoadOrCreateCorruptedFile(t *testing.T) {
	home := t.TempDir()
	dir := filepath.Join(home, ".many-ai-cli")
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(dir, "config.yaml")
	bak := path + ".bak"

	// 破損 YAML を書き込む（yaml.v3 が確実にパースエラーを返す形式）
	corruptContent := "\t\tinvalid yaml content"
	if err := os.WriteFile(path, []byte(corruptContent), 0o600); err != nil {
		t.Fatal(err)
	}

	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)

	cfg, err := LoadOrCreate()
	if err != nil {
		t.Fatalf("LoadOrCreate should succeed on corrupt config, got: %v", err)
	}
	if cfg == nil {
		t.Fatal("cfg should not be nil")
	}

	// .bak が生成されていること
	if _, statErr := os.Stat(bak); statErr != nil {
		t.Errorf(".bak not created: %v", statErr)
	}

	// .bak の内容が元の破損データであること
	bakData, err := os.ReadFile(bak)
	if err != nil {
		t.Fatal(err)
	}
	if string(bakData) != corruptContent {
		t.Errorf(".bak content mismatch: got %q", string(bakData))
	}
}

// TestLoadOrCreatePermissionError は読み取り権限のないファイルで権限エラーが返ることを確認する。
// Windows では chmod 000 が機能しないためスキップする。
func TestLoadOrCreatePermissionError(t *testing.T) {
	if os.Getenv("OS") == "Windows_NT" {
		t.Skip("chmod 000 not reliable on Windows")
	}
	// UID 0（root）では chmod が無意味なのでスキップ
	if os.Getuid() == 0 {
		t.Skip("running as root; permission test not meaningful")
	}

	home := t.TempDir()
	dir := filepath.Join(home, ".many-ai-cli")
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(dir, "config.yaml")

	if err := os.WriteFile(path, []byte("token: abc\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	// 読み取り禁止
	if err := os.Chmod(path, 0o000); err != nil {
		t.Fatal(err)
	}
	defer os.Chmod(path, 0o600) //nolint:errcheck

	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)

	_, err := LoadOrCreate()
	if err == nil {
		t.Fatal("LoadOrCreate should return error for unreadable config")
	}
	if !strings.Contains(err.Error(), "read config:") {
		t.Errorf("unexpected error message: %v", err)
	}
}

// unmarshalYAML は yaml.Unmarshal のパッケージ内ヘルパ（テスト用）。
func unmarshalYAML(b []byte, v interface{}) error {
	return yaml.Unmarshal(b, v)
}

// TestLoadOrCreate_SpawnMigration は旧 cfg.Spawn.LastModel → UserPrefs.Spawn.LastModel
// への移行ロジックを確認する。
func TestLoadOrCreate_SpawnMigration(t *testing.T) {
	home := t.TempDir()
	dir := filepath.Join(home, ".many-ai-cli")
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatal(err)
	}
	// 旧形式（spawn.last_model に値がある）の config.yaml を書き込む
	oldYAML := `token: "old-migration-token"
spawn:
  last_model:
    claude: "claude-opus-4"
`
	path := filepath.Join(dir, "config.yaml")
	if err := os.WriteFile(path, []byte(oldYAML), 0o600); err != nil {
		t.Fatal(err)
	}

	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)

	cfg, err := LoadOrCreate()
	if err != nil {
		t.Fatalf("LoadOrCreate: %v", err)
	}

	// user_prefs.spawn.last_model に移行されていること
	if cfg.UserPrefs.Spawn.LastModel["claude"] != "claude-opus-4" {
		t.Errorf("UserPrefs.Spawn.LastModel[claude] = %q, want %q",
			cfg.UserPrefs.Spawn.LastModel["claude"], "claude-opus-4")
	}
	// 旧フィールドがクリアされていること
	if len(cfg.Spawn.LastModel) != 0 {
		t.Errorf("cfg.Spawn.LastModel should be nil after migration, got %v", cfg.Spawn.LastModel)
	}
}

// TestSaveRoundTrip は Save → LoadOrCreate の round-trip で設定が一致することを確認する。
func TestSaveRoundTrip(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)

	// 初回作成
	cfg1, err := LoadOrCreate()
	if err != nil {
		t.Fatalf("LoadOrCreate (first): %v", err)
	}
	cfg1.Token = "roundtrip-token"
	cfg1.UserPrefs.Display.Theme = "dark"
	cfg1.UserPrefs.Spawn.LastModel = map[string]string{"claude": "claude-sonnet-4"}

	if err := Save(cfg1); err != nil {
		t.Fatalf("Save: %v", err)
	}

	// 再読み込み
	cfg2, err := LoadOrCreate()
	if err != nil {
		t.Fatalf("LoadOrCreate (second): %v", err)
	}
	if cfg2.Token != cfg1.Token {
		t.Errorf("Token: got %q, want %q", cfg2.Token, cfg1.Token)
	}
	if cfg2.UserPrefs.Display.Theme != "dark" {
		t.Errorf("Theme: got %q, want %q", cfg2.UserPrefs.Display.Theme, "dark")
	}
	if cfg2.UserPrefs.Spawn.LastModel["claude"] != "claude-sonnet-4" {
		t.Errorf("LastModel[claude]: got %q, want %q",
			cfg2.UserPrefs.Spawn.LastModel["claude"], "claude-sonnet-4")
	}
}

func TestHubTokenlessAccessRoundTrip(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)

	cfg1, err := LoadOrCreate()
	if err != nil {
		t.Fatalf("LoadOrCreate: %v", err)
	}
	cfg1.Hub.AllowLoopbackWithoutToken = true
	cfg1.Hub.TrustedNetworks = []string{"172.19.0.1/32"}
	cfg1.Hub.AllowedHosts = []string{"10.8.0.1", "hub.example"}
	if err := Save(cfg1); err != nil {
		t.Fatalf("Save: %v", err)
	}

	cfg2, err := LoadOrCreate()
	if err != nil {
		t.Fatalf("LoadOrCreate second: %v", err)
	}
	if !cfg2.Hub.AllowLoopbackWithoutToken {
		t.Fatal("AllowLoopbackWithoutToken = false, want true")
	}
	if got := strings.Join(cfg2.Hub.TrustedNetworks, ","); got != "172.19.0.1/32" {
		t.Fatalf("TrustedNetworks = %q", got)
	}
	if got := strings.Join(cfg2.Hub.AllowedHosts, ","); got != "10.8.0.1,hub.example" {
		t.Fatalf("AllowedHosts = %q", got)
	}
}

func TestOllamaBaseURLRoundTripAndValidation(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)

	cfg1, err := LoadOrCreate()
	if err != nil {
		t.Fatalf("LoadOrCreate: %v", err)
	}
	cfg1.Ollama.BaseURL = "http://192.168.11.50:11434"
	cfg1.Ollama.AllowPrivateHosts = true
	if err := Save(cfg1); err != nil {
		t.Fatalf("Save: %v", err)
	}

	cfg2, err := LoadOrCreate()
	if err != nil {
		t.Fatalf("LoadOrCreate second: %v", err)
	}
	if cfg2.Ollama.BaseURL != "http://192.168.11.50:11434" {
		t.Fatalf("Ollama.BaseURL = %q", cfg2.Ollama.BaseURL)
	}
	if !cfg2.Ollama.AllowPrivateHosts {
		t.Fatal("Ollama.AllowPrivateHosts = false, want true")
	}
	if got := EffectiveOllamaBaseURL(""); got != DefaultOllamaBaseURL {
		t.Fatalf("EffectiveOllamaBaseURL(\"\") = %q", got)
	}

	for _, raw := range []string{
		"ftp://127.0.0.1:11434",
		"http://127.0.0.1:11434/v1",
		"http://user:pass@127.0.0.1:11434",
		"http://127.0.0.1:11434?x=1",
	} {
		cfg := defaultConfig(t.TempDir())
		cfg.Ollama.BaseURL = raw
		if err := cfg.Validate(); err == nil {
			t.Fatalf("Validate() with ollama.base_url %q succeeded, want error", raw)
		}
	}
	// private host + opt-in 無しは起動を止めない（error ではなく Warnings() で伝える）。
	// 任意機能であるモデル一覧の設定不備で serve / version / stop / wrap まで
	// 全滅するのを防ぐため。実際の遮断は internal/hub/models_fetch.go の
	// newLocalModelHTTPClient がトランスポート層で行う。
	// localhost / 127.0.0.1 も isPrivateModelHost では private 判定になるので、
	// Ollama の標準構成を明示指定しただけのケースも起動できることを併せて確認する。
	for _, target := range []struct {
		name string
		set  func(*Config)
	}{
		{"ollama", func(cfg *Config) { cfg.Ollama.BaseURL = "http://10.0.0.20:11434" }},
		{"lm_studio", func(cfg *Config) { cfg.LMStudio.BaseURL = "http://192.168.1.20:1234" }},
		{"ollama", func(cfg *Config) { cfg.Ollama.BaseURL = "http://localhost:11434" }},
	} {
		cfg := defaultConfig(t.TempDir())
		target.set(cfg)
		if err := cfg.Validate(); err != nil {
			t.Fatalf("Validate() with private %s host failed: %v", target.name, err)
		}
		warnings := cfg.Warnings()
		if len(warnings) != 1 || !strings.Contains(warnings[0], target.name+".base_url") {
			t.Fatalf("Warnings() with private %s host = %v, want exactly one warning naming %s.base_url",
				target.name, warnings, target.name)
		}
	}
	// opt-in 済みなら警告も出ない。
	for _, target := range []struct {
		name string
		set  func(*Config)
	}{
		{"ollama", func(cfg *Config) {
			cfg.Ollama.BaseURL = "http://10.0.0.20:11434"
			cfg.Ollama.AllowPrivateHosts = true
		}},
		{"lm_studio", func(cfg *Config) {
			cfg.LMStudio.BaseURL = "http://192.168.1.20:1234"
			cfg.LMStudio.AllowPrivateHosts = true
		}},
	} {
		cfg := defaultConfig(t.TempDir())
		target.set(cfg)
		if got := cfg.Warnings(); len(got) != 0 {
			t.Fatalf("Warnings() with %s opt-in = %v, want none", target.name, got)
		}
	}
}

func TestConfigValidationRejectsUnsafeTrustedNetworks(t *testing.T) {
	// IPv4 は /24、IPv6 は /64 より広い CIDR も too broad として拒否する（MAC-09）。
	// 検証はマスク幅だけを見るので、アドレスは全て RFC5737 TEST-NET / ULA の
	// 例示リテラルで表す（RFC1918 はローカル secrets-scan が、CGNAT 等の
	// 公開レンジは CI の public-ipv4-literal ルールが弾くため、両方が許す
	// 例示 IP に寄せる）。
	for _, cidr := range []string{"0.0.0.0/0", "::/0", "not-a-cidr", "203.0.113.0/8", "0.0.0.0/1", "203.0.113.0/10", "198.51.100.0/16", "::/1", "fd00::/48"} {
		cfg := defaultConfig(t.TempDir())
		cfg.Hub.TrustedNetworks = []string{cidr}
		if err := cfg.Validate(); err == nil {
			t.Fatalf("Validate() with trusted network %q succeeded, want error", cidr)
		}
	}
}

func TestConfigValidationAcceptsNarrowTrustedNetworks(t *testing.T) {
	for _, cidr := range []string{"198.51.100.0/24", "203.0.113.7/32", "fd00::/64", "fe80::1/128"} {
		cfg := defaultConfig(t.TempDir())
		cfg.Hub.TrustedNetworks = []string{cidr}
		if err := cfg.Validate(); err != nil {
			t.Fatalf("Validate() with trusted network %q failed: %v", cidr, err)
		}
	}
}

func TestConfigValidationRejectsInvalidAllowedHosts(t *testing.T) {
	for _, host := range []string{"", "*", "10.8.0.1:47801", "http://10.8.0.1", "bad/host"} {
		cfg := defaultConfig(t.TempDir())
		cfg.Hub.AllowedHosts = []string{host}
		if err := cfg.Validate(); err == nil {
			t.Fatalf("Validate() with allowed host %q succeeded, want error", host)
		}
	}
}

func TestVoiceWhisperConfigDefaultsAndValidation(t *testing.T) {
	cfg := defaultConfig(t.TempDir())
	if cfg.Voice.Whisper.Language != "ja" {
		t.Fatalf("default whisper language = %q, want ja", cfg.Voice.Whisper.Language)
	}
	if cfg.Voice.Whisper.TimeoutSeconds != 60 {
		t.Fatalf("default whisper timeout = %d, want 60", cfg.Voice.Whisper.TimeoutSeconds)
	}
	cfg.Voice.Whisper.ServerURL = "http://127.0.0.1:8178"
	cfg.Voice.Whisper.RequestPath = "/inference"
	if err := cfg.Validate(); err != nil {
		t.Fatalf("Validate localhost whisper config: %v", err)
	}

	cfg.Voice.Whisper.ServerURL = "https://api.openai.com"
	if err := cfg.Validate(); err == nil {
		t.Fatal("Validate external whisper server succeeded, want error")
	}
}

func TestConfigCloneDeepCopiesUserPrefs(t *testing.T) {
	cfg := &Config{}
	cfg.Spawn.LastModel = map[string]string{"legacy": "a"}
	cfg.UserPrefs.ProjectFavorites = []string{"one"}
	cfg.UserPrefs.CwdHistory = []string{"D:/dev/one"}
	cfg.UserPrefs.Spawn.Defaults = map[string]string{"claude": "default"}
	cfg.UserPrefs.Spawn.LastModel = map[string]string{"claude": "sonnet"}
	cfg.Hub.TrustedNetworks = []string{"172.19.0.1/32"}
	cfg.Hub.AllowedHosts = []string{"10.8.0.1"}

	clone := cfg.Clone()
	cfg.Spawn.LastModel["legacy"] = "b"
	cfg.UserPrefs.ProjectFavorites[0] = "two"
	cfg.UserPrefs.CwdHistory[0] = "D:/dev/two"
	cfg.UserPrefs.Spawn.Defaults["claude"] = "changed"
	cfg.UserPrefs.Spawn.LastModel["claude"] = "opus"
	cfg.Hub.TrustedNetworks[0] = "172.19.0.2/32"
	cfg.Hub.AllowedHosts[0] = "10.8.0.2"

	if clone.Spawn.LastModel["legacy"] != "a" {
		t.Fatalf("legacy spawn map was aliased")
	}
	if clone.UserPrefs.ProjectFavorites[0] != "one" {
		t.Fatalf("project favorites slice was aliased")
	}
	if clone.UserPrefs.CwdHistory[0] != "D:/dev/one" {
		t.Fatalf("cwd history slice was aliased")
	}
	if clone.UserPrefs.Spawn.Defaults["claude"] != "default" {
		t.Fatalf("spawn defaults map was aliased")
	}
	if clone.UserPrefs.Spawn.LastModel["claude"] != "sonnet" {
		t.Fatalf("spawn last model map was aliased")
	}
	if clone.Hub.TrustedNetworks[0] != "172.19.0.1/32" {
		t.Fatalf("trusted networks slice was aliased")
	}
	if clone.Hub.AllowedHosts[0] != "10.8.0.1" {
		t.Fatalf("allowed hosts slice was aliased")
	}
}

func sessionOrderEqual(got SessionOrderIDs, want []int) bool {
	if len(got) != len(want) {
		return false
	}
	for i := range want {
		if got[i] != want[i] {
			return false
		}
	}
	return true
}

// session_order は以前 []string で宣言されており、旧版が文字列配列を書いた
// config.yaml が残っている環境がある。型を変えた結果 yaml.Unmarshal がそこで
// 失敗すると、LoadOrCreate の破損フォールバックが config.yaml 全体を .bak へ
// 退避してデフォルト再生成する（token も作り直しになる）。壊れた 1 項目のために
// 設定全体を失わないことをここで固定する。
func TestSessionOrderYAMLAcceptsNumbersAndLegacyStrings(t *testing.T) {
	cases := []struct {
		name string
		src  string
		want []int
	}{
		{"numbers", "user_prefs:\n    session_order: [10, 5, 3]\n", []int{10, 5, 3}},
		{"legacy strings", "user_prefs:\n    session_order: [\"10\", \"5\"]\n", []int{10, 5}},
		{"mixed junk dropped", "user_prefs:\n    session_order: [10, \"abc\", 7]\n", []int{10, 7}},
		{"not a sequence", "user_prefs:\n    session_order: nope\n", nil},
		{"empty", "user_prefs:\n    session_order: []\n", nil},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			var cfg Config
			if err := yaml.Unmarshal([]byte(tc.src), &cfg); err != nil {
				t.Fatalf("yaml.Unmarshal() error = %v; whole config must stay loadable", err)
			}
			if !sessionOrderEqual(cfg.UserPrefs.SessionOrder, tc.want) {
				t.Fatalf("SessionOrder = %v, want %v", cfg.UserPrefs.SessionOrder, tc.want)
			}
		})
	}
}

func TestSessionOrderJSONAcceptsNumbersAndLegacyStrings(t *testing.T) {
	var prefs UserPrefs
	if err := json.Unmarshal([]byte(`{"session_order":[11,2,"7","x"]}`), &prefs); err != nil {
		t.Fatalf("json.Unmarshal() error = %v; PUT /api/user-prefs must stay accepted", err)
	}
	if !sessionOrderEqual(prefs.SessionOrder, []int{11, 2, 7}) {
		t.Fatalf("SessionOrder = %v, want [11 2 7]", prefs.SessionOrder)
	}
}

func TestSessionOrderSurvivesSaveRoundTrip(t *testing.T) {
	var cfg Config
	cfg.UserPrefs.SessionOrder = SessionOrderIDs{4, 9, 1}
	out, err := yaml.Marshal(&cfg)
	if err != nil {
		t.Fatalf("yaml.Marshal() error = %v", err)
	}
	var back Config
	if err := yaml.Unmarshal(out, &back); err != nil {
		t.Fatalf("yaml.Unmarshal() error = %v", err)
	}
	if !sessionOrderEqual(back.UserPrefs.SessionOrder, []int{4, 9, 1}) {
		t.Fatalf("SessionOrder = %v, want [4 9 1]", back.UserPrefs.SessionOrder)
	}
	clone := cfg.Clone()
	cfg.UserPrefs.SessionOrder[0] = 99
	if clone.UserPrefs.SessionOrder[0] != 4 {
		t.Fatalf("session order slice was aliased")
	}
}

func TestChildFullBypassEnabledDefaultTrue(t *testing.T) {
	var o OrchestrationConfig
	if !o.ChildFullBypassEnabled() {
		t.Fatal("nil ChildFullBypass should default to true")
	}
	f := false
	o.ChildFullBypass = &f
	if o.ChildFullBypassEnabled() {
		t.Fatal("explicit false should disable full bypass")
	}
	tr := true
	o.ChildFullBypass = &tr
	if !o.ChildFullBypassEnabled() {
		t.Fatal("explicit true should enable full bypass")
	}
}
