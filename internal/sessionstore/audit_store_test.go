package sessionstore

import (
	"context"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// startAuditSession は audit テスト用に live セッションを 1 つ作って Store を返す。
func startAuditSession(t *testing.T, liveID int) *Store {
	t.Helper()
	logDir := filepath.Join(t.TempDir(), "logs")
	store, err := OpenForLogDir(logDir)
	if err != nil {
		t.Fatalf("OpenForLogDir: %v", err)
	}
	t.Cleanup(func() { store.Close() })
	if _, err := store.StartSession(SessionStart{
		LiveSessionID: liveID,
		Provider:      "claude",
		CWD:           t.TempDir(),
		State:         "standby",
		StartedAt:     time.Now().Format(time.RFC3339),
	}); err != nil {
		t.Fatalf("StartSession: %v", err)
	}
	return store
}

func auditCountRows(t *testing.T, store *Store, query string, args ...any) int {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), defaultTimeout)
	defer cancel()
	var n int
	if err := store.db.QueryRowContext(ctx, query, args...).Scan(&n); err != nil {
		t.Fatalf("count query %q: %v", query, err)
	}
	return n
}

// TestMessagesMentionText_UserOnly は read-only バイパスの言及照合が role='user' に
// 限定され、AI 出力（pty_output / role='ai'）一致では許可されないことを確認する。
func TestMessagesMentionText_UserOnly(t *testing.T) {
	store := startAuditSession(t, 1)
	ts := time.Now().Format(time.RFC3339)

	// AI 出力にのみ現れるパス（インジェクション想定）
	aiPath := filepath.Join("C:", "Users", "victim", ".ssh", "id_rsa")
	if err := store.StoreEvent(1, map[string]any{"ts": ts, "type": "pty_output", "session_id": 1, "text": "open " + aiPath + " now"}); err != nil {
		t.Fatalf("StoreEvent(pty_output): %v", err)
	}
	if ok, err := store.MessagesMentionText(1, []string{aiPath}); err != nil || ok {
		t.Fatalf("AI-only mention: got ok=%v err=%v, want ok=false", ok, err)
	}

	// ユーザー入力に現れるパスは許可される
	userPath := filepath.Join("C:", "Users", "me", "project", "plan.md")
	if err := store.StoreEvent(1, map[string]any{"ts": ts, "type": "user_input", "session_id": 1, "text": "look at " + userPath}); err != nil {
		t.Fatalf("StoreEvent(user_input): %v", err)
	}
	if ok, err := store.MessagesMentionText(1, []string{userPath}); err != nil || !ok {
		t.Fatalf("user mention: got ok=%v err=%v, want ok=true", ok, err)
	}
}

// TestClearSessionHistory_DeletesAttachments は per-session 履歴クリアで
// attachments 行も削除され孤児が残らないことを確認する（sessions 行は保持）。
func TestClearSessionHistory_DeletesAttachments(t *testing.T) {
	store := startAuditSession(t, 2)
	ts := time.Now().Format(time.RFC3339)

	if err := store.StoreEvent(2, map[string]any{
		"ts": ts, "type": "attach", "session_id": 2,
		"path": filepath.Join("C:", "tmp", "shot.png"), "filename": "shot.png",
	}); err != nil {
		t.Fatalf("StoreEvent(attach): %v", err)
	}
	if err := store.StoreEvent(2, map[string]any{"ts": ts, "type": "user_input", "session_id": 2, "text": "hi"}); err != nil {
		t.Fatalf("StoreEvent(user_input): %v", err)
	}

	sid, err := store.sessionIDForLive(2)
	if err != nil || sid == 0 {
		t.Fatalf("sessionIDForLive: sid=%d err=%v", sid, err)
	}
	if n := auditCountRows(t, store, `SELECT COUNT(*) FROM attachments WHERE session_id=?`, sid); n != 1 {
		t.Fatalf("attachments before clear = %d, want 1", n)
	}

	store.ClearSessionHistory(2)

	if n := auditCountRows(t, store, `SELECT COUNT(*) FROM attachments WHERE session_id=?`, sid); n != 0 {
		t.Fatalf("attachments after clear = %d, want 0 (orphan rows remained)", n)
	}
	if n := auditCountRows(t, store, `SELECT COUNT(*) FROM messages WHERE session_id=?`, sid); n != 0 {
		t.Fatalf("messages after clear = %d, want 0", n)
	}
	// sessions 行は保持される（per-session クリアの仕様）。
	if n := auditCountRows(t, store, `SELECT COUNT(*) FROM sessions WHERE id=?`, sid); n != 1 {
		t.Fatalf("sessions row after clear = %d, want 1 (must be preserved)", n)
	}
}

