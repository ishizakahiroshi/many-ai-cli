package hub

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func writeCodexRollout(t *testing.T, dir, name, cwd, timestamp string) string {
	t.Helper()
	if err := os.MkdirAll(dir, 0o700); err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(dir, name)
	meta := codexSessionMeta{Type: "session_meta"}
	meta.Payload.CWD = cwd
	meta.Payload.Timestamp = timestamp
	metaLine, err := json.Marshal(meta)
	if err != nil {
		t.Fatal(err)
	}
	content := string(metaLine) + "\n" + `{"type":"event_msg","payload":{"type":"token_count"}}` + "\n"
	if err := os.WriteFile(path, []byte(content), 0o600); err != nil {
		t.Fatal(err)
	}
	return path
}

func TestFindCodexRolloutLog(t *testing.T) {
	codexHome := t.TempDir()
	cwd := `C:\workspace\many-ai-cli`
	dayDir := filepath.Join(codexHome, "sessions", "2026", "07", "20")
	want := writeCodexRollout(t, dayDir, "rollout-2026-07-20T11-12-25-abc.jsonl", cwd, "2026-07-20T11:12:25+09:00")
	// 別 cwd のセッション（誤って拾わないことを確認）。
	writeCodexRollout(t, dayDir, "rollout-2026-07-20T11-12-30-def.jsonl", `C:\workspace\other`, "2026-07-20T11:12:30+09:00")

	startedAt, err := time.Parse(time.RFC3339, "2026-07-20T11:12:24+09:00")
	if err != nil {
		t.Fatal(err)
	}
	got, ok := findCodexRolloutLog(codexHome, cwd, startedAt)
	if !ok {
		t.Fatal("findCodexRolloutLog: expected ok")
	}
	if got != want {
		t.Errorf("got %q, want %q", got, want)
	}
}

func TestFindCodexRolloutLogDayBoundary(t *testing.T) {
	codexHome := t.TempDir()
	cwd := `C:\workspace\many-ai-cli`
	// セッション開始は 2026-07-20 23:59 だが rollout ファイルは日付を跨いで
	// 2026-07-21 の下に作られるケース。
	dayDir := filepath.Join(codexHome, "sessions", "2026", "07", "21")
	want := writeCodexRollout(t, dayDir, "rollout-2026-07-21T00-00-05-abc.jsonl", cwd, "2026-07-21T00:00:05+09:00")

	startedAt, err := time.Parse(time.RFC3339, "2026-07-20T23:59:58+09:00")
	if err != nil {
		t.Fatal(err)
	}
	got, ok := findCodexRolloutLog(codexHome, cwd, startedAt)
	if !ok {
		t.Fatal("findCodexRolloutLog: expected ok across day boundary")
	}
	if got != want {
		t.Errorf("got %q, want %q", got, want)
	}
}

func TestFindCodexRolloutLogNoMatch(t *testing.T) {
	codexHome := t.TempDir()
	dayDir := filepath.Join(codexHome, "sessions", "2026", "07", "20")
	writeCodexRollout(t, dayDir, "rollout-2026-07-20T11-12-25-abc.jsonl", `C:\workspace\other`, "2026-07-20T11:12:25+09:00")

	startedAt, err := time.Parse(time.RFC3339, "2026-07-20T11:12:24+09:00")
	if err != nil {
		t.Fatal(err)
	}
	if _, ok := findCodexRolloutLog(codexHome, `C:\workspace\many-ai-cli`, startedAt); ok {
		t.Error("findCodexRolloutLog: expected no match for different cwd")
	}
}

func TestFindCodexRolloutLogRefusesAmbiguousNearestMatches(t *testing.T) {
	codexHome := t.TempDir()
	cwd := `C:\workspace\many-ai-cli`
	dayDir := filepath.Join(codexHome, "sessions", "2026", "07", "20")
	writeCodexRollout(t, dayDir, "rollout-first.jsonl", cwd, "2026-07-20T11:12:24.100+09:00")
	writeCodexRollout(t, dayDir, "rollout-second.jsonl", cwd, "2026-07-20T11:12:24.800+09:00")

	startedAt, err := time.Parse(time.RFC3339, "2026-07-20T11:12:24+09:00")
	if err != nil {
		t.Fatal(err)
	}
	if got, ok := findCodexRolloutLog(codexHome, cwd, startedAt); ok {
		t.Fatalf("findCodexRolloutLog = %q, true; want ambiguous match to fail closed", got)
	}
}

func writeCopilotWorkspace(t *testing.T, sessionStateDir, uuid, cwd, createdAt string) string {
	t.Helper()
	dir := filepath.Join(sessionStateDir, uuid)
	if err := os.MkdirAll(dir, 0o700); err != nil {
		t.Fatal(err)
	}
	doc := "id: " + uuid + "\ncwd: " + cwd + "\ncreated_at: " + createdAt + "\n"
	if err := os.WriteFile(filepath.Join(dir, "workspace.yaml"), []byte(doc), 0o600); err != nil {
		t.Fatal(err)
	}
	return dir
}

