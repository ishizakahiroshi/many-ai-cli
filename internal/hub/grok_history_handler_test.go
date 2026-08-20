package hub

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func testGrokUUIDV7(at time.Time) string {
	ms := at.UnixMilli()
	hexTS := ""
	for shift := 44; shift >= 0; shift -= 4 {
		hexTS += string("0123456789abcdef"[(ms>>shift)&0xf])
	}
	return hexTS[:8] + "-" + hexTS[8:12] + "-7000-8000-000000000000"
}

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

	mkSession := func(openedAt time.Time) string {
		id := testGrokUUIDV7(openedAt)
		d := filepath.Join(grokDir, "sessions", encoded, id)
		if err := os.MkdirAll(d, 0o700); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(d, "chat_history.jsonl"), []byte("{}\n"), 0o600); err != nil {
			t.Fatal(err)
		}
		return id
	}

	// 開始時刻から 8 秒後に作られたセッション（正解）と、1 時間前の別セッション
	wantID := mkSession(startedAt.Add(8 * time.Second))
	mkSession(startedAt.Add(-1 * time.Hour))

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

func TestFindGrokChatHistoryPrefersConversationOverCloserEmptyStub(t *testing.T) {
	grokDir := t.TempDir()
	cwd := filepath.Join(grokDir, "work", "proj")
	encoded := strings.NewReplacer(":", "%3A", "\\", "%5C", "/", "%2F").Replace(cwd)
	startedAt := time.Date(2026, 7, 31, 1, 0, 0, 0, time.UTC)
	writeSession := func(at time.Time, content string) string {
		id := testGrokUUIDV7(at)
		dir := filepath.Join(grokDir, "sessions", encoded, id)
		if err := os.MkdirAll(dir, 0o700); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(dir, "chat_history.jsonl"), []byte(content), 0o600); err != nil {
			t.Fatal(err)
		}
		return id
	}

	emptyID := writeSession(startedAt.Add(time.Second), "{}\n")
	conversationID := writeSession(startedAt.Add(3*time.Second),
		"{\"type\":\"user\",\"content\":\"<user_query>hello</user_query>\"}\n"+
			"{\"type\":\"assistant\",\"content\":\"world\"}\n")
	active, err := json.Marshal([]grokActiveSession{{
		SessionID: emptyID,
		CWD:       cwd,
		OpenedAt:  startedAt.Add(time.Second).Format(time.RFC3339Nano),
	}})
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(grokDir, "active_sessions.json"), active, 0o600); err != nil {
		t.Fatal(err)
	}

	got, ok := findGrokChatHistory(grokDir, cwd, startedAt)
	if !ok {
		t.Fatal("findGrokChatHistory: expected ok")
	}
	want := filepath.Join(grokDir, "sessions", encoded, conversationID, "chat_history.jsonl")
	if got != want {
		t.Fatalf("path = %s, want conversation path %s", got, want)
	}
}

func TestFindGrokChatHistoryUsesTimeAmongNonEmptyCandidates(t *testing.T) {
	grokDir := t.TempDir()
	cwd := filepath.Join(grokDir, "work", "proj")
	encoded := strings.NewReplacer(":", "%3A", "\\", "%5C", "/", "%2F").Replace(cwd)
	startedAt := time.Date(2026, 7, 31, 2, 0, 0, 0, time.UTC)
	writeConversation := func(at time.Time, answer string) string {
		id := testGrokUUIDV7(at)
		dir := filepath.Join(grokDir, "sessions", encoded, id)
		if err := os.MkdirAll(dir, 0o700); err != nil {
			t.Fatal(err)
		}
		content := "{\"type\":\"assistant\",\"content\":\"" + answer + "\"}\n"
		if err := os.WriteFile(filepath.Join(dir, "chat_history.jsonl"), []byte(content), 0o600); err != nil {
			t.Fatal(err)
		}
		return id
	}

	nearID := writeConversation(startedAt.Add(2*time.Second), "short")
	writeConversation(startedAt.Add(5*time.Second), "one")

	got, ok := findGrokChatHistory(grokDir, cwd, startedAt)
	if !ok {
		t.Fatal("findGrokChatHistory: expected ok")
	}
	want := filepath.Join(grokDir, "sessions", encoded, nearID, "chat_history.jsonl")
	if got != want {
		t.Fatalf("path = %s, want nearest non-empty path %s", got, want)
	}
}

func TestFindGrokChatHistoryAllEmptyKeepsActiveFallback(t *testing.T) {
	grokDir := t.TempDir()
	cwd := filepath.Join(grokDir, "work", "proj")
	encoded := strings.NewReplacer(":", "%3A", "\\", "%5C", "/", "%2F").Replace(cwd)
	startedAt := time.Date(2026, 7, 31, 3, 0, 0, 0, time.UTC)
	writeEmpty := func(at time.Time) string {
		id := testGrokUUIDV7(at)
		dir := filepath.Join(grokDir, "sessions", encoded, id)
		if err := os.MkdirAll(dir, 0o700); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(dir, "chat_history.jsonl"), []byte("{}\n"), 0o600); err != nil {
			t.Fatal(err)
		}
		return id
	}

	uuidNearID := writeEmpty(startedAt.Add(time.Second))
	activeID := writeEmpty(startedAt.Add(8 * time.Second))
	active, err := json.Marshal([]grokActiveSession{{
		SessionID: activeID,
		CWD:       cwd,
		OpenedAt:  startedAt.Add(8 * time.Second).Format(time.RFC3339Nano),
	}})
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(grokDir, "active_sessions.json"), active, 0o600); err != nil {
		t.Fatal(err)
	}

	got, ok := findGrokChatHistory(grokDir, cwd, startedAt)
	if !ok {
		t.Fatal("findGrokChatHistory: expected ok")
	}
	activePath := filepath.Join(grokDir, "sessions", encoded, activeID, "chat_history.jsonl")
	if got != activePath {
		t.Fatalf("path = %s, want legacy active fallback %s (near UUID was %s)", got, activePath, uuidNearID)
	}
}

