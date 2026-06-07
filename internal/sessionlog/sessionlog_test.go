package sessionlog

import (
	"encoding/json"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
	"time"
)

func TestBaseName(t *testing.T) {
	meta := Metadata{
		SessionID: 12,
		Provider:  "claude",
		CWD:       filepath.Join("anywhere", "any-ai-cli"),
		StartedAt: time.Date(2026, 5, 10, 9, 15, 32, 0, time.Local),
	}
	got := BaseName(meta)
	if !strings.HasPrefix(got, "claude_2026-05-10_091532_any-ai-cli_s12") {
		t.Fatalf("unexpected basename: %s", got)
	}
}

func TestSanitizeFilePart(t *testing.T) {
	if got := SanitizeFilePart(`foo:bar<>baz`); got != "foo_bar__baz" {
		t.Fatalf("sanitize failed: %s", got)
	}
	if got := SanitizeFilePart(""); got != "no-project" {
		t.Fatalf("empty sanitize failed: %s", got)
	}
	long := strings.Repeat("a", 120)
	if got := SanitizeFilePart(long); len(got) != 80 {
		t.Fatalf("expected 80 chars, got %d", len(got))
	}
}

func TestStripANSIRemovesOSCAndCSI(t *testing.T) {
	in := "\x1b]0;title\x07hello \x1b[31mred\x1b[0m"
	if got := StripANSI(in); got != "hello red" {
		t.Fatalf("unexpected strip result: %q", got)
	}
}

func TestWriterSizeCap(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "test.jsonl")

	const maxBytes = 1024 * 1024 // 1 MB cap
	w, err := NewJSONLWriter(path, maxBytes)
	if err != nil {
		t.Fatal(err)
	}
	defer w.Close()

	// 最初のイベント: 1.1 MB — キャップ未満から書き込むので許可される。
	// 書き込み後に written が cap を超え、以降の書き込みがブロックされる。
	big := strings.Repeat("x", 1100*1024)
	if err := w.Event(map[string]string{"data": big}); err != nil {
		t.Fatal("first event failed:", err)
	}

	// 2 番目のイベント — written >= maxBytes なので no-op になるはず。
	if err := w.Event(map[string]string{"n": "2"}); err != nil {
		t.Fatal("second event failed:", err)
	}

	_ = w.Close()

	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}

	// ファイルは 1 行または 2 行（truncated マーカーを含む場合）。
	// 第2イベント（{"n":"2"}）は含まれないこと。
	lines := strings.Split(strings.TrimRight(string(data), "\n"), "\n")
	if len(lines) < 1 || len(lines) > 2 {
		t.Fatalf("expected 1 or 2 lines (1st event + optional truncated marker), got %d\n%s", len(lines), string(data))
	}

	var m map[string]string
	if err := json.Unmarshal([]byte(lines[0]), &m); err != nil {
		t.Fatalf("invalid JSON in line 1: %v", err)
	}
	if m["data"] != big {
		t.Fatal("unexpected data in first event")
	}
	if strings.Contains(string(data), `"n":"2"`) {
		t.Fatal("second event should have been blocked by size cap")
	}
}

func TestNewJSONLWriterUsesPrivatePermissions(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("Windows does not report POSIX mode bits reliably")
	}
	dir := filepath.Join(t.TempDir(), "logs", "sessions")
	path := filepath.Join(dir, "test.jsonl")

	w, err := NewJSONLWriter(path, 0)
	if err != nil {
		t.Fatal(err)
	}
	if err := w.Close(); err != nil {
		t.Fatal(err)
	}

	dirInfo, err := os.Stat(dir)
	if err != nil {
		t.Fatal(err)
	}
	if got := dirInfo.Mode().Perm(); got != PrivateDirMode {
		t.Fatalf("dir mode = %#o, want %#o", got, PrivateDirMode)
	}
	fileInfo, err := os.Stat(path)
	if err != nil {
		t.Fatal(err)
	}
	if got := fileInfo.Mode().Perm(); got != PrivateFileMode {
		t.Fatalf("file mode = %#o, want %#o", got, PrivateFileMode)
	}
}

func TestWriteTranscriptFileUsesPrivatePermissions(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("Windows does not report POSIX mode bits reliably")
	}
	dir := t.TempDir()
	jsonlPath := filepath.Join(dir, "session.jsonl")
	outPath := filepath.Join(dir, "session.txt")
	if err := os.WriteFile(jsonlPath, []byte(`{"type":"session_end","state":"completed"}`+"\n"), PrivateFileMode); err != nil {
		t.Fatal(err)
	}
	if err := WriteTranscriptFile(jsonlPath, outPath); err != nil {
		t.Fatal(err)
	}
	info, err := os.Stat(outPath)
	if err != nil {
		t.Fatal(err)
	}
	if got := info.Mode().Perm(); got != PrivateFileMode {
		t.Fatalf("transcript mode = %#o, want %#o", got, PrivateFileMode)
	}
}

