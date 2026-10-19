//go:build windows

package clitrust

import (
	"strings"
	"testing"
)

// dunce::simplified の規則: \\?\C:\ の形だけを、普通の形で同じ意味になるときに
// 外す。UNC・長すぎるパス・予約名・末尾の点や空白を含むものは外さない。
func TestSimplifyVerbatimPath(t *testing.T) {
	long := `\\?\C:\` + strings.Repeat("a", 260)
	for in, want := range map[string]string{
		`\\?\C:\work\repo`:        `C:\work\repo`,
		`\\?\c:\`:                 `c:\`,
		`C:\work\repo`:            `C:\work\repo`,
		`\\?\UNC\server\share\x`:  `\\?\UNC\server\share\x`,
		`\\?\Volume{0000}\x`:      `\\?\Volume{0000}\x`,
		`\\?\C:\work\con`:         `\\?\C:\work\con`,
		`\\?\C:\work\CON.txt`:     `\\?\C:\work\CON.txt`,
		`\\?\C:\work\trailing.`:   `\\?\C:\work\trailing.`,
		`\\?\C:\work\trailing `:   `\\?\C:\work\trailing `,
		`\\?\C:\work\console`:     `C:\work\console`,
		long:                      long,
		`\\?\C:\work\Ärger\Datei`: `C:\work\Ärger\Datei`,
	} {
		if got := simplifyVerbatimPath(in); got != want {
			t.Errorf("simplifyVerbatimPath(%q) = %q, want %q", in, got, want)
		}
	}
}

// canonicalPath は存在するフォルダで \\?\ を付けずに返し、存在しないパスでは
// エラーを返す（codex もそのとき渡された綴りへ戻る）。
func TestCanonicalPathOnWindows(t *testing.T) {
	dir := t.TempDir()
	got, err := canonicalPath(dir)
	if err != nil {
		t.Fatalf("canonicalPath: %v", err)
	}
	if strings.HasPrefix(got, `\\?\`) {
		t.Errorf("canonicalPath kept the verbatim prefix: %q", got)
	}
	if !strings.EqualFold(got[:2], dir[:2]) {
		t.Errorf("canonicalPath changed the drive: %q → %q", dir, got)
	}
	if _, err := canonicalPath(dir + `\no-such-folder`); err == nil {
		t.Error("canonicalPath of a missing folder: want an error")
	}
}
