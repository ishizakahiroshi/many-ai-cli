package config

import (
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"

	"gopkg.in/yaml.v3"
)

// TestSubscriptionsAbsentKeepsExistingBehaviour は `subscriptions:` を書いていない
// 既存 config.yaml が従来どおり読めることを確認する（後方互換の要）。
func TestSubscriptionsAbsentKeepsExistingBehaviour(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)
	dir := filepath.Join(home, ".many-ai-cli")
	if err := os.MkdirAll(dir, 0o700); err != nil {
		t.Fatal(err)
	}
	body := "token: abc123\nhub:\n  port: 47777\n"
	if err := os.WriteFile(filepath.Join(dir, "config.yaml"), []byte(body), 0o600); err != nil {
		t.Fatal(err)
	}
	cfg, err := LoadOrCreate()
	if err != nil {
		t.Fatalf("LoadOrCreate: %v", err)
	}
	if cfg.Token != "abc123" {
		t.Fatalf("token = %q, want abc123", cfg.Token)
	}
	if cfg.Subscriptions != nil {
		t.Fatalf("Subscriptions = %#v, want nil for a config without the key", cfg.Subscriptions)
	}
	if len(cfg.Warnings()) != 0 {
		t.Fatalf("Warnings() = %v, want none", cfg.Warnings())
	}
}

// TestSubscriptionsUnknownProviderKeyIsKept は未知の provider キーがあっても
// 読み込みが落ちないことを確認する（map なので構造体フィールドと違い増減に強い）。
func TestSubscriptionsUnknownProviderKeyIsKept(t *testing.T) {
	var cfg Config
	body := "subscriptions:\n  claude:\n    - id: main\n      name: Main\n  someday-cli:\n    - id: main\n"
	if err := yaml.Unmarshal([]byte(body), &cfg); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if len(cfg.Subscriptions["claude"]) != 1 {
		t.Fatalf("claude profiles = %d, want 1", len(cfg.Subscriptions["claude"]))
	}
	if len(cfg.Subscriptions["someday-cli"]) != 1 {
		t.Fatalf("unknown provider key was dropped: %#v", cfg.Subscriptions)
	}
}

// TestSubscriptionsMalformedSectionDoesNotFailWholeConfig は壊れた
// `subscriptions:` が config 全体の破損（.bak 退避＋token 再生成）を
// 引き起こさないことを確認する。
func TestSubscriptionsMalformedSectionDoesNotFailWholeConfig(t *testing.T) {
	for _, body := range []string{
		"token: keepme\nsubscriptions: \"not a map\"\n",
		"token: keepme\nsubscriptions:\n  claude: 42\n",
		"token: keepme\nsubscriptions:\n  claude:\n    - 7\n",
		"token: keepme\nsubscriptions:\n  claude:\n    - name: no id here\n",
	} {
		var cfg Config
		if err := yaml.Unmarshal([]byte(body), &cfg); err != nil {
			t.Fatalf("unmarshal(%q) returned an error; the whole config would have been discarded: %v", body, err)
		}
		if cfg.Token != "keepme" {
			t.Fatalf("token = %q for %q, want keepme", cfg.Token, body)
		}
		if len(cfg.Subscriptions) != 0 {
			t.Fatalf("Subscriptions = %#v for %q, want empty", cfg.Subscriptions, body)
		}
	}
}

func TestSubscriptionProfileIsEnabledDefaultsToTrue(t *testing.T) {
	if !(SubscriptionProfile{ID: "a"}).IsEnabled() {
		t.Fatal("a profile without an explicit enabled flag must count as enabled")
	}
	off := false
	if (SubscriptionProfile{ID: "a", Enabled: &off}).IsEnabled() {
		t.Fatal("enabled:false must disable the profile")
	}
	on := true
	if !(SubscriptionProfile{ID: "a", Enabled: &on}).IsEnabled() {
		t.Fatal("enabled:true must enable the profile")
	}
}