func TestGrokHomeDir(t *testing.T) {
	custom := filepath.Join(t.TempDir(), "profile")
	user := filepath.Join(t.TempDir(), "user")
	if got := grokHomeDir(custom, user); got != custom {
		t.Errorf("GrokHome wins: got %q want %q", got, custom)
	}
	wantDefault := filepath.Join(user, ".grok")
	if got := grokHomeDir("", user); got != wantDefault {
		t.Errorf("empty GrokHome falls back: got %q want %q", got, wantDefault)
	}
	if got := grokHomeDir("  "+custom+"  ", user); got != custom {
		t.Errorf("trim GrokHome: got %q want %q", got, custom)
	}
	if got := grokHomeDir("  ", "  "); got != "" {
		t.Errorf("blank inputs: got %q, want empty", got)
	}
}

func grokHistoryTestLayout(t *testing.T, grokDir, cwd string, startedAt time.Time, assistantText string) {
	t.Helper()
	encoded := strings.NewReplacer(":", "%3A", "\\", "%5C", "/", "%2F").Replace(cwd)
	id := testGrokUUIDV7(startedAt.Add(8 * time.Second))
	dir := filepath.Join(grokDir, "sessions", encoded, id)
	if err := os.MkdirAll(dir, 0o700); err != nil {
		t.Fatal(err)
	}
	body := `{"type":"user","content":"<user_query>from test</user_query>"}` + "\n" +
		`{"type":"assistant","content":` + grokHistoryJSONQuote(assistantText) + `}` + "\n"
	if err := os.WriteFile(filepath.Join(dir, "chat_history.jsonl"), []byte(body), 0o600); err != nil {
		t.Fatal(err)
	}
}

func grokHistoryJSONQuote(s string) string {
	b, err := json.Marshal(s)
	if err != nil {
		return `""`
	}
	return string(b)
}

func TestHandleGrokHistoryUsesGrokHome(t *testing.T) {
	s := newTestServer()
	s.cfg.Token = "tok"
	userHome := t.TempDir()
	grokHome := t.TempDir()
	cwd := filepath.Join(userHome, "work", "proj")
	startedAt := time.Date(2026, 8, 19, 14, 36, 16, 0, time.UTC)
	grokHistoryTestLayout(t, grokHome, cwd, startedAt, "from grok home")
	grokHistoryTestLayout(t, filepath.Join(userHome, ".grok"), cwd, startedAt, "from default home")

	ses := registerTestSession(s, 10, "grok")
	s.sessionsMu.Lock()
	ses.CWD = cwd
	ses.StartedAt = startedAt.Format(time.RFC3339)
	ses.HomeDir = userHome
	ses.GrokHome = grokHome
	s.sessionsMu.Unlock()

	req := httptest.NewRequest(http.MethodGet, "/api/grok-history?token=tok&session_id=10", nil)
	req.Host = "127.0.0.1:47777"
	req.RemoteAddr = "127.0.0.1:12345"
	w := httptest.NewRecorder()
	s.handleGrokHistory(w, req)
	if w.Code != http.StatusOK {
		t.Fatalf("status = %d, body = %s", w.Code, w.Body.String())
	}
	body := w.Body.String()
	if !strings.Contains(body, "from grok home") {
		t.Fatalf("expected GrokHome history, body = %s", body)
	}
	if strings.Contains(body, "from default home") {
		t.Fatalf("picked ~/.grok instead of GrokHome, body = %s", body)
	}

	loc := s.agentLogForSession(10)
	if !loc.Available {
		t.Fatalf("agent log: %+v", loc)
	}
	if !strings.Contains(loc.Path, grokHome) {
		t.Fatalf("agent log path = %s, want under GrokHome %s", loc.Path, grokHome)
	}
}

func TestHandleGrokHistoryDoesNotFallBackToDefaultHome(t *testing.T) {
	s := newTestServer()
	s.cfg.Token = "tok"
	userHome := t.TempDir()
	grokHome := t.TempDir()
	cwd := filepath.Join(userHome, "work", "proj")
	startedAt := time.Date(2026, 8, 19, 14, 36, 16, 0, time.UTC)
	grokHistoryTestLayout(t, filepath.Join(userHome, ".grok"), cwd, startedAt, "from default home")

	ses := registerTestSession(s, 11, "grok")
	s.sessionsMu.Lock()
	ses.CWD = cwd
	ses.StartedAt = startedAt.Format(time.RFC3339)
	ses.HomeDir = userHome
	ses.GrokHome = grokHome
	s.sessionsMu.Unlock()

	req := httptest.NewRequest(http.MethodGet, "/api/grok-history?token=tok&session_id=11", nil)
	req.Host = "127.0.0.1:47777"
	req.RemoteAddr = "127.0.0.1:12345"
	w := httptest.NewRecorder()
	s.handleGrokHistory(w, req)
	if w.Code != http.StatusNotFound {
		t.Fatalf("status = %d, want 404 so the other profile is not used; body = %s", w.Code, w.Body.String())
	}
	if strings.Contains(w.Body.String(), "from default home") {
		t.Fatalf("fell back to ~/.grok, body = %s", w.Body.String())
	}
}
