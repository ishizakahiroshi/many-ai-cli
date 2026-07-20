package hub

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func TestUUIDV7Time(t *testing.T) {
	// 0x018f00000000 ms = 2024-04-23T16:03:53.6Z 近傍（値そのものより往復一致を確認）
	ts, ok := uuidV7Time("018f0000-0000-7000-8000-000000000000")
	if !ok {
		t.Fatal("uuidV7Time: expected ok")
	}
	if got := ts.UnixMilli(); got != 0x018f00000000 {
		t.Errorf("UnixMilli = %d, want %d", got, int64(0x018f00000000))
	}
	// version ニブルが 7 以外は不採用
	if _, ok := uuidV7Time("018f0000-0000-4000-8000-000000000000"); ok {
		t.Error("uuidV7Time: UUIDv4 should not be accepted")
	}
	if _, ok := uuidV7Time("not-a-uuid"); ok {
		t.Error("uuidV7Time: malformed id should not be accepted")
	}
}

func TestExtractGrokUserQuery(t *testing.T) {
	q, ok := extractGrokUserQuery("<user_query>\nhello world\n</user_query>")
	if !ok || q != "hello world" {
		t.Errorf("got %q ok=%v, want %q", q, ok, "hello world")
	}
	// 閉じタグが無い（書き込み途中）場合は末尾までを採用
	q, ok = extractGrokUserQuery("<user_query>partial input")
	if !ok || q != "partial input" {
		t.Errorf("got %q ok=%v, want %q", q, ok, "partial input")
	}
	// タグ無し（<user_info> 等の環境ブロック）は対象外
	if _, ok := extractGrokUserQuery("<user_info>\nOS: test\n</user_info>"); ok {
		t.Error("user_info block should not be extracted")
	}
}

func TestReadGrokChatHistory(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "chat_history.jsonl")
	lines := `{"type":"system","content":"synthetic system prompt"}
{"type":"user","content":"<user_info>\nOS: test\n</user_info>"}
{"type":"user","synthetic_reason":"system_reminder","content":"<system-reminder>synthetic</system-reminder>"}
{"type":"user","content":[{"type":"text","text":"<user_query>\nfirst question\n</user_query>"}]}
{"type":"reasoning","encrypted_content":"zzzz"}
{"type":"assistant","content":"first answer"}
{"type":"tool_result","content":"synthetic tool output"}
{"type":"user","content":[{"type":"text","text":"<user_query>second question</user_query>"},{"type":"image","image_url":"x"}]}
{"type":"assistant","content":"second answer"}
`
	if err := os.WriteFile(path, []byte(lines), 0o600); err != nil {
		t.Fatal(err)
	}
	msgs, err := readGrokChatHistory(path)
	if err != nil {
		t.Fatalf("readGrokChatHistory: %v", err)
	}
	want := []grokChatMessage{
		{Role: "user", Text: "first question"},
		{Role: "assistant", Text: "first answer"},
		{Role: "user", Text: "second question"},
		{Role: "assistant", Text: "second answer"},
	}
	if len(msgs) != len(want) {
		t.Fatalf("len = %d, want %d (%+v)", len(msgs), len(want), msgs)
	}
	for i := range want {
		if msgs[i] != want[i] {
			t.Errorf("msgs[%d] = %+v, want %+v", i, msgs[i], want[i])
		}
	}
}