func TestValidateSubscriptionID(t *testing.T) {
	valid := []string{"main", "sub", "claude-max-2", "a", "a.b_c-d", "0"}
	for _, id := range valid {
		if err := ValidateSubscriptionID(id); err != nil {
			t.Errorf("ValidateSubscriptionID(%q) = %v, want nil", id, err)
		}
	}
	invalid := []string{
		"",              // 空
		"..",            // 親ディレクトリ
		"../../etc",     // path traversal
		"a..b",          // 途中の ..
		"/abs",          // 絶対パス
		`C:\abs`,        // Windows 絶対パス
		"a/b",           // パス区切り
		`a\b`,           // Windows パス区切り
		"Main",          // 大文字（NTFS と ext4 で意味が変わる）
		"-leading-dash", // 先頭記号
		".hidden",       // 先頭ドット
		"a b",           // 空白
		strings.Repeat("a", MaxSubscriptionIDLen+1),
	}
	for _, id := range invalid {
		if err := ValidateSubscriptionID(id); err == nil {
			t.Errorf("ValidateSubscriptionID(%q) = nil, want an error", id)
		}
	}
}

// TestResolveSubscriptionProfileDirStaysInsideRoot は profile ID を使った
// path traversal で subscriptions ルートの外へ書けないことを確認する。
func TestResolveSubscriptionProfileDirStaysInsideRoot(t *testing.T) {
	base := t.TempDir()
	root := SubscriptionsRoot(base)

	got, err := ResolveSubscriptionProfileDir(base, "claude", SubscriptionProfile{ID: "main"})
	if err != nil {
		t.Fatalf("ResolveSubscriptionProfileDir: %v", err)
	}
	want := filepath.Join(root, "claude", "main")
	if got != want {
		t.Fatalf("dir = %q, want %q", got, want)
	}

	for _, id := range []string{"../../etc", "..", "/etc/passwd", `..\..\Windows`, ""} {
		dir, err := ResolveSubscriptionProfileDir(base, "claude", SubscriptionProfile{ID: id})
		if err == nil {
			t.Fatalf("id %q resolved to %q; it must be rejected", id, dir)
		}
	}

	// provider キーもパス要素なので同じ検証を通す。
	if _, err := ResolveSubscriptionProfileDir(base, "../evil", SubscriptionProfile{ID: "main"}); err == nil {
		t.Fatal("a provider key containing .. must be rejected")
	}
}

func TestResolveSubscriptionProfileDirCustomPathMustBeAbsolute(t *testing.T) {
	base := t.TempDir()
	if _, err := ResolveSubscriptionProfileDir(base, "claude", SubscriptionProfile{ID: "main", ProfileDir: "relative/dir"}); err == nil {
		t.Fatal("a relative profile_dir must be rejected")
	}
	abs := filepath.Join(t.TempDir(), "elsewhere")
	got, err := ResolveSubscriptionProfileDir(base, "claude", SubscriptionProfile{ID: "main", ProfileDir: abs})
	if err != nil {
		t.Fatalf("absolute profile_dir: %v", err)
	}
	if got != filepath.Clean(abs) {
		t.Fatalf("dir = %q, want %q", got, abs)
	}
}

func TestResolveSubscriptionProfileDirExpandsTilde(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)
	got, err := ResolveSubscriptionProfileDir(t.TempDir(), "claude", SubscriptionProfile{ID: "main", ProfileDir: "~/custom-profile"})
	if err != nil {
		t.Fatalf("tilde profile_dir: %v", err)
	}
	want := filepath.Join(home, "custom-profile")
	if got != want {
		t.Fatalf("dir = %q, want %q", got, want)
	}
}

// TestSubscriptionsFindIsCaseInsensitive は保存済み ID の大文字小文字揺れでも
// 同じ profile を引くことを確認する（大文字 ID は登録時に弾くが、手書き config
// で混入しうる）。
func TestSubscriptionsFindNormalizes(t *testing.T) {
	profiles := SubscriptionProfiles{"claude": {{ID: "Main", Name: "Main"}}}
	if _, ok := profiles.Find("claude", " MAIN "); !ok {
		t.Fatal("Find must normalize case and surrounding whitespace")
	}
	if _, ok := profiles.Find("claude", ""); ok {
		t.Fatal("an empty id must not match any profile")
	}
	if _, ok := profiles.Find("codex", "main"); ok {
		t.Fatal("a profile must not be found under a different provider")
	}
}

