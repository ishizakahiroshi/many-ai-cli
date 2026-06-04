package sessionstore

import (
	"path/filepath"
	"strconv"
	"strings"
	"testing"
	"time"
)

func TestStoreChatRestoreSearchAndPrune(t *testing.T) {
	logDir := filepath.Join(t.TempDir(), "logs")
	store, err := OpenForLogDir(logDir)
	if err != nil {
		t.Fatalf("OpenForLogDir: %v", err)
	}
	defer store.Close()

	startedAt := time.Now().Add(-2 * time.Hour).Format(time.RFC3339)
	if _, err := store.StartSession(SessionStart{
		LiveSessionID: 1,
		Provider:      "codex",
		CWD:           filepath.Join("C:", "dev", "any-ai-cli"),
		Branch:        "develop",
		State:         "standby",
		StartedAt:     startedAt,
		LogPath:       filepath.Join(logDir, "sessions", "s1.log"),
		JSONLPath:     filepath.Join(logDir, "sessions", "s1.jsonl"),
	}); err != nil {
		t.Fatalf("StartSession: %v", err)
	}

	events := []map[string]any{
		{"ts": startedAt, "type": "session_start", "session_id": 1, "provider": "codex"},
		{"ts": startedAt, "type": "user_input", "session_id": 1, "text": "調査して\r"},
		{"ts": startedAt, "type": "pty_output", "session_id": 1, "text": "SQLite retention search result\n"},
		{"ts": startedAt, "type": "pty_output", "session_id": 1, "text": "second chunk\n"},
	}
	for _, ev := range events {
		if err := store.StoreEvent(1, ev); err != nil {
			t.Fatalf("StoreEvent(%s): %v", ev["type"], err)
		}
	}

	msgs, err := store.ChatMessagesByLiveSession(1, 100)
	if err != nil {
		t.Fatalf("ChatMessagesByLiveSession: %v", err)
	}
	if len(msgs) != 2 {
		t.Fatalf("messages len = %d, want 2: %#v", len(msgs), msgs)
	}
	if msgs[0].Role != "user" || !strings.Contains(msgs[0].RawText, "調査して") {
		t.Fatalf("first message = %#v", msgs[0])
	}
	if msgs[1].Role != "ai" || !strings.Contains(msgs[1].RawText, "second chunk") {
		t.Fatalf("ai message = %#v", msgs[1])
	}

	results, err := store.SearchMessages("retention", 10)
	if err != nil {
		t.Fatalf("SearchMessages: %v", err)
	}
	if len(results) == 0 || !strings.Contains(results[0].Text, "retention") {
		t.Fatalf("search results = %#v", results)
	}

	meta, err := store.UpdateSessionMeta(1, "SQLite work", []string{"sqlite", "retention", "sqlite"}, "summary text", false)
	if err != nil {
		t.Fatalf("UpdateSessionMeta: %v", err)
	}
	if meta.Title != "SQLite work" || meta.Summary != "summary text" || len(meta.Tags) != 2 {
		t.Fatalf("meta = %#v", meta)
	}
	timeline, err := store.TimelineByLiveSession(1, 10)
	if err != nil {
		t.Fatalf("TimelineByLiveSession: %v", err)
	}
	if len(timeline) != len(events) || timeline[1].Type != "user_input" {
		t.Fatalf("timeline = %#v", timeline)
	}
	list, err := store.ListSessions(10, false)
	if err != nil {
		t.Fatalf("ListSessions: %v", err)
	}
	if len(list) != 1 || list[0].MessageCount != 3 || list[0].EventCount != len(events) {
		t.Fatalf("session list = %#v", list)
	}
	usage, err := store.UsageSummary()
	if err != nil {
		t.Fatalf("UsageSummary: %v", err)
	}
	if usage.TotalSessions != 1 || usage.TotalMessages != 3 {
		t.Fatalf("usage = %#v", usage)
	}

	store.EndSession(1, "completed", "", time.Now().Add(-time.Hour))
	stale, err := store.StaleSessions(time.Now().Add(1*time.Hour), 10)
	if err != nil {
		t.Fatalf("StaleSessions: %v", err)
	}
	if len(stale) != 1 {
		t.Fatalf("stale = %#v", stale)
	}
	if err := store.PruneOlderThan(time.Now().Add(24 * time.Hour)); err != nil {
		t.Fatalf("PruneOlderThan: %v", err)
	}
	msgs, err = store.ChatMessagesByLiveSession(1, 100)
	if err != nil {
		t.Fatalf("ChatMessagesByLiveSession after prune: %v", err)
	}
	if len(msgs) != 0 {
		t.Fatalf("messages after prune len = %d, want 0", len(msgs))
	}
}