func TestClearSessionHistoryNullsDerivedColumns(t *testing.T) {
	store := startAuditSession(t, 20)
	store.UpdateSessionMessages(20, "first", "last")
	if _, err := store.UpdateSessionMeta(20, "title", []string{"tag"}, "summary", false); err != nil {
		t.Fatalf("UpdateSessionMeta: %v", err)
	}

	if err := store.ClearSessionHistory(20); err != nil {
		t.Fatalf("ClearSessionHistory: %v", err)
	}
	var first, last, title, tags, summary string
	if err := store.db.QueryRow(`SELECT COALESCE(first_message, ''), COALESCE(last_message, ''),
		COALESCE(title, ''), COALESCE(tags_json, ''), COALESCE(summary, '')
		FROM sessions WHERE live_session_id=?`, 20).Scan(&first, &last, &title, &tags, &summary); err != nil {
		t.Fatalf("query derived columns: %v", err)
	}
	if first != "" || last != "" || title != "" || tags != "" || summary != "" {
		t.Fatalf("derived columns after clear = %q, %q, %q, %q, %q; want all empty", first, last, title, tags, summary)
	}
}

func TestStartSessionUsesUniqueVirtualJSONLPath(t *testing.T) {
	store := startAuditSession(t, 21)
	first, err := store.StartSession(SessionStart{LiveSessionID: 21, Provider: "claude"})
	if err != nil {
		t.Fatalf("first StartSession: %v", err)
	}
	second, err := store.StartSession(SessionStart{LiveSessionID: 22, Provider: "codex"})
	if err != nil {
		t.Fatalf("second StartSession: %v", err)
	}
	if first == second {
		t.Fatalf("empty JSONLPath sessions collided: first=%d second=%d", first, second)
	}
}

func TestStoreEventSkipsEndedSession(t *testing.T) {
	store := startAuditSession(t, 23)
	store.EndSession(23, "completed", "done", time.Now())
	if err := store.StoreEvent(23, map[string]any{"ts": time.Now().Format(time.RFC3339), "type": "late"}); err != nil {
		t.Fatalf("StoreEvent after EndSession: %v", err)
	}
	if got := auditCountRows(t, store, "SELECT COUNT(*) FROM events"); got != 0 {
		t.Fatalf("late event count = %d, want 0", got)
	}
}

// TestSearchLike_EscapesUnderscore は LIKE フォールバックがリテラル '_' を
// ワイルドカードとして扱わず（過剰一致しない）厳密に部分一致することを確認する。
func TestSearchLike_EscapesUnderscore(t *testing.T) {
	store := startAuditSession(t, 3)
	ts := time.Now().Format(time.RFC3339)

	// リテラル "go_test" と、'_' をワイルドカード扱いすると誤一致する "goXtest"
	for _, msg := range []string{"run go_test here", "run goXtest here"} {
		if err := store.StoreEvent(3, map[string]any{"ts": ts, "type": "user_input", "session_id": 3, "text": msg}); err != nil {
			t.Fatalf("StoreEvent: %v", err)
		}
	}

	results, err := store.searchLike("go_test", 10)
	if err != nil {
		t.Fatalf("searchLike: %v", err)
	}
	if len(results) != 1 {
		t.Fatalf("searchLike(\"go_test\") matched %d rows, want 1 (literal '_' must not wildcard-match): %#v", len(results), results)
	}
	for _, r := range results {
		if !strings.Contains(r.Text, "go_test") {
			t.Fatalf("unexpected match: %q", r.Text)
		}
	}
}