// TestSubscriptionWarningsSurfaceBrokenEntries は「起動は止めないが利用者に伝える」
// 側に重複 ID・不正 ID が出ることを確認する。
func TestSubscriptionWarningsSurfaceBrokenEntries(t *testing.T) {
	cfg := &Config{Subscriptions: SubscriptionProfiles{
		"claude": {
			{ID: "main"},
			{ID: "MAIN"},                       // 正規化すると重複
			{ID: "../escape"},                  // 不正
			{ID: "ok", ProfileDir: "relative"}, // 相対パス
		},
	}}
	warnings := cfg.Warnings()
	if len(warnings) != 3 {
		t.Fatalf("Warnings() = %v, want 3 entries", warnings)
	}
	joined := strings.Join(warnings, "\n")
	for _, want := range []string{"duplicate profile id", "may only contain", "absolute path"} {
		if !strings.Contains(joined, want) {
			t.Errorf("warnings %q do not mention %q", joined, want)
		}
	}
	// 警告があっても起動は止めない（Validate は LoadOrCreate と同じく
	// applyDefaults の後に呼ばれる）。
	cfg.applyDefaults()
	if err := cfg.Validate(); err != nil {
		t.Fatalf("Validate() = %v; broken subscription entries must not stop the Hub", err)
	}
}

func TestSubscriptionProfilesCloneIsDeep(t *testing.T) {
	on := true
	src := SubscriptionProfiles{"claude": {{ID: "main", Name: "Main", Enabled: &on}}}
	dst := src.Clone()
	dst["claude"][0].Name = "changed"
	*dst["claude"][0].Enabled = false
	if src["claude"][0].Name != "Main" {
		t.Fatal("Clone shared the profile slice")
	}
	if !*src["claude"][0].Enabled {
		t.Fatal("Clone shared the Enabled pointer")
	}
	if SubscriptionProfiles(nil).Clone() != nil {
		t.Fatal("cloning nil must stay nil")
	}
}

// TestSubscriptionsRoundTripThroughYAML は保存した profile を読み戻せること、
// および secret を書ける余地が構造体に無いことを確認する。
func TestSubscriptionsRoundTripThroughYAML(t *testing.T) {
	off := false
	cfg := &Config{Subscriptions: SubscriptionProfiles{
		"claude": {{ID: "main", Name: "Claude Max Main", Plan: "max"}, {ID: "sub", Enabled: &off}},
	}}
	out, err := yaml.Marshal(cfg)
	if err != nil {
		t.Fatal(err)
	}
	text := string(out)
	for _, forbidden := range []string{"token", "credential", "refresh"} {
		if strings.Contains(strings.ToLower(text), forbidden) {
			// Config 全体を marshal しているので hub token 等は出るが、
			// subscriptions ブロックに現れてはいけない。
			block := text[strings.Index(text, "subscriptions:"):]
			if strings.Contains(strings.ToLower(block), forbidden) {
				t.Fatalf("subscriptions block leaks %q:\n%s", forbidden, block)
			}
		}
	}
	var back Config
	if err := yaml.Unmarshal(out, &back); err != nil {
		t.Fatal(err)
	}
	if len(back.Subscriptions["claude"]) != 2 {
		t.Fatalf("round trip lost profiles: %#v", back.Subscriptions)
	}
	if back.Subscriptions["claude"][1].IsEnabled() {
		t.Fatal("enabled:false did not survive the round trip")
	}
}

// TestSubscriptionsDirPermissions は既定 root が config ディレクトリの下に来ることを
// 確認する（実際の権限付与は internal/subscription.EnsureProfileDir が行う）。
func TestSubscriptionsRootIsUnderConfigDir(t *testing.T) {
	base := filepath.Join("x", ".many-ai-cli")
	root := SubscriptionsRoot(base)
	if filepath.Dir(root) != base {
		t.Fatalf("root %q is not directly under %q", root, base)
	}
	if runtime.GOOS == "windows" && strings.Contains(root, "/") {
		t.Fatalf("root %q must use OS separators", root)
	}
}
