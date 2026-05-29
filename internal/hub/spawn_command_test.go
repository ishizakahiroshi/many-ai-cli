package hub

import (
	"os"
	"strings"
	"testing"
)

func TestSanitizeEnvRemovesEmptyPathEntries(t *testing.T) {
	sep := string(os.PathListSeparator)
	env := []string{
		"ANTHROPIC_API_KEY=keep-secret-value",
		"Path=" + strings.Join([]string{"", "alpha", " ", "beta", ""}, sep),
		"OTHER=value",
	}

	got := sanitizeEnv(env)
	if len(got) != len(env) {
		t.Fatalf("env len = %d, want %d: %#v", len(got), len(env), got)
	}
	if got[0] != env[0] || got[2] != env[2] {
		t.Fatalf("non-Path env entries changed: %#v", got)
	}
	pathValue := strings.TrimPrefix(got[1], "Path=")
	parts := strings.Split(pathValue, sep)
	for _, part := range parts {
		if strings.TrimSpace(part) == "" {
			t.Fatalf("sanitized Path contains empty entry: %q", got[1])
		}
	}
	if len(parts) < 2 || parts[0] != "alpha" || parts[1] != "beta" {
		t.Fatalf("sanitized Path prefix = %#v, want alpha/beta first", parts)
	}
}

func TestSanitizeEnvKeepsMalformedEntries(t *testing.T) {
	env := []string{"NO_EQUALS", "=bad", "PATH=C:\\Tools"}
	got := sanitizeEnv(env)
	if got[0] != "NO_EQUALS" || got[1] != "=bad" {
		t.Fatalf("malformed entries should be preserved: %#v", got)
	}
}
