package sessionstore

import (
	"context"
	"database/sql"
	"path/filepath"
	"testing"
	"time"
)

// approvals 台帳へマーカー由来の承認が入り、回答で resolved になることを確認する。
// マーカー経路は Go 側に選択肢パーサを持たないので、options ではなく Block と
// 同一性（candidate_key / source_epoch）が保存されていることが要点。
func TestApprovalLedgerStoresMarkerApprovalAndResolution(t *testing.T) {
	logDir := filepath.Join(t.TempDir(), "logs")
	store, err := OpenForLogDir(logDir)
	if err != nil {
		t.Fatalf("OpenForLogDir: %v", err)
	}
	defer store.Close()

	if _, err := store.StartSession(SessionStart{
		LiveSessionID: 7,
		Provider:      "claude",
		CWD:           filepath.Join("D:", "dev", "many-ai-cli"),
		Branch:        "develop",
		State:         "standby",
		StartedAt:     time.Now().Add(-time.Hour).Format(time.RFC3339),
	}); err != nil {
		t.Fatalf("StartSession: %v", err)
	}

	detectedAt := time.Now().Add(-time.Minute)
	block := "Q1 どうしますか\n 1. やる\n 2. やらない"
	store.StoreApprovalDetected(ApprovalDetected{
		LiveSessionID: 7,
		Sig:           "sig-marker-1",
		Source:        "transcript",
		Kind:          "marker",
		Provider:      "claude",
		Block:         block,
		CandidateKey:  "claude\x00marker\x00q1",
		SourceEpoch:   3,
		DetectedAt:    detectedAt,
	})

	var (
		gotState        string
		gotBlock        string
		gotCandidateKey string
		gotEpoch        int64
		gotProvider     string
		gotSelected     sql.NullString
	)
	row := store.db.QueryRowContext(context.Background(),
		`SELECT state, block, candidate_key, source_epoch, provider, selected_text FROM approvals WHERE sig=?`,
		"sig-marker-1")
	if err := row.Scan(&gotState, &gotBlock, &gotCandidateKey, &gotEpoch, &gotProvider, &gotSelected); err != nil {
		t.Fatalf("select approval row: %v", err)
	}
	if gotState != "pending" {
		t.Fatalf("state = %q, want pending", gotState)
	}
	if gotBlock != block {
		t.Fatalf("block = %q, want the original marker block", gotBlock)
	}
	if gotCandidateKey != "claude\x00marker\x00q1" {
		t.Fatalf("candidate_key = %q", gotCandidateKey)
	}
	if gotEpoch != 3 {
		t.Fatalf("source_epoch = %d, want 3", gotEpoch)
	}
	if gotProvider != "claude" {
		t.Fatalf("provider = %q, want claude", gotProvider)
	}

	store.StoreApprovalConsumed(7, "sig-marker-1", "2", time.Now())

	var resolvedState, resolvedText string
	var resolvedAt sql.NullString
	row = store.db.QueryRowContext(context.Background(),
		`SELECT state, selected_text, resolved_at FROM approvals WHERE sig=?`, "sig-marker-1")
	if err := row.Scan(&resolvedState, &resolvedText, &resolvedAt); err != nil {
		t.Fatalf("select resolved row: %v", err)
	}
	if resolvedState != "resolved" {
		t.Fatalf("state = %q, want resolved", resolvedState)
	}
	if resolvedText != "2" {
		t.Fatalf("selected_text = %q, want 2", resolvedText)
	}
	if !resolvedAt.Valid || resolvedAt.String == "" {
		t.Fatal("resolved_at is empty")
	}
}

