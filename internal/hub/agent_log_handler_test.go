package hub

import (
	"encoding/json"
	"os"
	"path/filepath"
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
	cwd := `D:\dev\github\public\many-ai-cli`
	dayDir := filepath.Join(codexHome, "sessions", "2026", "07", "20")
	want := writeCodexRollout(t, dayDir, "rollout-2026-07-20T11-12-25-abc.jsonl", cwd, "2026-07-20T11:12:25+09:00")
	// 別 cwd のセッション（誤って拾わないことを確認）。
	writeCodexRollout(t, dayDir, "rollout-2026-07-20T11-12-30-def.jsonl", `D:\dev\other`, "2026-07-20T11:12:30+09:00")

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
	cwd := `D:\dev\github\public\many-ai-cli`
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
	writeCodexRollout(t, dayDir, "rollout-2026-07-20T11-12-25-abc.jsonl", `D:\dev\other`, "2026-07-20T11:12:25+09:00")

	startedAt, err := time.Parse(time.RFC3339, "2026-07-20T11:12:24+09:00")
	if err != nil {
		t.Fatal(err)
	}
	if _, ok := findCodexRolloutLog(codexHome, `D:\dev\github\public\many-ai-cli`, startedAt); ok {
		t.Error("findCodexRolloutLog: expected no match for different cwd")
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
	cwd := `D:\dev\github\public\many-ai-cli`
	want := writeCopilotWorkspace(t, sessionStateDir, "d8c12c75-a3f5-4fb1-ade8-5c7fd586490b", cwd, "2026-07-20T11:12:25.000Z")
	writeCopilotWorkspace(t, sessionStateDir, "dd01451f-8555-4dfd-ad79-914492dd413b", `D:\dev\other`, "2026-07-20T11:12:30.000Z")

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
	cwd := `D:\dev\github\public\many-ai-cli`
	startedAt := time.Date(2026, 7, 20, 11, 12, 24, 0, time.UTC)
	want := writeCursorChatMeta(t, chatsDir, "1007b8f9fc7b983d40a7c18e93ca27bf", "5d8e09c5-4798-43e8-8c69-e1b5c4286032", cwd, startedAt.Add(1*time.Second).UnixMilli())
	writeCursorChatMeta(t, chatsDir, "62d283ad19d9e5c24511949f99953bbc", "31497bc0-3ebe-46e7-a418-822013f3a868", `D:\dev\other`, startedAt.Add(2*time.Second).UnixMilli())

	got, ok := findCursorChatDir(cursorHome, cwd, startedAt)
	if !ok {
		t.Fatal("findCursorChatDir: expected ok")
	}
	if got != want {
		t.Errorf("got %q, want %q", got, want)
	}
}