func TestFindCopilotSessionState(t *testing.T) {
	copilotHome := t.TempDir()
	sessionStateDir := filepath.Join(copilotHome, "session-state")
	cwd := `C:\workspace\many-ai-cli`
	want := writeCopilotWorkspace(t, sessionStateDir, "d8c12c75-a3f5-4fb1-ade8-5c7fd586490b", cwd, "2026-07-20T11:12:25.000Z")
	writeCopilotWorkspace(t, sessionStateDir, "dd01451f-8555-4dfd-ad79-914492dd413b", `C:\workspace\other`, "2026-07-20T11:12:30.000Z")

	startedAt, err := time.Parse(time.RFC3339, "2026-07-20T11:12:24Z")
	if err != nil {
		t.Fatal(err)
	}
	got, ok := findCopilotSessionState(copilotHome, cwd, startedAt)
	if !ok {
		t.Fatal("findCopilotSessionState: expected ok")
	}
	if got != want {
		t.Errorf("got %q, want %q", got, want)
	}
}

func writeCursorChatMeta(t *testing.T, chatsDir, hash, uuid, cwd string, createdAtMs int64) string {
	t.Helper()
	dir := filepath.Join(chatsDir, hash, uuid)
	if err := os.MkdirAll(dir, 0o700); err != nil {
		t.Fatal(err)
	}
	meta := cursorChatMeta{CWD: cwd, CreatedAtMs: createdAtMs}
	data, err := json.Marshal(meta)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, "meta.json"), data, 0o600); err != nil {
		t.Fatal(err)
	}
	return dir
}

func TestFindCursorChatDir(t *testing.T) {
	cursorHome := t.TempDir()
	chatsDir := filepath.Join(cursorHome, "chats")
	cwd := `C:\workspace\many-ai-cli`
	startedAt := time.Date(2026, 7, 20, 11, 12, 24, 0, time.UTC)
	want := writeCursorChatMeta(t, chatsDir, "1007b8f9fc7b983d40a7c18e93ca27bf", "5d8e09c5-4798-43e8-8c69-e1b5c4286032", cwd, startedAt.Add(1*time.Second).UnixMilli())
	writeCursorChatMeta(t, chatsDir, "62d283ad19d9e5c24511949f99953bbc", "31497bc0-3ebe-46e7-a418-822013f3a868", `C:\workspace\other`, startedAt.Add(2*time.Second).UnixMilli())

	got, ok := findCursorChatDir(cursorHome, cwd, startedAt)
	if !ok {
		t.Fatal("findCursorChatDir: expected ok")
	}
	if got != want {
		t.Errorf("got %q, want %q", got, want)
	}
}

func writeClaudeTranscript(t *testing.T, dir, name, cwd, timestamp string) string {
	t.Helper()
	if err := os.MkdirAll(dir, 0o700); err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(dir, name)
	meta := map[string]any{"type": "user", "sessionId": strings.TrimSuffix(name, ".jsonl"), "cwd": cwd, "timestamp": timestamp}
	data, err := json.Marshal(meta)
	if err != nil {
		t.Fatal(err)
	}
	assistant, err := json.Marshal(map[string]any{
		"type": "assistant", "timestamp": timestamp,
		"message": map[string]any{"role": "assistant", "content": []any{map[string]any{"type": "text", "text": "fixture"}}},
	})
	if err != nil {
		t.Fatal(err)
	}
	content := append(data, '\n')
	content = append(content, assistant...)
	content = append(content, '\n')
	if err := os.WriteFile(path, content, 0o600); err != nil {
		t.Fatal(err)
	}
	return path
}

func TestClaudeTranscriptPathUsesSessionID(t *testing.T) {
	claudeDir := t.TempDir()
	cwd := `C:\workspace\many-ai-cli`
	projectDir := filepath.Join(claudeDir, "projects", claudeProjectDirName(cwd))
	id := "123e4567-e89b-42d3-a456-426614174000"
	want := writeClaudeTranscript(t, projectDir, id+".jsonl", cwd, "2026-07-20T11:12:25Z")
	got, ok := claudeTranscriptPath(claudeDir, cwd, id)
	if !ok || got != want {
		t.Fatalf("claudeTranscriptPath = %q, %v; want %q, true", got, ok, want)
	}
	if _, ok := claudeTranscriptPath(claudeDir, cwd, "../../other"); ok {
		t.Fatal("unsafe session ID was accepted")
	}
}

func TestFindClaudeTranscriptChoosesNearestAndRefusesTie(t *testing.T) {
	claudeDir := t.TempDir()
	cwd := `C:\workspace\many-ai-cli`
	projectDir := filepath.Join(claudeDir, "projects", claudeProjectDirName(cwd))
	startedAt, err := time.Parse(time.RFC3339, "2026-07-20T11:12:24Z")
	if err != nil {
		t.Fatal(err)
	}
	want := writeClaudeTranscript(t, projectDir, "near.jsonl", cwd, "2026-07-20T11:12:25Z")
	writeClaudeTranscript(t, projectDir, "far.jsonl", cwd, "2026-07-20T11:13:00Z")
	got, ok := findClaudeTranscript(claudeDir, cwd, startedAt)
	if !ok || got != want {
		t.Fatalf("findClaudeTranscript = %q, %v; want nearest %q", got, ok, want)
	}

	tieDir := t.TempDir()
	tieProjectDir := filepath.Join(tieDir, "projects", claudeProjectDirName(cwd))
	writeClaudeTranscript(t, tieProjectDir, "a.jsonl", cwd, "2026-07-20T11:12:23Z")
	writeClaudeTranscript(t, tieProjectDir, "b.jsonl", cwd, "2026-07-20T11:12:25Z")
	if _, ok := findClaudeTranscript(tieDir, cwd, startedAt); ok {
		t.Fatal("equal-time candidates should not be guessed")
	}
}