// TestWriterTruncatedMarker は上限到達時に log_truncated マーカーが一度だけ書かれることを確認する。
func TestWriterTruncatedMarker(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "trunc.jsonl")

	const maxBytes = 100
	w, err := NewJSONLWriter(path, maxBytes)
	if err != nil {
		t.Fatal(err)
	}

	// 上限を超えるイベントを連続書き込み
	for i := 0; i < 10; i++ {
		_ = w.Event(map[string]string{"x": strings.Repeat("a", 20)})
	}
	_ = w.Close()

	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}

	content := string(data)
	// log_truncated が含まれること
	if !strings.Contains(content, `"log_truncated"`) {
		t.Fatalf("expected log_truncated marker, got:\n%s", content)
	}
	// 1 回だけであること
	count := strings.Count(content, `"log_truncated"`)
	if count != 1 {
		t.Fatalf("expected log_truncated exactly once, got %d times", count)
	}
}

// TestWriterSessionEndAfterTruncation は上限到達後も session_end が書かれることを確認する。
func TestWriterSessionEndAfterTruncation(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "end.jsonl")

	const maxBytes = 50
	w, err := NewJSONLWriter(path, maxBytes)
	if err != nil {
		t.Fatal(err)
	}

	// 上限を超えるイベント
	for i := 0; i < 5; i++ {
		_ = w.Event(map[string]string{"data": strings.Repeat("x", 20)})
	}
	// session_end は例外書き込みされるはず
	_ = w.Event(map[string]any{"type": "session_end", "state": "completed"})
	_ = w.Close()

	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(data), `"session_end"`) {
		t.Fatalf("expected session_end in log after truncation, got:\n%s", string(data))
	}
}

// TestMaskSecrets はマスキング関数のパターンテスト。
func TestMaskSecrets(t *testing.T) {
	cases := []struct {
		name  string
		input string
		want  string // 含まれてはいけない文字列
	}{
		{
			name:  "Anthropic API key",
			input: "ANTHROPIC_API_KEY=sk-ant-api03-ABCDEFGHIJKLMNOPQRSTUVWXYZ12345",
			want:  "sk-ant-api03-ABCDEFGHIJKLMNOPQRSTUVWXYZ12345",
		},
		{
			name:  "sk- token",
			input: "Authorization: Bearer sk-abcdefghijklmnopqrstuvwxyz123456",
			want:  "sk-abcdefghijklmnopqrstuvwxyz123456",
		},
		{
			name:  "GitHub PAT",
			input: "token: ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef",
			want:  "ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef",
		},
		{
			name:  "GitHub fine-grained PAT",
			input: "token: github_pat_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef123456",
			want:  "github_pat_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef123456",
		},
		{
			name:  "GitLab PAT",
			input: "token: glpat-ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef",
			want:  "glpat-ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef",
		},
		{
			name:  "Slack token",
			input: "token: xoxb-123456789012-abcdefghijklmnopqr",
			want:  "xoxb-123456789012-abcdefghijklmnopqr",
		},
		{
			name:  "Google API key",
			input: "token: AIzaABCDEFGHIJKLMNOPQRSTUVWXYZabc",
			want:  "AIzaABCDEFGHIJKLMNOPQRSTUVWXYZabc",
		},
		{
			name:  "Hugging Face token",
			input: "token: hf_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef",
			want:  "hf_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef",
		},
		{
			name:  "API_KEY env",
			input: "OPENAI_API_KEY=sk-testkey1234567890abcdef",
			want:  "sk-testkey1234567890abcdef",
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			got := MaskSecrets(tc.input)
			if strings.Contains(got, tc.want) {
				t.Fatalf("MaskSecrets(%q) = %q; still contains %q", tc.input, got, tc.want)
			}
		})
	}
}

// TestMaskSecretsPreservesNormal は通常テキストが変化しないことを確認する。
func TestMaskSecretsPreservesNormal(t *testing.T) {
	normal := "hello world, running go test ./..."
	if got := MaskSecrets(normal); got != normal {
		t.Fatalf("MaskSecrets changed normal text: %q -> %q", normal, got)
	}
}
