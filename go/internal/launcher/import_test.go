package launcher

import (
	"context"
	"strings"
	"testing"
)

// --- UniqueProfileName ---

func TestUniqueProfileName(t *testing.T) {
	existing := []Profile{{Name: "vps"}, {Name: "vps-2"}, {Name: "other"}}
	cases := []struct {
		in   string
		want string
	}{
		{"fresh", "fresh"},   // 未使用はそのまま
		{"vps", "vps-3"},     // vps, vps-2 が埋まっているので vps-3
		{"other", "other-2"}, // 末尾採番
		{"", "remote"},       // 空は remote 既定
	}
	for _, c := range cases {
		if got := UniqueProfileName(existing, c.in); got != c.want {
			t.Errorf("UniqueProfileName(%q) = %q, want %q", c.in, got, c.want)
		}
	}
}

func TestUniqueProfileNameEmptyCollision(t *testing.T) {
	existing := []Profile{{Name: "remote"}}
	if got := UniqueProfileName(existing, ""); got != "remote-2" {
		t.Errorf("UniqueProfileName empty with collision = %q, want %q", got, "remote-2")
	}
}

// --- parseExportedProfile ---

func TestParseExportedProfileClean(t *testing.T) {
	in := []byte(`{"profile":{"name":"vps","type":"ssh","mode":"serve","host":"203.0.113.5","user":"root"},"host_candidates":["203.0.113.5","vps.local"]}`)
	got, err := parseExportedProfile(in)
	if err != nil {
		t.Fatal(err)
	}
	if got.Profile.Name != "vps" || got.Profile.Host != "203.0.113.5" {
		t.Errorf("unexpected profile: %+v", got.Profile)
	}
	if len(got.HostCandidates) != 2 || got.HostCandidates[1] != "vps.local" {
		t.Errorf("unexpected host candidates: %v", got.HostCandidates)
	}
}

func TestParseExportedProfileWithLeadingNoise(t *testing.T) {
	// ログインシェルが stdout に MOTD 等を混ぜたケース。
	in := []byte("Welcome to Ubuntu\nLast login: ...\n{\"profile\":{\"name\":\"vps\",\"type\":\"ssh\"},\"host_candidates\":[\"203.0.113.5\"]}\n")
	got, err := parseExportedProfile(in)
	if err != nil {
		t.Fatal(err)
	}
	if got.Profile.Name != "vps" {
		t.Errorf("name = %q, want vps", got.Profile.Name)
	}
}

func TestParseExportedProfileNoJSON(t *testing.T) {
	if _, err := parseExportedProfile([]byte("command not found\n")); err == nil {
		t.Fatal("expected error for non-JSON output")
	}
}

// --- FetchRemoteProfile: Validate 拒否（ssh への引数インジェクション対策） ---

func TestFetchRemoteProfileRejectsDashHost(t *testing.T) {
	// "-" 始まりの host は ssh のローカルオプションに化けるため Validate が弾く。
	// ssh を起動する前にエラーになる（コマンドは走らない）。
	_, err := FetchRemoteProfile(context.Background(), FetchParams{Host: "-oProxyCommand=evil"})
	if err == nil {
		t.Fatal("expected validation error for host starting with '-'")
	}
	if !strings.Contains(err.Error(), "must not start with '-'") {
		t.Errorf("unexpected error: %v", err)
	}
}

func TestFetchRemoteProfileRejectsWhitespaceHost(t *testing.T) {
	_, err := FetchRemoteProfile(context.Background(), FetchParams{Host: "1.2.3.4 evil"})
	if err == nil {
		t.Fatal("expected validation error for host containing whitespace")
	}
}
