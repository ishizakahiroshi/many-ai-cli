//go:build windows

package launcher

import "testing"

// buildTunnelHubURL: tunnel モードでは launcher だけが「SSH 経由・接続先 host」を
// 知っているため、バッジ表示用ヒントを URL クエリで Hub UI へ渡す。
func TestBuildTunnelHubURL(t *testing.T) {
	got := buildTunnelHubURL(47777, "abc123", "203.0.113.10")
	want := "http://127.0.0.1:47777/?token=abc123&via=ssh&host_label=203.0.113.10"
	if got != want {
		t.Errorf("buildTunnelHubURL = %q, want %q", got, want)
	}
}

// host にクエリとして危険な文字が含まれてもエスケープされること。
func TestBuildTunnelHubURLEscapesHost(t *testing.T) {
	got := buildTunnelHubURL(47777, "t", "my host&x=1")
	want := "http://127.0.0.1:47777/?token=t&via=ssh&host_label=my+host%26x%3D1"
	if got != want {
		t.Errorf("buildTunnelHubURL = %q, want %q", got, want)
	}
}

func TestBuildTunnelHubURLEscapesToken(t *testing.T) {
	got := buildTunnelHubURL(47777, "a+b&x=1", "host")
	want := "http://127.0.0.1:47777/?token=a%2Bb%26x%3D1&via=ssh&host_label=host"
	if got != want {
		t.Errorf("buildTunnelHubURL = %q, want %q", got, want)
	}
}

func TestNormalizeHubTokenRejectsWhitespace(t *testing.T) {
	for _, token := range []string{"", "abc def", "abc\ndef", "abc\tdef"} {
		if got, err := normalizeHubToken(token); err == nil {
			t.Errorf("normalizeHubToken(%q) = %q, want error", token, got)
		}
	}
}