func TestCommandCodeProjectSlug(t *testing.T) {
	cases := []struct {
		cwd  string
		want string
	}{
		{`D:\tmp\cc-capture\workspace`, "d-tmp-cc-capture-workspace"},
		{`D:\tmp\cc-capture\fresh-trust`, "d-tmp-cc-capture-fresh-trust"},
		{`C:\home\alice`, "c-home-alice"},
		{`E:\src\github\public\sample-cli`, "e-src-github-public-sample-cli"},
		{`D:/tmp/cc-capture/workspace`, "d-tmp-cc-capture-workspace"},
		{"", "root"},
	}
	for _, tc := range cases {
		if got := commandCodeProjectSlug(tc.cwd); got != tc.want {
			t.Errorf("commandCodeProjectSlug(%q) = %q, want %q", tc.cwd, got, tc.want)
		}
	}
}

func writeCommandCodeTranscript(t *testing.T, dir, name, id, cwd, timestamp string) string {
	t.Helper()
	if err := os.MkdirAll(dir, 0o700); err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(dir, name)
	hdr := commandCodeSessionHeader{Type: "session", ID: id, Timestamp: timestamp, CWD: cwd}
	line, err := json.Marshal(hdr)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, append(line, '\n'), 0o600); err != nil {
		t.Fatal(err)
	}
	return path
}

func TestFindCommandCodeTranscript(t *testing.T) {
	home := t.TempDir()
	cwd := `D:\tmp\cc-capture\workspace`
	dir := commandCodeProjectDir(home, cwd)
	wantID := "dafa1f84-9ed3-439e-a30f-6615b26c4423"
	want := writeCommandCodeTranscript(t, dir, wantID+".jsonl", wantID, cwd, "2026-09-12T16:43:31.897Z")
	writeCommandCodeTranscript(t, dir, "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb.jsonl", "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb", `D:\tmp\other`, "2026-09-12T16:43:31.897Z")

	startedAt, err := time.Parse(time.RFC3339, "2026-09-12T16:43:32Z")
	if err != nil {
		t.Fatal(err)
	}
	got, ok := findCommandCodeTranscript(home, cwd, startedAt)
	if !ok || got != want {
		t.Fatalf("findCommandCodeTranscript = %q, %v; want %q, true", got, ok, want)
	}
}

func TestAgentLogForSessionCommandCode(t *testing.T) {
	s := newTestServer()
	home := t.TempDir()
	cwd := `D:\tmp\cc-capture\workspace`
	id := "dafa1f84-9ed3-439e-a30f-6615b26c4423"
	dir := commandCodeProjectDir(home, cwd)
	want := writeCommandCodeTranscript(t, dir, id+".jsonl", id, cwd, "2026-09-12T16:43:31.897Z")

	ses := registerTestSession(s, 6, "command-code")
	s.sessionsMu.Lock()
	ses.CWD = cwd
	ses.HomeDir = home
	ses.StartedAt = "2026-09-12T16:43:32Z"
	s.sessionsMu.Unlock()

	loc := s.agentLogForSession(6)
	if !loc.Available {
		t.Fatalf("agent log: %+v", loc)
	}
	if loc.Path != want {
		t.Fatalf("path = %q, want %q", loc.Path, want)
	}
	if loc.Label != "Command Code session transcript" {
		t.Fatalf("label = %q", loc.Label)
	}

	s.sessionsMu.Lock()
	ses.AgentSessionID = id
	s.sessionsMu.Unlock()
	loc = s.agentLogForSession(6)
	if !loc.Available || loc.Path != want {
		t.Fatalf("by AgentSessionID: %+v", loc)
	}
}

func TestAgentLogForSessionCommandCodeFallsBackToProjectDir(t *testing.T) {
	s := newTestServer()
	home := t.TempDir()
	cwd := `D:\tmp\cc-capture\workspace`
	dir := commandCodeProjectDir(home, cwd)
	if err := os.MkdirAll(dir, 0o700); err != nil {
		t.Fatal(err)
	}

	ses := registerTestSession(s, 6, "command-code")
	s.sessionsMu.Lock()
	ses.CWD = cwd
	ses.HomeDir = home
	s.sessionsMu.Unlock()

	loc := s.agentLogForSession(6)
	if !loc.Available {
		t.Fatalf("agent log: %+v", loc)
	}
	if loc.Path != dir {
		t.Fatalf("path = %q, want project dir %q", loc.Path, dir)
	}
}
