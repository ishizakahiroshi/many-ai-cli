package sessionlog

import (
	"strings"
	"testing"
)

// TestMaskSecretsPasswordAndDSN は finding #16 の回帰テスト。
// PASSWORD= 等の代入形・資格情報付き URL・PEM 秘密鍵ブロックが伏字化されること、
// かつキー名 / scheme / host など秘密でない文脈は保持されることを確認する。
func TestMaskSecretsPasswordAndDSN(t *testing.T) {
	cases := []struct {
		name   string
		input  string
		secret string // 出力に含まれてはいけない秘密値
		keep   string // 出力に残っているべき非秘密の文脈（空なら検査しない）
	}{
		{
			name:   "DB_PASSWORD assignment",
			input:  "DB_PASSWORD=hunter2longvalue",
			secret: "hunter2longvalue",
			keep:   "DB_PASSWORD=",
		},
		{
			name:   "PASSWORD assignment no prefix",
			input:  "PASSWORD=topsecretvalue",
			secret: "topsecretvalue",
			keep:   "PASSWORD=",
		},
		{
			name:   "SECRET colon separator",
			input:  "client_SECRET: abcdef123456",
			secret: "abcdef123456",
			keep:   "SECRET",
		},
		{
			name:   "ACCESS_TOKEN assignment",
			input:  "ACCESS_TOKEN=abcdefghijklmno",
			secret: "abcdefghijklmno",
			keep:   "ACCESS_TOKEN=",
		},
		{
			name:   "postgres DSN credentials",
			input:  "postgres://user:secretpw@host:5432/db",
			secret: "secretpw",
			keep:   "postgres://user:",
		},
		{
			name:   "mysql DSN credentials keeps host",
			input:  "mysql://admin:p4ssw0rd@db.example.com/app",
			secret: "p4ssw0rd",
			keep:   "@db.example.com/app",
		},
		{
			name:   "PEM private key block",
			input:  "-----BEGIN RSA PRIVATE KEY-----\nMIIBOgIBAAJBAKj34GkxFh\n-----END RSA PRIVATE KEY-----",
			secret: "MIIBOgIBAAJBAKj34GkxFh",
		},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			got := MaskSecrets(tc.input)
			if tc.secret != "" && strings.Contains(got, tc.secret) {
				t.Fatalf("MaskSecrets(%q) = %q; still contains secret %q", tc.input, got, tc.secret)
			}
			if !strings.Contains(got, "***") {
				t.Fatalf("MaskSecrets(%q) = %q; expected a masked marker", tc.input, got)
			}
			if tc.keep != "" && !strings.Contains(got, tc.keep) {
				t.Fatalf("MaskSecrets(%q) = %q; dropped non-secret context %q", tc.input, got, tc.keep)
			}
		})
	}
}

// TestMaskSecretsNoOverMaskShortValues は短い値・非シークレット文脈を過剰マスクしないことを確認する。
func TestMaskSecretsNoOverMask(t *testing.T) {
	preserved := []string{
		// 値が 6 文字未満の代入は対象外（過剰マスク防止）。
		"PWD=abc",
		// 通常の URL（資格情報なし）はパスワード伏字化の対象外。
		"https://example.com/path?q=1",
		// password という語を含むが代入形ではない通常文。
		"please reset your password soon",
		// keyword を含むが区切り（=/:）が無い通常文。
		"the access token flow is documented here",
	}
	for _, in := range preserved {
		if got := MaskSecrets(in); got != in {
			t.Errorf("MaskSecrets over-masked normal text: %q -> %q", in, got)
		}
	}
}