func TestFindGrokChatHistory(t *testing.T) {
	grokDir := t.TempDir()
	cwd := filepath.Join(grokDir, "work", "proj")
	// Grok 実装と同様に cwd をパーセントエンコードしたディレクトリ名を作る
	// （実物は : と \ をエンコードする。url.PathEscape は : を素通しして
	// Windows で不正なディレクトリ名になるため手で組む）
	encoded := strings.NewReplacer(":", "%3A", "\\", "%5C", "/", "%2F").Replace(cwd)
	startedAt := time.Date(2026, 7, 9, 1, 30, 0, 0, time.UTC)

	mkSession := func(openedAt time.Time, history string) string {
		ms := openedAt.UnixMilli()
		hexTS := ""
		for shift := 44; shift >= 0; shift -= 4 {
			hexTS += string("0123456789abcdef"[(ms>>shift)&0xf])
		}
		id := hexTS[:8] + "-" + hexTS[8:12] + "-7000-8000-000000000000"
		d := filepath.Join(grokDir, "sessions", encoded, id)
		if err := os.MkdirAll(d, 0o700); err != nil {
			t.Fatal(err)
		}
		if history == "" {
			history = "{}\n"
		}
		if err := os.WriteFile(filepath.Join(d, "chat_history.jsonl"), []byte(history), 0o600); err != nil {
			t.Fatal(err)
		}
		return id
	}

	// 開始時刻から 8 秒後に作られたセッション（正解）と、1 時間前の別セッション
	wantID := mkSession(startedAt.Add(8*time.Second), "")
	mkSession(startedAt.Add(-1*time.Hour), "")

	got, ok := findGrokChatHistory(grokDir, cwd, startedAt)
	if !ok {
		t.Fatal("findGrokChatHistory: expected ok")
	}
	if want := filepath.Join(grokDir, "sessions", encoded, wantID, "chat_history.jsonl"); got != want {
		t.Errorf("path = %s, want %s", got, want)
	}

	// 対応する cwd ディレクトリが無い場合は不一致
	if _, ok := findGrokChatHistory(grokDir, filepath.Join(grokDir, "other"), startedAt); ok {
		t.Error("expected not found for unknown cwd")
	}

	// 時刻が許容窓（grokHistoryMatchWindow）を超えて離れている場合は不一致
	if _, ok := findGrokChatHistory(grokDir, cwd, startedAt.Add(2*time.Hour)); ok {
		t.Error("expected not found for start time outside match window")
	}
}

// TestFindGrokChatHistoryPrefersNonEmpty は「Hub 開始直後の空 stub」と
// 「少し後に会話が入った Grok session」が並立するとき、空 stub を捨てて
// 表示可能な会話がある側を選ぶことを検証する（↑ ボタン空表示の回帰）。
func TestFindGrokChatHistoryPrefersNonEmpty(t *testing.T) {
	grokDir := t.TempDir()
	cwd := filepath.Join(grokDir, "work", "proj")
	encoded := strings.NewReplacer(":", "%3A", "\\", "%5C", "/", "%2F").Replace(cwd)
	startedAt := time.Date(2026, 7, 18, 12, 54, 33, 0, time.UTC)

	mkSession := func(id string, openedAt time.Time, history string) {
		d := filepath.Join(grokDir, "sessions", encoded, id)
		if err := os.MkdirAll(d, 0o700); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(d, "chat_history.jsonl"), []byte(history), 0o600); err != nil {
			t.Fatal(err)
		}
		_ = openedAt
	}

	// Stub closer to hub start (system + synthetic only → 0 displayable).
	stubID := "019f754b-9db4-79a3-b7ab-06915d9167d2"
	stubOpened := startedAt.Add(44 * time.Second)
	stubHist := `{"type":"system","content":"You are Grok"}
{"type":"user","synthetic_reason":"system_reminder","content":[{"type":"text","text":"<system-reminder>x</system-reminder>"}]}
`
	mkSession(stubID, stubOpened, stubHist)

	// Real conversation farther in time but still inside the 10m window.
	richID := "019f68ac-51a4-78d1-a5d2-d6e6b0dc1fae"
	richOpened := startedAt.Add(8 * time.Minute)
	richHist := `{"type":"user","content":[{"type":"text","text":"<user_query>\nhello\n</user_query>"}]}
{"type":"assistant","content":"hi there"}
`
	mkSession(richID, richOpened, richHist)

	// active_sessions lists stub first (closer) — old logic would pick stub.
	cwdJSON, err := json.Marshal(cwd)
	if err != nil {
		t.Fatal(err)
	}
	actives := `[
  {"session_id":"` + stubID + `","cwd":` + string(cwdJSON) + `,"opened_at":"` + stubOpened.Format(time.RFC3339Nano) + `"},
  {"session_id":"` + richID + `","cwd":` + string(cwdJSON) + `,"opened_at":"` + richOpened.Format(time.RFC3339Nano) + `"}
]`
	if err := os.WriteFile(filepath.Join(grokDir, "active_sessions.json"), []byte(actives), 0o600); err != nil {
		t.Fatal(err)
	}

	got, ok := findGrokChatHistory(grokDir, cwd, startedAt)
	if !ok {
		t.Fatal("findGrokChatHistory: expected ok")
	}
	want := filepath.Join(grokDir, "sessions", encoded, richID, "chat_history.jsonl")
	if got != want {
		t.Fatalf("path = %s\nwant %s (should prefer non-empty conversation over closer stub)", got, want)
	}
}
