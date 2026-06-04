package launcher

import (
	"strings"
	"testing"
)

func TestStartupBannerContainsSubtitle(t *testing.T) {
	out := StartupBanner("1.2.3")
	if !strings.Contains(out, "Connection launcher (WSL / SSH) v1.2.3") {
		t.Errorf("subtitle/version missing: %q", out)
	}
	if !strings.Contains(out, repositoryURL) {
		t.Errorf("GitHub URL missing: %q", out)
	}
	if !strings.Contains(out, "█") {
		t.Errorf("launcher banner should use the same Unicode block logo as the Hub: %q", out)
	}
}

func TestFormatVersionLabel(t *testing.T) {
	cases := map[string]string{"": "dev", "dev": "dev", "1.0.0": "v1.0.0", "v2.1.0": "v2.1.0"}
	for in, want := range cases {
		if got := formatVersionLabel(in); got != want {
			t.Errorf("formatVersionLabel(%q) = %q, want %q", in, got, want)
		}
	}
}

// 閉じたときの挙動説明は接続方式で出し分ける:
// tunnel はトンネルのみ切断（常駐 Hub 継続）、serve / wsl は Hub ごと終了。
func TestCloseBehaviorNoticeByMode(t *testing.T) {
	tunnel := CloseBehaviorNotice(Profile{Type: ProfileTypeSSH, Mode: SSHModeTunnel})
	if !strings.Contains(tunnel, "disconnects only the SSH tunnel") {
		t.Errorf("tunnel notice wrong: %q", tunnel)
	}
	for name, p := range map[string]Profile{
		"ssh-serve":   {Type: ProfileTypeSSH, Mode: SSHModeServe},
		"ssh-default": {Type: ProfileTypeSSH},
		"wsl":         {Type: ProfileTypeWSL},
	} {
		out := CloseBehaviorNotice(p)
		if !strings.Contains(out, "also stops the Hub started on the target") {
			t.Errorf("%s notice wrong: %q", name, out)
		}
	}
}