// 台帳を読み戻せることを確認する。新しい順・pending 絞り込み・セッション絞り込みの 3 つ。
func TestApprovalLedgerReadsBackRows(t *testing.T) {
	logDir := filepath.Join(t.TempDir(), "logs")
	store, err := OpenForLogDir(logDir)
	if err != nil {
		t.Fatalf("OpenForLogDir: %v", err)
	}
	defer store.Close()

	for _, live := range []int{4, 5} {
		if _, err := store.StartSession(SessionStart{
			LiveSessionID: live,
			Provider:      "claude",
			CWD:           filepath.Join("D:", "dev", "repo"+string(rune('0'+live))),
			State:         "standby",
			StartedAt:     time.Now().Add(-time.Hour).Format(time.RFC3339),
		}); err != nil {
			t.Fatalf("StartSession %d: %v", live, err)
		}
	}

	base := time.Now().Add(-10 * time.Minute)
	store.StoreApprovalDetected(ApprovalDetected{
		LiveSessionID: 4, Sig: "old", Source: "transcript", Kind: "marker",
		Provider: "claude", Block: "Q1\n 1. a\n 2. b", CandidateKey: "k-old",
		SourceEpoch: 1, DetectedAt: base,
	})
	store.StoreApprovalDetected(ApprovalDetected{
		LiveSessionID: 5, Sig: "new", Source: "transcript", Kind: "marker",
		Provider: "claude", Block: "Q1\n 1. c\n 2. d", CandidateKey: "k-new",
		SourceEpoch: 2, DetectedAt: base.Add(5 * time.Minute),
	})
	store.StoreApprovalConsumed(4, "old", "1", base.Add(time.Minute))

	all, err := store.RecentApprovals(0, false)
	if err != nil {
		t.Fatalf("RecentApprovals: %v", err)
	}
	if len(all) != 2 {
		t.Fatalf("RecentApprovals returned %d rows, want 2", len(all))
	}
	if all[0].Sig != "new" {
		t.Fatalf("newest first broken: got %q", all[0].Sig)
	}
	if all[0].Block == "" || all[0].CandidateKey != "k-new" || all[0].SourceEpoch != 2 {
		t.Fatalf("identity columns not read back: %+v", all[0])
	}
	if all[0].CWD == "" || all[0].Provider != "claude" {
		t.Fatalf("session columns not joined: %+v", all[0])
	}

	pending, err := store.RecentApprovals(0, true)
	if err != nil {
		t.Fatalf("RecentApprovals pending: %v", err)
	}
	if len(pending) != 1 || pending[0].Sig != "new" {
		t.Fatalf("pending filter broken: %+v", pending)
	}

	perSession, err := store.ApprovalsByLiveSession(4, 0, false)
	if err != nil {
		t.Fatalf("ApprovalsByLiveSession: %v", err)
	}
	if len(perSession) != 1 || perSession[0].Sig != "old" || perSession[0].State != "resolved" {
		t.Fatalf("per-session read broken: %+v", perSession)
	}
	if perSession[0].SelectedText != "1" {
		t.Fatalf("selected_text = %q, want 1", perSession[0].SelectedText)
	}

	empty, err := store.ApprovalsByLiveSession(999, 0, false)
	if err != nil {
		t.Fatalf("ApprovalsByLiveSession unknown: %v", err)
	}
	if len(empty) != 0 {
		t.Fatalf("unknown session returned %d rows", len(empty))
	}
}

// 列を持たない既存 DB を開いても移行が通ることを確認する。
// CREATE TABLE IF NOT EXISTS は既存テーブルへ列を足さないので、この経路が無いと
// 既存利用者の DB では承認の記録が毎回 SQL エラーになる。
func TestApprovalLedgerMigratesLegacyTable(t *testing.T) {
	base := t.TempDir()
	logDir := filepath.Join(base, "logs")
	dbPath := filepath.Join(base, "any-ai-cli.db")

	legacy, err := sql.Open("sqlite", dbPath)
	if err != nil {
		t.Fatalf("open legacy db: %v", err)
	}
	if _, err := legacy.Exec(`CREATE TABLE approvals (
		id INTEGER PRIMARY KEY AUTOINCREMENT,
		session_id INTEGER NOT NULL,
		sig TEXT NOT NULL,
		source TEXT,
		kind TEXT,
		question TEXT,
		context TEXT,
		options_json TEXT,
		selected_text TEXT,
		state TEXT NOT NULL,
		detected_at TEXT,
		resolved_at TEXT,
		UNIQUE(session_id, sig)
	)`); err != nil {
		t.Fatalf("create legacy approvals: %v", err)
	}
	if err := legacy.Close(); err != nil {
		t.Fatalf("close legacy db: %v", err)
	}

	store, err := OpenForLogDir(logDir)
	if err != nil {
		t.Fatalf("OpenForLogDir on legacy db: %v", err)
	}
	defer store.Close()

	rows, err := store.db.QueryContext(context.Background(), `PRAGMA table_info(approvals)`)
	if err != nil {
		t.Fatalf("table_info: %v", err)
	}
	defer rows.Close()
	have := map[string]bool{}
	for rows.Next() {
		var cid int
		var name, typ string
		var notNull int
		var dflt any
		var pk int
		if err := rows.Scan(&cid, &name, &typ, &notNull, &dflt, &pk); err != nil {
			t.Fatalf("scan table_info: %v", err)
		}
		have[name] = true
	}
	if err := rows.Err(); err != nil {
		t.Fatalf("table_info rows: %v", err)
	}
	for _, col := range []string{"provider", "candidate_key", "source_epoch", "block"} {
		if !have[col] {
			t.Fatalf("column %q was not migrated onto the legacy approvals table", col)
		}
	}
}