func TestResetHistoryPreservesActiveSessionRows(t *testing.T) {
	logDir := filepath.Join(t.TempDir(), "logs")
	store, err := OpenForLogDir(logDir)
	if err != nil {
		t.Fatalf("OpenForLogDir: %v", err)
	}
	defer store.Close()

	startedAt := time.Now().Format(time.RFC3339)
	for _, liveID := range []int{1, 2} {
		if _, err := store.StartSession(SessionStart{
			LiveSessionID: liveID,
			Provider:      "codex",
			State:         "standby",
			StartedAt:     startedAt,
			LogPath:       filepath.Join(logDir, "sessions", "s.log"),
			JSONLPath:     filepath.Join(logDir, "sessions", "s"+strconv.Itoa(liveID)+".jsonl"),
		}); err != nil {
			t.Fatalf("StartSession(%d): %v", liveID, err)
		}
		if err := store.StoreEvent(liveID, map[string]any{"ts": startedAt, "type": "user_input", "session_id": liveID, "text": "hello"}); err != nil {
			t.Fatalf("StoreEvent(%d): %v", liveID, err)
		}
	}
	store.EndSession(2, "completed", "", time.Now())

	result, err := store.ResetHistory([]int{1})
	if err != nil {
		t.Fatalf("ResetHistory: %v", err)
	}
	if result.Sessions != 2 || result.Messages != 2 || result.Preserved != 1 {
		t.Fatalf("reset result = %#v", result)
	}
	list, err := store.ListSessions(10, true)
	if err != nil {
		t.Fatalf("ListSessions: %v", err)
	}
	if len(list) != 1 || list[0].LiveSessionID != 1 || list[0].MessageCount != 0 {
		t.Fatalf("sessions after reset = %#v", list)
	}
	if err := store.StoreEvent(1, map[string]any{"ts": startedAt, "type": "user_input", "session_id": 1, "text": "after reset"}); err != nil {
		t.Fatalf("StoreEvent after reset: %v", err)
	}
	msgs, err := store.ChatMessagesByLiveSession(1, 10)
	if err != nil {
		t.Fatalf("ChatMessagesByLiveSession: %v", err)
	}
	if len(msgs) != 1 || !strings.Contains(msgs[0].RawText, "after reset") {
		t.Fatalf("messages after reset = %#v", msgs)
	}
}

// Hub クラッシュで未終了のまま残った行が、次回 run の同じ live_session_id の
// 新セッションに上書きされないこと（CloseStaleSessions）と、reattach 時の
// StartSession upsert が ended_at を NULL に戻して行を継続利用できることを検証する。
func TestCloseStaleSessionsAndReattachRevive(t *testing.T) {
	logDir := filepath.Join(t.TempDir(), "logs")
	store, err := OpenForLogDir(logDir)
	if err != nil {
		t.Fatalf("OpenForLogDir: %v", err)
	}
	defer store.Close()

	startedAt := time.Now().Add(-time.Hour).Format(time.RFC3339)
	// 前回 run: live 1 のセッションがクラッシュで EndSession されずに残る
	oldID, err := store.StartSession(SessionStart{
		LiveSessionID: 1,
		Provider:      "claude",
		State:         "running",
		StartedAt:     startedAt,
		JSONLPath:     filepath.Join(logDir, "sessions", "old_run_s1.jsonl"),
	})
	if err != nil {
		t.Fatalf("StartSession(old): %v", err)
	}
	store.UpdateSessionMessages(1, "古いセッションの最初の入力", "古いセッションの最後の出力")

	// 次回 run 起動: 未終了行を一括クローズ
	n, err := store.CloseStaleSessions(time.Now(), "hub_restart")
	if err != nil {
		t.Fatalf("CloseStaleSessions: %v", err)
	}
	if n != 1 {
		t.Fatalf("CloseStaleSessions closed = %d, want 1", n)
	}

	// 次回 run: 同じ live 1 を別セッションが再利用
	newID, err := store.StartSession(SessionStart{
		LiveSessionID: 1,
		Provider:      "codex",
		State:         "standby",
		StartedAt:     time.Now().Format(time.RFC3339),
		JSONLPath:     filepath.Join(logDir, "sessions", "new_run_s1.jsonl"),
	})
	if err != nil {
		t.Fatalf("StartSession(new): %v", err)
	}
	if newID == oldID {
		t.Fatalf("new session reused old row id %d", oldID)
	}
	store.UpdateSessionMessages(1, "新しいセッションの入力", "新しいセッションの出力")
	store.UpdateSessionState(1, "running", time.Now().Format(time.RFC3339))

	list, err := store.ListSessions(10, true)
	if err != nil {
		t.Fatalf("ListSessions: %v", err)
	}
	if len(list) != 2 {
		t.Fatalf("sessions len = %d, want 2: %#v", len(list), list)
	}
	for _, it := range list {
		switch it.ID {
		case oldID:
			if it.FirstMessage != "古いセッションの最初の入力" || it.LastMessage != "古いセッションの最後の出力" {
				t.Fatalf("old row contaminated by new session: %#v", it)
			}
			if it.State != "disconnected" || it.EndReason != "hub_restart" || it.EndedAt == "" {
				t.Fatalf("old row not closed: %#v", it)
			}
		case newID:
			if it.FirstMessage != "新しいセッションの入力" || it.State != "running" {
				t.Fatalf("new row = %#v", it)
			}
		default:
			t.Fatalf("unexpected row: %#v", it)
		}
	}

	// reattach: 同じ jsonl_path での StartSession upsert が ended_at を解除して行を復活させる
	store.EndSession(1, "disconnected", "hub_lost", time.Now())
	revivedID, err := store.StartSession(SessionStart{
		LiveSessionID: 1,
		Provider:      "codex",
		State:         "running",
		StartedAt:     time.Now().Format(time.RFC3339),
		JSONLPath:     filepath.Join(logDir, "sessions", "new_run_s1.jsonl"),
	})
	if err != nil {
		t.Fatalf("StartSession(reattach): %v", err)
	}
	if revivedID != newID {
		t.Fatalf("reattach row id = %d, want %d", revivedID, newID)
	}
	store.UpdateSessionState(1, "waiting", time.Now().Format(time.RFC3339))
	ov, err := store.SessionOverviewByLiveSession(1)
	if err != nil {
		t.Fatalf("SessionOverviewByLiveSession: %v", err)
	}
	if ov.ID != newID || ov.EndedAt != "" || ov.State != "waiting" {
		t.Fatalf("revived row = %#v", ov)
	}
}
