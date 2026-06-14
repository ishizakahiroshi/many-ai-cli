package notify

import (
	"errors"
	neturl "net/url"
	"strings"
	"testing"
)

// secretTopicURL は ntfy のトピック秘密を含む URL の代表例。
// http.Client.Do が失敗すると *url.Error.Error() にこの URL 全体が埋め込まれ、
// 生のまま hub.log へ出すと秘密が平文で残る（finding #9）。
const secretTopicURL = "https://ntfy.sh/anyaicli-deadbeefcafef00d0123abcd"
const secretWebhookURL = "https://hooks.example.com/services/T000/B000?token=supersecret123"

// TestNotifySanitizeErrStripsURL は、*url.Error から URL（=トピック秘密 / token）が
// 完全に除去され、transport メッセージのみが残ることを検証する。
func TestNotifySanitizeErrStripsURL(t *testing.T) {
	// client.Do が DNS 解決失敗などで返す *url.Error を模す。
	ue := &neturl.Error{
		Op:  "Post",
		URL: secretTopicURL,
		Err: errors.New("dial tcp: lookup ntfy.sh: no such host"),
	}
	got := notifySanitizeErr(ue)

	if strings.Contains(got, secretTopicURL) {
		t.Fatalf("sanitized err leaks full URL: %q", got)
	}
	if strings.Contains(got, "anyaicli-deadbeefcafef00d0123abcd") {
		t.Fatalf("sanitized err leaks topic secret: %q", got)
	}
	if !strings.Contains(got, "no such host") {
		t.Fatalf("sanitized err dropped transport detail, diagnostics lost: %q", got)
	}
}

// TestNotifySanitizeErrUnwrapsThroughFmtWrap は、fmt.Errorf("%w") で wrap された
// *url.Error でも errors.As で到達でき URL が剥がれることを検証する。
func TestNotifySanitizeErrUnwrapsThroughFmtWrap(t *testing.T) {
	ue := &neturl.Error{
		Op:  "Post",
		URL: secretWebhookURL,
		Err: errors.New("connection refused"),
	}
	// sendWebhook の `fmt.Errorf("webhook: send: %w", err)` 相当。
	wrapped := wrapErr("webhook: send", ue)

	got := notifySanitizeErr(wrapped)

	if strings.Contains(got, "supersecret123") {
		t.Fatalf("sanitized err leaks webhook token: %q", got)
	}
	if strings.Contains(got, secretWebhookURL) {
		t.Fatalf("sanitized err leaks full webhook URL: %q", got)
	}
	if !strings.Contains(got, "connection refused") {
		t.Fatalf("sanitized err dropped transport detail: %q", got)
	}
}

// wrapErr は fmt.Errorf("%w") と同等の wrap をテスト内で再現する小ヘルパ。
func wrapErr(prefix string, err error) error {
	return &auditWrappedErr{prefix: prefix, err: err}
}

type auditWrappedErr struct {
	prefix string
	err    error
}

func (e *auditWrappedErr) Error() string { return e.prefix + ": " + e.err.Error() }
func (e *auditWrappedErr) Unwrap() error { return e.err }

// TestNotifySanitizeErrPassesNonURLErr は、URL を含まない自前エラー
// （build request / marshal / status N 等）はそのまま残ることを検証する。
func TestNotifySanitizeErrPassesNonURLErr(t *testing.T) {
	in := errors.New("ntfy: unexpected status 503")
	if got := notifySanitizeErr(in); got != "ntfy: unexpected status 503" {
		t.Fatalf("non-url err should pass through unchanged, got %q", got)
	}
	if got := notifySanitizeErr(nil); got != "" {
		t.Fatalf("nil err should yield empty string, got %q", got)
	}
}

// TestNotifyHostOf は backend URL から host のみが取り出され、
// パス/クエリ（秘密を含み得る部分）が漏れないことを検証する。
func TestNotifyHostOf(t *testing.T) {
	cases := []struct {
		in   string
		want string
	}{
		{secretTopicURL, "ntfy.sh"},
		{secretWebhookURL, "hooks.example.com"},
		{"https://ntfy.example.com:8443/anyaicli-secret", "ntfy.example.com:8443"},
		{"", "unknown"},
		{"::::not a url", "unknown"},
	}
	for _, c := range cases {
		got := notifyHostOf(c.in)
		if got != c.want {
			t.Errorf("notifyHostOf(%q) = %q, want %q", c.in, got, c.want)
		}
		// host だけのはずなので、トピック秘密や token を含まないこと。
		if strings.Contains(got, "anyaicli-") || strings.Contains(got, "token") || strings.Contains(got, "supersecret") {
			t.Errorf("notifyHostOf(%q) leaked secret material: %q", c.in, got)
		}
	}
}
