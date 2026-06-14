package hub

import (
	"testing"

	"many-ai-cli/internal/config"
)

// TestValidateNotifyBackend は通知バックエンドの URL/Type/Topic バリデーション
// （finding #35: notify backend URL が未検証で空/非 http スキームを許容）を確認する。
// 正常な ntfy/webhook 設定は通過し、空 URL・非 http(s) スキーム・ホスト欠落・
// ntfy の空 topic だけを弾くこと（保存形式・挙動互換）を検証する。
func TestValidateNotifyBackend(t *testing.T) {
	cases := []struct {
		name    string
		backend config.NotifyBackendConfig
		wantErr bool
	}{
		// 正常系（従来どおり通過する）
		{"webhook https", config.NotifyBackendConfig{Type: "webhook", URL: "https://example.com/hook"}, false},
		{"webhook http", config.NotifyBackendConfig{Type: "webhook", URL: "http://example.com/hook"}, false},
		{"ntfy https with topic", config.NotifyBackendConfig{Type: "ntfy", URL: "https://ntfy.sh", Topic: "my-topic"}, false},
		{"ntfy self-hosted lan", config.NotifyBackendConfig{Type: "ntfy", URL: "http://192.168.1.10:8080", Topic: "alerts"}, false},
		{"webhook url with surrounding spaces", config.NotifyBackendConfig{Type: "webhook", URL: "  https://example.com/hook  "}, false},

		// 異常系（拒否される）
		{"unknown type", config.NotifyBackendConfig{Type: "slack", URL: "https://example.com"}, true},
		{"empty type", config.NotifyBackendConfig{Type: "", URL: "https://example.com"}, true},
		{"empty url", config.NotifyBackendConfig{Type: "webhook", URL: ""}, true},
		{"whitespace url", config.NotifyBackendConfig{Type: "webhook", URL: "   "}, true},
		{"file scheme", config.NotifyBackendConfig{Type: "webhook", URL: "file:///etc/passwd"}, true},
		{"ftp scheme", config.NotifyBackendConfig{Type: "webhook", URL: "ftp://example.com/x"}, true},
		{"no scheme bare host", config.NotifyBackendConfig{Type: "webhook", URL: "example.com/hook"}, true},
		{"http no host", config.NotifyBackendConfig{Type: "webhook", URL: "http:///path"}, true},
		{"ntfy empty topic", config.NotifyBackendConfig{Type: "ntfy", URL: "https://ntfy.sh", Topic: ""}, true},
		{"ntfy whitespace topic", config.NotifyBackendConfig{Type: "ntfy", URL: "https://ntfy.sh", Topic: "   "}, true},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			err := validateNotifyBackend(tc.backend)
			if tc.wantErr && err == nil {
				t.Fatalf("validateNotifyBackend(%+v) = nil, want error", tc.backend)
			}
			if !tc.wantErr && err != nil {
				t.Fatalf("validateNotifyBackend(%+v) = %v, want nil", tc.backend, err)
			}
		})
	}
}
