package sessionstore

import (
	"context"
	"database/sql"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"

	"any-ai-cli/internal/proto"
	"any-ai-cli/internal/sessionlog"

	_ "modernc.org/sqlite"
)

const defaultTimeout = 3 * time.Second

type Store struct {
	db         *sql.DB
	ftsEnabled bool
}

type SessionStart struct {
	LiveSessionID int
	Provider      string
	Display       string
	CWD           string
	Branch        string
	Label         string
	Model         string
	Route         string
	Shell         string
	State         string
	StartedAt     string
	LogPath       string
	JSONLPath     string
}

type ChatMessage struct {
	ID             int64           `json:"id"`
	SessionID      int64           `json:"session_db_id,omitempty"`
	LiveSessionID  int             `json:"session_id,omitempty"`
	TS             string          `json:"ts"`
	Role           string          `json:"role"`
	Kind           string          `json:"kind"`
	RawText        string          `json:"rawText"`
	NormalizedText string          `json:"normalizedText,omitempty"`
	Attachments    []AttachmentRef `json:"attachments,omitempty"`
	Meta           map[string]any  `json:"meta,omitempty"`
}

type AttachmentRef struct {
	Path     string `json:"path,omitempty"`
	Filename string `json:"filename,omitempty"`
	Kind     string `json:"kind,omitempty"`
}

type SearchResult struct {
	MessageID     int64  `json:"message_id"`
	SessionDBID   int64  `json:"session_db_id"`
	LiveSessionID int    `json:"session_id"`
	Provider      string `json:"provider"`
	CWD           string `json:"cwd"`
	Branch        string `json:"branch,omitempty"`
	Model         string `json:"model,omitempty"`
	State         string `json:"state,omitempty"`
	StartedAt     string `json:"started_at,omitempty"`
	TS            string `json:"ts"`
	Role          string `json:"role"`
	Kind          string `json:"kind"`
	Text          string `json:"text"`
	Snippet       string `json:"snippet"`
}

type SessionOverview struct {
	ID            int64    `json:"id"`
	LiveSessionID int      `json:"session_id"`
	Provider      string   `json:"provider,omitempty"`
	Display       string   `json:"display_name,omitempty"`
	CWD           string   `json:"cwd,omitempty"`
	Branch        string   `json:"branch,omitempty"`
	Label         string   `json:"label,omitempty"`
	Model         string   `json:"model,omitempty"`
	Route         string   `json:"route,omitempty"`
	Shell         string   `json:"shell,omitempty"`
	State         string   `json:"state,omitempty"`
	StartedAt     string   `json:"started_at,omitempty"`
	LastOutputAt  string   `json:"last_output_at,omitempty"`
	EndedAt       string   `json:"ended_at,omitempty"`
	FirstMessage  string   `json:"first_message,omitempty"`
	LastMessage   string   `json:"last_message,omitempty"`
	EndReason     string   `json:"end_reason,omitempty"`
	Title         string   `json:"title,omitempty"`
	Tags          []string `json:"tags,omitempty"`
	Summary       string   `json:"summary,omitempty"`
	Archived      bool     `json:"archived"`
	LogPath       string   `json:"log_path,omitempty"`
	JSONLPath     string   `json:"jsonl_path,omitempty"`
	MessageCount  int      `json:"message_count,omitempty"`
	EventCount    int      `json:"event_count,omitempty"`
	ApprovalCount int      `json:"approval_count,omitempty"`
	PendingCount  int      `json:"pending_count,omitempty"`
}

type TimelineEvent struct {
	ID      int64          `json:"id"`
	Session int64          `json:"session_db_id"`
	TS      string         `json:"ts,omitempty"`
	Type    string         `json:"type"`
	Payload map[string]any `json:"payload,omitempty"`
}

type UsageBucket struct {
	Provider string `json:"provider,omitempty"`
	Model    string `json:"model,omitempty"`
	Sessions int    `json:"sessions"`
	Messages int    `json:"messages"`
	UserMsgs int    `json:"user_messages"`
	AIMsgs   int    `json:"ai_messages"`
}

type UsageSummary struct {
	TotalSessions int           `json:"total_sessions"`
	TotalMessages int           `json:"total_messages"`
	Providers     []UsageBucket `json:"providers"`
}

type ResetResult struct {
	Sessions    int `json:"sessions"`
	Events      int `json:"events"`
	Messages    int `json:"messages"`
	Approvals   int `json:"approvals"`
	Attachments int `json:"attachments"`
	Preserved   int `json:"preserved_sessions"`
}

func OpenForLogDir(logDir string) (*Store, error) {
	base := filepath.Dir(filepath.Clean(logDir))
	if base == "." || base == string(filepath.Separator) {
		base = logDir
	}
	if err := os.MkdirAll(base, sessionlog.PrivateDirMode); err != nil {
		return nil, fmt.Errorf("create session store dir: %w", err)
	}
	path := filepath.Join(base, "any-ai-cli.db")
	db, err := sql.Open("sqlite", path)
	if err != nil {
		return nil, fmt.Errorf("open sqlite session store: %w", err)
	}
	db.SetMaxOpenConns(1)
	db.SetMaxIdleConns(1)
	s := &Store{db: db}
	if err := s.init(); err != nil {
		_ = db.Close()
		return nil, err
	}
	return s, nil
}

func (s *Store) Close() error {
	if s == nil || s.db == nil {
		return nil
	}
	return s.db.Close()
}

func (s *Store) init() error {
	ctx, cancel := context.WithTimeout(context.Background(), defaultTimeout)
	defer cancel()
	pragmas := []string{
		`PRAGMA journal_mode=WAL`,
		`PRAGMA synchronous=NORMAL`,
		`PRAGMA foreign_keys=ON`,
		`PRAGMA busy_timeout=3000`,
	}
	for _, q := range pragmas {
		if _, err := s.db.ExecContext(ctx, q); err != nil {
			return fmt.Errorf("sqlite pragma: %w", err)
		}
	}
	stmts := []string{
		`CREATE TABLE IF NOT EXISTS sessions (
			id INTEGER PRIMARY KEY AUTOINCREMENT,
			live_session_id INTEGER NOT NULL,
			provider TEXT,
			display_name TEXT,
			cwd TEXT,
			branch TEXT,
			label TEXT,
			model TEXT,
			route TEXT,
			shell TEXT,
			state TEXT,
			started_at TEXT,
			last_output_at TEXT,
			ended_at TEXT,
			log_path TEXT,
			jsonl_path TEXT UNIQUE,
			first_message TEXT,
			last_message TEXT,
			end_reason TEXT,
			title TEXT,
			tags_json TEXT,
			summary TEXT,
			archived INTEGER NOT NULL DEFAULT 0,
			created_at TEXT NOT NULL DEFAULT (datetime('now')),
			updated_at TEXT NOT NULL DEFAULT (datetime('now'))
		)`,
		`CREATE TABLE IF NOT EXISTS events (
			id INTEGER PRIMARY KEY AUTOINCREMENT,
			session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
			ts TEXT,
			type TEXT NOT NULL,
			payload_json TEXT NOT NULL
		)`,
		`CREATE TABLE IF NOT EXISTS messages (
			id INTEGER PRIMARY KEY AUTOINCREMENT,
			session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
			ts TEXT,
			role TEXT NOT NULL,
			kind TEXT NOT NULL,
			text TEXT,
			raw_text TEXT,
			payload_json TEXT
		)`,
		`CREATE TABLE IF NOT EXISTS approvals (
			id INTEGER PRIMARY KEY AUTOINCREMENT,
			session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
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
		)`,
		`CREATE TABLE IF NOT EXISTS attachments (
			id INTEGER PRIMARY KEY AUTOINCREMENT,
			session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
			ts TEXT,
			path TEXT,
			filename TEXT,
			mime TEXT,
			size INTEGER
		)`,
		`CREATE INDEX IF NOT EXISTS idx_sessions_live ON sessions(live_session_id)`,
		`CREATE INDEX IF NOT EXISTS idx_sessions_started ON sessions(started_at)`,
		`CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id, id)`,
		`CREATE INDEX IF NOT EXISTS idx_events_session ON events(session_id, id)`,
		`CREATE INDEX IF NOT EXISTS idx_approvals_state ON approvals(state, detected_at)`,
	}
	for _, q := range stmts {
		if _, err := s.db.ExecContext(ctx, q); err != nil {
			return fmt.Errorf("init sqlite schema: %w", err)
		}
	}
	if err := s.ensureSessionColumns(ctx); err != nil {
		return err
	}
	if _, err := s.db.ExecContext(ctx, `CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(text, raw_text, content='messages', content_rowid='id')`); err == nil {
		s.ftsEnabled = true
	}
	return nil
}

func (s *Store) ensureSessionColumns(ctx context.Context) error {
	rows, err := s.db.QueryContext(ctx, `PRAGMA table_info(sessions)`)
	if err != nil {
		return fmt.Errorf("inspect sessions schema: %w", err)
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
			return err
		}
		have[name] = true
	}
	if err := rows.Err(); err != nil {
		return err
	}
	add := []struct {
		name string
		sql  string
	}{
		{"title", `ALTER TABLE sessions ADD COLUMN title TEXT`},
		{"tags_json", `ALTER TABLE sessions ADD COLUMN tags_json TEXT`},
		{"summary", `ALTER TABLE sessions ADD COLUMN summary TEXT`},
		{"archived", `ALTER TABLE sessions ADD COLUMN archived INTEGER NOT NULL DEFAULT 0`},
	}
	for _, col := range add {
		if have[col.name] {
			continue
		}
		if _, err := s.db.ExecContext(ctx, col.sql); err != nil {
			return fmt.Errorf("migrate sessions.%s: %w", col.name, err)
		}
	}
	return nil
}

func (s *Store) StartSession(st SessionStart) (int64, error) {
	if s == nil || s.db == nil {
		return 0, nil
	}
	ctx, cancel := context.WithTimeout(context.Background(), defaultTimeout)
	defer cancel()
	state := strings.TrimSpace(st.State)
	if state == "" {
		state = "standby"
	}
	var id int64
	err := s.db.QueryRowContext(ctx, `INSERT INTO sessions (
		live_session_id, provider, display_name, cwd, branch, label, model, route, shell,
		state, started_at, log_path, jsonl_path, updated_at
	) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
	ON CONFLICT(jsonl_path) DO UPDATE SET
		live_session_id=excluded.live_session_id,
		provider=excluded.provider,
		display_name=excluded.display_name,
		cwd=excluded.cwd,
		branch=excluded.branch,
		label=excluded.label,
		model=excluded.model,
		route=excluded.route,
		shell=excluded.shell,
		state=excluded.state,
		started_at=excluded.started_at,
		log_path=excluded.log_path,
		updated_at=excluded.updated_at,
		ended_at=NULL,
		end_reason=NULL
	RETURNING id`,
		st.LiveSessionID, st.Provider, st.Display, st.CWD, st.Branch, st.Label, st.Model,
		st.Route, st.Shell, state, st.StartedAt, st.LogPath, st.JSONLPath, time.Now().Format(time.RFC3339),
	).Scan(&id)
	return id, err
}

// CloseStaleSessions は ended_at が未設定のまま残っている全行を一括クローズする。
// Hub がクラッシュ・強制終了した場合 EndSession が呼ばれず未終了行が残り、
// 次回 run で同じ live_session_id を採番された別セッションの UPDATE
// （live_session_id=? AND ended_at IS NULL 条件）が旧行にも書き込まれてしまう。
// Hub 起動直後（セッション登録前）に呼ぶこと。閉じた行数を返す。
func (s *Store) CloseStaleSessions(endedAt time.Time, reason string) (int64, error) {
	if s == nil || s.db == nil {
		return 0, nil
	}
	if endedAt.IsZero() {
		endedAt = time.Now()
	}
	ctx, cancel := context.WithTimeout(context.Background(), defaultTimeout)
	defer cancel()
	res, err := s.db.ExecContext(ctx, `UPDATE sessions SET state='disconnected',
		end_reason=COALESCE(NULLIF(?, ''), end_reason), ended_at=?, updated_at=?
		WHERE ended_at IS NULL`,
		reason, endedAt.Format(time.RFC3339), time.Now().Format(time.RFC3339))
	if err != nil {
		return 0, err
	}
	n, _ := res.RowsAffected()
	return n, nil
}

func (s *Store) UpdateSessionMessages(liveSessionID int, firstMessage, lastMessage string) {
	_ = s.exec(`UPDATE sessions SET first_message=CASE WHEN first_message='' OR first_message IS NULL THEN ? ELSE first_message END,
		last_message=?, updated_at=? WHERE live_session_id=? AND ended_at IS NULL`,
		firstMessage, lastMessage, time.Now().Format(time.RFC3339), liveSessionID)
}

func (s *Store) UpdateSessionState(liveSessionID int, state, lastOutputAt string) {
	if strings.TrimSpace(state) == "" && strings.TrimSpace(lastOutputAt) == "" {
		return
	}
	_ = s.exec(`UPDATE sessions SET state=COALESCE(NULLIF(?, ''), state),
		last_output_at=COALESCE(NULLIF(?, ''), last_output_at), updated_at=?
		WHERE live_session_id=? AND ended_at IS NULL`,
		state, lastOutputAt, time.Now().Format(time.RFC3339), liveSessionID)
}

func (s *Store) EndSession(liveSessionID int, state, reason string, endedAt time.Time) {
	if endedAt.IsZero() {
		endedAt = time.Now()
	}
	_ = s.exec(`UPDATE sessions SET state=COALESCE(NULLIF(?, ''), state),
		end_reason=COALESCE(NULLIF(?, ''), end_reason), ended_at=?, updated_at=?
		WHERE live_session_id=? AND ended_at IS NULL`,
		state, reason, endedAt.Format(time.RFC3339), time.Now().Format(time.RFC3339), liveSessionID)
}

func (s *Store) ClearSessionHistory(liveSessionID int) {
	id, err := s.sessionIDForLive(liveSessionID)
	if err != nil || id == 0 {
		return
	}
	ctx, cancel := context.WithTimeout(context.Background(), defaultTimeout)
	defer cancel()
	tx, err := s.db.BeginTx(ctx, nil)
	if err != nil {
		return
	}
	defer tx.Rollback()
	if s.ftsEnabled {
		_, _ = tx.ExecContext(ctx, `DELETE FROM messages_fts WHERE rowid IN (SELECT id FROM messages WHERE session_id=?)`, id)
	}
	_, _ = tx.ExecContext(ctx, `DELETE FROM messages WHERE session_id=?`, id)
	_, _ = tx.ExecContext(ctx, `DELETE FROM events WHERE session_id=?`, id)
	_, _ = tx.ExecContext(ctx, `DELETE FROM approvals WHERE session_id=?`, id)
	_ = tx.Commit()
}

func (s *Store) StoreEvent(liveSessionID int, event map[string]any) error {
	if s == nil || s.db == nil {
		return nil
	}
	sessionID, err := s.sessionIDForLive(liveSessionID)
	if err != nil || sessionID == 0 {
		return err
	}
	payload, err := json.Marshal(event)
	if err != nil {
		return err
	}
	ts := stringValue(event["ts"])
	typ := stringValue(event["type"])
	if typ == "" {
		typ = "event"
	}
	ctx, cancel := context.WithTimeout(context.Background(), defaultTimeout)
	defer cancel()
	tx, err := s.db.BeginTx(ctx, nil)
	if err != nil {
		return err
	}
	defer tx.Rollback()
	if _, err := tx.ExecContext(ctx, `INSERT INTO events(session_id, ts, type, payload_json) VALUES (?, ?, ?, ?)`, sessionID, ts, typ, string(payload)); err != nil {
		return err
	}
	if err := s.storeMessageForEvent(ctx, tx, sessionID, ts, typ, event, payload); err != nil {
		return err
	}
	if err := updateSessionForEvent(ctx, tx, liveSessionID, typ, event); err != nil {
		return err
	}
	return tx.Commit()
}

func (s *Store) StoreApprovalDetected(liveSessionID int, sig, source, kind, question, contextText string, options []proto.ApprovalOption, detectedAt time.Time) {
	sessionID, err := s.sessionIDForLive(liveSessionID)
	if err != nil || sessionID == 0 || sig == "" {
		return
	}
	if detectedAt.IsZero() {
		detectedAt = time.Now()
	}
	optionsJSON, _ := json.Marshal(options)
	_ = s.exec(`INSERT INTO approvals(session_id, sig, source, kind, question, context, options_json, state, detected_at)
		VALUES (?, ?, ?, ?, ?, ?, ?, 'pending', ?)
		ON CONFLICT(session_id, sig) DO UPDATE SET
			source=excluded.source,
			kind=excluded.kind,
			question=excluded.question,
			context=excluded.context,
			options_json=excluded.options_json,
			state='pending',
			detected_at=excluded.detected_at,
			resolved_at=NULL`,
		sessionID, sig, source, kind, question, contextText, string(optionsJSON), detectedAt.Format(time.RFC3339))
}

func (s *Store) StoreApprovalConsumed(liveSessionID int, sig, selectedText string, resolvedAt time.Time) {
	sessionID, err := s.sessionIDForLive(liveSessionID)
	if err != nil || sessionID == 0 || sig == "" {
		return
	}
	if resolvedAt.IsZero() {
		resolvedAt = time.Now()
	}
	_ = s.exec(`UPDATE approvals SET state='resolved', selected_text=?, resolved_at=? WHERE session_id=? AND sig=?`,
		selectedText, resolvedAt.Format(time.RFC3339), sessionID, sig)
}

func (s *Store) ChatMessagesByLiveSession(liveSessionID, limit int) ([]ChatMessage, error) {
	sessionID, err := s.sessionIDForLive(liveSessionID)
	if err != nil || sessionID == 0 {
		return nil, err
	}
	if limit <= 0 || limit > 1000 {
		limit = 400
	}
	ctx, cancel := context.WithTimeout(context.Background(), defaultTimeout)
	defer cancel()
	rows, err := s.db.QueryContext(ctx, `SELECT id, ts, role, kind, COALESCE(text, ''), COALESCE(raw_text, ''), COALESCE(payload_json, '')
		FROM messages WHERE session_id=? ORDER BY id DESC LIMIT ?`, sessionID, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var rev []ChatMessage
	for rows.Next() {
		var msg ChatMessage
		var payload string
		msg.SessionID = sessionID
		msg.LiveSessionID = liveSessionID
		if err := rows.Scan(&msg.ID, &msg.TS, &msg.Role, &msg.Kind, &msg.NormalizedText, &msg.RawText, &payload); err != nil {
			return nil, err
		}
		if msg.RawText == "" {
			msg.RawText = msg.NormalizedText
		}
		if payload != "" {
			var meta map[string]any
			if json.Unmarshal([]byte(payload), &meta) == nil {
				msg.Meta = meta
				if msg.Kind == "attach" {
					msg.Attachments = []AttachmentRef{{
						Path:     stringValue(meta["path"]),
						Filename: stringValue(meta["filename"]),
						Kind:     "file",
					}}
				}
			}
		}
		rev = append(rev, msg)
	}
	if err := rows.Err(); err != nil {
		return nil, err
	}
	for i, j := 0, len(rev)-1; i < j; i, j = i+1, j-1 {
		rev[i], rev[j] = rev[j], rev[i]
	}
	return coalesceAIMessages(rev), nil
}

func (s *Store) SearchMessages(query string, limit int) ([]SearchResult, error) {
	query = strings.TrimSpace(query)
	if query == "" {
		return nil, nil
	}
	if limit <= 0 || limit > 100 {
		limit = 50
	}
	if s.ftsEnabled {
		if results, err := s.searchFTS(query, limit); err == nil {
			return results, nil
		}
	}
	return s.searchLike(query, limit)
}

func (s *Store) ListSessions(limit int, includeArchived bool) ([]SessionOverview, error) {
	if s == nil || s.db == nil {
		return nil, nil
	}
	if limit <= 0 || limit > 500 {
		limit = 100
	}
	ctx, cancel := context.WithTimeout(context.Background(), defaultTimeout)
	defer cancel()
	where := "WHERE COALESCE(se.archived, 0)=0"
	if includeArchived {
		where = ""
	}
	rows, err := s.db.QueryContext(ctx, `SELECT
			se.id, se.live_session_id, COALESCE(se.provider, ''), COALESCE(se.display_name, ''), COALESCE(se.cwd, ''),
			COALESCE(se.branch, ''), COALESCE(se.label, ''), COALESCE(se.model, ''), COALESCE(se.route, ''),
			COALESCE(se.shell, ''), COALESCE(se.state, ''), COALESCE(se.started_at, ''), COALESCE(se.last_output_at, ''),
			COALESCE(se.ended_at, ''), COALESCE(se.first_message, ''), COALESCE(se.last_message, ''),
			COALESCE(se.end_reason, ''), COALESCE(se.title, ''), COALESCE(se.tags_json, '[]'), COALESCE(se.summary, ''),
			COALESCE(se.archived, 0), COALESCE(se.log_path, ''), COALESCE(se.jsonl_path, ''),
			(SELECT COUNT(*) FROM messages m WHERE m.session_id=se.id),
			(SELECT COUNT(*) FROM events e WHERE e.session_id=se.id),
			(SELECT COUNT(*) FROM approvals a WHERE a.session_id=se.id),
			(SELECT COUNT(*) FROM approvals a WHERE a.session_id=se.id AND a.state='pending')
		FROM sessions se `+where+`
		ORDER BY COALESCE(NULLIF(se.last_output_at, ''), NULLIF(se.ended_at, ''), NULLIF(se.started_at, ''), se.updated_at) DESC, se.id DESC
		LIMIT ?`, limit)
	if err != nil {
		return nil, err
	}
	return scanSessionOverviews(rows)
}

func (s *Store) SessionOverviewByLiveSession(liveSessionID int) (SessionOverview, error) {
	sessionID, err := s.sessionIDForLive(liveSessionID)
	if err != nil || sessionID == 0 {
		return SessionOverview{}, err
	}
	ctx, cancel := context.WithTimeout(context.Background(), defaultTimeout)
	defer cancel()
	rows, err := s.db.QueryContext(ctx, `SELECT
			se.id, se.live_session_id, COALESCE(se.provider, ''), COALESCE(se.display_name, ''), COALESCE(se.cwd, ''),
			COALESCE(se.branch, ''), COALESCE(se.label, ''), COALESCE(se.model, ''), COALESCE(se.route, ''),
			COALESCE(se.shell, ''), COALESCE(se.state, ''), COALESCE(se.started_at, ''), COALESCE(se.last_output_at, ''),
			COALESCE(se.ended_at, ''), COALESCE(se.first_message, ''), COALESCE(se.last_message, ''),
			COALESCE(se.end_reason, ''), COALESCE(se.title, ''), COALESCE(se.tags_json, '[]'), COALESCE(se.summary, ''),
			COALESCE(se.archived, 0), COALESCE(se.log_path, ''), COALESCE(se.jsonl_path, ''),
			(SELECT COUNT(*) FROM messages m WHERE m.session_id=se.id),
			(SELECT COUNT(*) FROM events e WHERE e.session_id=se.id),
			(SELECT COUNT(*) FROM approvals a WHERE a.session_id=se.id),
			(SELECT COUNT(*) FROM approvals a WHERE a.session_id=se.id AND a.state='pending')
		FROM sessions se WHERE se.id=?`, sessionID)
	if err != nil {
		return SessionOverview{}, err
	}
	items, err := scanSessionOverviews(rows)
	if err != nil || len(items) == 0 {
		return SessionOverview{}, err
	}
	return items[0], nil
}

func (s *Store) UpdateSessionMeta(liveSessionID int, title string, tags []string, summary string, archived bool) (SessionOverview, error) {
	sessionID, err := s.sessionIDForLive(liveSessionID)
	if err != nil || sessionID == 0 {
		return SessionOverview{}, err
	}
	title = trimRunes(strings.TrimSpace(title), 160)
	summary = trimRunes(strings.TrimSpace(summary), 4000)
	tags = normalizeTags(tags)
	tagsJSON, _ := json.Marshal(tags)
	if err := s.exec(`UPDATE sessions SET title=?, tags_json=?, summary=?, archived=?, updated_at=? WHERE id=?`,
		title, string(tagsJSON), summary, boolInt(archived), time.Now().Format(time.RFC3339), sessionID); err != nil {
		return SessionOverview{}, err
	}
	return s.SessionOverviewByLiveSession(liveSessionID)
}

func (s *Store) TimelineByLiveSession(liveSessionID, limit int) ([]TimelineEvent, error) {
	sessionID, err := s.sessionIDForLive(liveSessionID)
	if err != nil || sessionID == 0 {
		return nil, err
	}
	if limit <= 0 || limit > 2000 {
		limit = 400
	}
	ctx, cancel := context.WithTimeout(context.Background(), defaultTimeout)
	defer cancel()
	rows, err := s.db.QueryContext(ctx, `SELECT id, session_id, COALESCE(ts, ''), type, payload_json
		FROM events WHERE session_id=? ORDER BY id DESC LIMIT ?`, sessionID, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var rev []TimelineEvent
	for rows.Next() {
		var ev TimelineEvent
		var payload string
		if err := rows.Scan(&ev.ID, &ev.Session, &ev.TS, &ev.Type, &payload); err != nil {
			return nil, err
		}
		if payload != "" {
			_ = json.Unmarshal([]byte(payload), &ev.Payload)
		}
		rev = append(rev, ev)
	}
	if err := rows.Err(); err != nil {
		return nil, err
	}
	for i, j := 0, len(rev)-1; i < j; i, j = i+1, j-1 {
		rev[i], rev[j] = rev[j], rev[i]
	}
	return rev, nil
}

func (s *Store) UsageSummary() (UsageSummary, error) {
	if s == nil || s.db == nil {
		return UsageSummary{}, nil
	}
	ctx, cancel := context.WithTimeout(context.Background(), defaultTimeout)
	defer cancel()
	var out UsageSummary
	_ = s.db.QueryRowContext(ctx, `SELECT COUNT(*) FROM sessions`).Scan(&out.TotalSessions)
	_ = s.db.QueryRowContext(ctx, `SELECT COUNT(*) FROM messages`).Scan(&out.TotalMessages)
	rows, err := s.db.QueryContext(ctx, `SELECT COALESCE(se.provider, ''), COALESCE(se.model, ''),
			COUNT(DISTINCT se.id), COUNT(m.id),
			COALESCE(SUM(CASE WHEN m.role='user' THEN 1 ELSE 0 END), 0),
			COALESCE(SUM(CASE WHEN m.role='ai' THEN 1 ELSE 0 END), 0)
		FROM sessions se
		LEFT JOIN messages m ON m.session_id=se.id
		GROUP BY COALESCE(se.provider, ''), COALESCE(se.model, '')
		ORDER BY COUNT(m.id) DESC, COUNT(DISTINCT se.id) DESC`)
	if err != nil {
		return out, err
	}
	defer rows.Close()
	for rows.Next() {
		var b UsageBucket
		if err := rows.Scan(&b.Provider, &b.Model, &b.Sessions, &b.Messages, &b.UserMsgs, &b.AIMsgs); err != nil {
			return out, err
		}
		out.Providers = append(out.Providers, b)
	}
	return out, rows.Err()
}

func (s *Store) StaleSessions(cutoff time.Time, limit int) ([]SessionOverview, error) {
	if s == nil || s.db == nil || cutoff.IsZero() {
		return nil, nil
	}
	if limit <= 0 || limit > 500 {
		limit = 100
	}
	ctx, cancel := context.WithTimeout(context.Background(), defaultTimeout)
	defer cancel()
	rows, err := s.db.QueryContext(ctx, `SELECT
			se.id, se.live_session_id, COALESCE(se.provider, ''), COALESCE(se.display_name, ''), COALESCE(se.cwd, ''),
			COALESCE(se.branch, ''), COALESCE(se.label, ''), COALESCE(se.model, ''), COALESCE(se.route, ''),
			COALESCE(se.shell, ''), COALESCE(se.state, ''), COALESCE(se.started_at, ''), COALESCE(se.last_output_at, ''),
			COALESCE(se.ended_at, ''), COALESCE(se.first_message, ''), COALESCE(se.last_message, ''),
			COALESCE(se.end_reason, ''), COALESCE(se.title, ''), COALESCE(se.tags_json, '[]'), COALESCE(se.summary, ''),
			COALESCE(se.archived, 0), COALESCE(se.log_path, ''), COALESCE(se.jsonl_path, ''),
			(SELECT COUNT(*) FROM messages m WHERE m.session_id=se.id),
			(SELECT COUNT(*) FROM events e WHERE e.session_id=se.id),
			(SELECT COUNT(*) FROM approvals a WHERE a.session_id=se.id),
			(SELECT COUNT(*) FROM approvals a WHERE a.session_id=se.id AND a.state='pending')
		FROM sessions se
		WHERE COALESCE(se.archived, 0)=0
		  AND (
			se.ended_at IS NOT NULL
			OR se.state IN ('completed', 'error', 'disconnected', 'dismissed')
			OR COALESCE(NULLIF(se.last_output_at, ''), NULLIF(se.started_at, ''), se.created_at) < ?
		  )
		ORDER BY COALESCE(NULLIF(se.last_output_at, ''), NULLIF(se.ended_at, ''), NULLIF(se.started_at, ''), se.updated_at) ASC, se.id ASC
		LIMIT ?`, cutoff.Format(time.RFC3339), limit)
	if err != nil {
		return nil, err
	}
	return scanSessionOverviews(rows)
}

func (s *Store) PruneOlderThan(cutoff time.Time) error {
	if s == nil || s.db == nil || cutoff.IsZero() {
		return nil
	}
	ctx, cancel := context.WithTimeout(context.Background(), defaultTimeout)
	defer cancel()
	_, err := s.db.ExecContext(ctx, `DELETE FROM sessions
		WHERE ended_at IS NOT NULL AND ended_at < ?`, cutoff.Format(time.RFC3339))
	return err
}

func (s *Store) ResetHistory(preserveLiveIDs []int) (ResetResult, error) {
	var out ResetResult
	if s == nil || s.db == nil {
		return out, nil
	}
	ctx, cancel := context.WithTimeout(context.Background(), defaultTimeout)
	defer cancel()
	tx, err := s.db.BeginTx(ctx, nil)
	if err != nil {
		return out, err
	}
	defer tx.Rollback()

	for _, table := range []struct {
		name string
		dst  *int
	}{
		{"sessions", &out.Sessions},
		{"events", &out.Events},
		{"messages", &out.Messages},
		{"approvals", &out.Approvals},
		{"attachments", &out.Attachments},
	} {
		if err := tx.QueryRowContext(ctx, `SELECT COUNT(*) FROM `+table.name).Scan(table.dst); err != nil {
			return out, err
		}
	}
	preserveSet := make(map[int]bool, len(preserveLiveIDs))
	for _, id := range preserveLiveIDs {
		if id > 0 {
			preserveSet[id] = true
		}
	}
	preserved := make([]int64, 0, len(preserveSet))
	for id := range preserveSet {
		var dbID int64
		err := tx.QueryRowContext(ctx, `SELECT id FROM sessions WHERE live_session_id=? ORDER BY id DESC LIMIT 1`, id).Scan(&dbID)
		if err == nil && dbID > 0 {
			preserved = append(preserved, dbID)
		} else if err != nil && err != sql.ErrNoRows {
			return out, err
		}
	}
	out.Preserved = len(preserved)

	if s.ftsEnabled {
		if _, err := tx.ExecContext(ctx, `DELETE FROM messages_fts`); err != nil {
			return out, err
		}
	}
	if len(preserved) == 0 {
		if _, err := tx.ExecContext(ctx, `DELETE FROM sessions`); err != nil {
			return out, err
		}
		return out, tx.Commit()
	}

	placeholders := make([]string, len(preserved))
	args := make([]any, len(preserved))
	for i, id := range preserved {
		placeholders[i] = "?"
		args[i] = id
	}
	inClause := strings.Join(placeholders, ",")
	// #nosec G202 -- table は固定リスト、inClause は "?" プレースホルダの連結のみ（値は args で束縛）
	for _, table := range []string{"events", "messages", "approvals", "attachments"} {
		if _, err := tx.ExecContext(ctx, `DELETE FROM `+table+` WHERE session_id IN (`+inClause+`)`, args...); err != nil { // #nosec G202 -- 同上
			return out, err
		}
	}
	if _, err := tx.ExecContext(ctx, `DELETE FROM sessions WHERE id NOT IN (`+inClause+`)`, args...); err != nil { // #nosec G202 -- inClause はプレースホルダのみ
		return out, err
	}
	// #nosec G202 -- inClause はプレースホルダのみ
	if _, err := tx.ExecContext(ctx, `UPDATE sessions SET
		first_message=NULL,
		last_message=NULL,
		title=NULL,
		tags_json=NULL,
		summary=NULL,
		updated_at=?
		WHERE id IN (`+inClause+`)`, append([]any{time.Now().Format(time.RFC3339)}, args...)...); err != nil {
		return out, err
	}
	return out, tx.Commit()
}

func (s *Store) storeMessageForEvent(ctx context.Context, tx *sql.Tx, sessionID int64, ts, typ string, event map[string]any, payload []byte) error {
	switch typ {
	case "user_input":
		text := strings.TrimRight(stringValue(event["text"]), "\r\n")
		if strings.TrimSpace(text) == "" {
			return nil
		}
		return s.insertMessage(ctx, tx, sessionID, ts, "user", "text", text, text, string(payload))
	case "pty_output":
		text := sessionlog.CleanVisibleText(stringValue(event["text"]))
		text = strings.TrimSpace(text)
		if text == "" || isNoiseOutput(text) {
			return nil
		}
		return s.insertMessage(ctx, tx, sessionID, ts, "ai", "text", text, text, "")
	case "attach":
		text := stringValue(event["filename"])
		if text == "" {
			text = stringValue(event["path"])
		}
		if text == "" {
			return nil
		}
		if _, err := tx.ExecContext(ctx, `INSERT INTO attachments(session_id, ts, path, filename) VALUES (?, ?, ?, ?)`,
			sessionID, ts, stringValue(event["path"]), stringValue(event["filename"])); err != nil {
			return err
		}
		return s.insertMessage(ctx, tx, sessionID, ts, "system", "attach", text, text, string(payload))
	default:
		return nil
	}
}

func (s *Store) insertMessage(ctx context.Context, tx *sql.Tx, sessionID int64, ts, role, kind, text, rawText, payload string) error {
	res, err := tx.ExecContext(ctx, `INSERT INTO messages(session_id, ts, role, kind, text, raw_text, payload_json) VALUES (?, ?, ?, ?, ?, ?, ?)`,
		sessionID, ts, role, kind, text, rawText, payload)
	if err != nil {
		return err
	}
	if !s.ftsEnabled {
		return nil
	}
	id, err := res.LastInsertId()
	if err != nil {
		return nil
	}
	_, _ = tx.ExecContext(ctx, `INSERT INTO messages_fts(rowid, text, raw_text) VALUES (?, ?, ?)`, id, text, rawText)
	return nil
}

func updateSessionForEvent(ctx context.Context, tx *sql.Tx, liveSessionID int, typ string, event map[string]any) error {
	now := time.Now().Format(time.RFC3339)
	switch typ {
	case "session_start", "session_reattach":
		_, err := tx.ExecContext(ctx, `UPDATE sessions SET branch=COALESCE(NULLIF(?, ''), branch),
			label=COALESCE(NULLIF(?, ''), label), model=COALESCE(NULLIF(?, ''), model),
			shell=COALESCE(NULLIF(?, ''), shell), updated_at=? WHERE live_session_id=? AND ended_at IS NULL`,
			stringValue(event["branch"]), stringValue(event["label"]), stringValue(event["model"]), stringValue(event["shell"]), now, liveSessionID)
		return err
	case "session_end":
		_, err := tx.ExecContext(ctx, `UPDATE sessions SET state=COALESCE(NULLIF(?, ''), state),
			end_reason=COALESCE(NULLIF(?, ''), end_reason), ended_at=COALESCE(NULLIF(?, ''), ?), updated_at=?
			WHERE live_session_id=? AND ended_at IS NULL`,
			stringValue(event["state"]), stringValue(event["reason"]), stringValue(event["ts"]), now, liveSessionID)
		return err
	case "user_input":
		text := strings.TrimRight(stringValue(event["text"]), "\r\n")
		if strings.TrimSpace(text) == "" {
			return nil
		}
		last := text
		if isDigitsText(text) {
			last = ""
		}
		_, err := tx.ExecContext(ctx, `UPDATE sessions SET
			first_message=CASE WHEN (first_message IS NULL OR first_message='') THEN ? ELSE first_message END,
			last_message=COALESCE(NULLIF(?, ''), last_message),
			updated_at=?
			WHERE live_session_id=? AND ended_at IS NULL`,
			text, last, now, liveSessionID)
		return err
	}
	return nil
}

func (s *Store) searchFTS(query string, limit int) ([]SearchResult, error) {
	ctx, cancel := context.WithTimeout(context.Background(), defaultTimeout)
	defer cancel()
	rows, err := s.db.QueryContext(ctx, `SELECT m.id, m.session_id, se.live_session_id, se.provider, se.cwd, se.branch, se.model, se.state, se.started_at,
			m.ts, m.role, m.kind, COALESCE(m.text, ''), snippet(messages_fts, 0, '', '', '...', 16)
		FROM messages_fts
		JOIN messages m ON m.id = messages_fts.rowid
		JOIN sessions se ON se.id = m.session_id
		WHERE messages_fts MATCH ?
		ORDER BY rank
		LIMIT ?`, ftsQuery(query), limit)
	if err != nil {
		return nil, err
	}
	return scanSearchResults(rows)
}

func (s *Store) searchLike(query string, limit int) ([]SearchResult, error) {
	ctx, cancel := context.WithTimeout(context.Background(), defaultTimeout)
	defer cancel()
	like := "%" + strings.ReplaceAll(query, "%", "\\%") + "%"
	rows, err := s.db.QueryContext(ctx, `SELECT m.id, m.session_id, se.live_session_id, se.provider, se.cwd, se.branch, se.model, se.state, se.started_at,
			m.ts, m.role, m.kind, COALESCE(m.text, ''), COALESCE(m.text, '')
		FROM messages m
		JOIN sessions se ON se.id = m.session_id
		WHERE m.text LIKE ? ESCAPE '\'
		ORDER BY m.id DESC
		LIMIT ?`, like, limit)
	if err != nil {
		return nil, err
	}
	return scanSearchResults(rows)
}

func scanSearchResults(rows *sql.Rows) ([]SearchResult, error) {
	defer rows.Close()
	var out []SearchResult
	for rows.Next() {
		var r SearchResult
		if err := rows.Scan(&r.MessageID, &r.SessionDBID, &r.LiveSessionID, &r.Provider, &r.CWD, &r.Branch, &r.Model, &r.State, &r.StartedAt, &r.TS, &r.Role, &r.Kind, &r.Text, &r.Snippet); err != nil {
			return nil, err
		}
		if r.Snippet == "" {
			r.Snippet = r.Text
		}
		if len([]rune(r.Snippet)) > 240 {
			r.Snippet = string([]rune(r.Snippet)[:240]) + "..."
		}
		out = append(out, r)
	}
	return out, rows.Err()
}

func scanSessionOverviews(rows *sql.Rows) ([]SessionOverview, error) {
	defer rows.Close()
	var out []SessionOverview
	for rows.Next() {
		var item SessionOverview
		var tagsJSON string
		var archived int
		if err := rows.Scan(
			&item.ID, &item.LiveSessionID, &item.Provider, &item.Display, &item.CWD, &item.Branch,
			&item.Label, &item.Model, &item.Route, &item.Shell, &item.State, &item.StartedAt,
			&item.LastOutputAt, &item.EndedAt, &item.FirstMessage, &item.LastMessage,
			&item.EndReason, &item.Title, &tagsJSON, &item.Summary, &archived, &item.LogPath,
			&item.JSONLPath, &item.MessageCount, &item.EventCount, &item.ApprovalCount, &item.PendingCount,
		); err != nil {
			return nil, err
		}
		item.Tags = decodeTags(tagsJSON)
		item.Archived = archived != 0
		out = append(out, item)
	}
	return out, rows.Err()
}

func (s *Store) sessionIDForLive(liveSessionID int) (int64, error) {
	if s == nil || s.db == nil || liveSessionID <= 0 {
		return 0, nil
	}
	ctx, cancel := context.WithTimeout(context.Background(), defaultTimeout)
	defer cancel()
	var id int64
	err := s.db.QueryRowContext(ctx, `SELECT id FROM sessions WHERE live_session_id=? ORDER BY id DESC LIMIT 1`, liveSessionID).Scan(&id)
	if err == sql.ErrNoRows {
		return 0, nil
	}
	return id, err
}

func (s *Store) exec(query string, args ...any) error {
	if s == nil || s.db == nil {
		return nil
	}
	ctx, cancel := context.WithTimeout(context.Background(), defaultTimeout)
	defer cancel()
	_, err := s.db.ExecContext(ctx, query, args...)
	return err
}

func coalesceAIMessages(in []ChatMessage) []ChatMessage {
	out := make([]ChatMessage, 0, len(in))
	var pending *ChatMessage
	flush := func() {
		if pending != nil {
			pending.RawText = strings.TrimSpace(pending.RawText)
			pending.NormalizedText = strings.TrimSpace(pending.NormalizedText)
			if pending.RawText != "" {
				out = append(out, *pending)
			}
			pending = nil
		}
	}
	for _, msg := range in {
		if msg.Role == "ai" && msg.Kind == "text" {
			if pending == nil {
				cp := msg
				pending = &cp
			} else {
				if pending.RawText != "" {
					pending.RawText += "\n"
				}
				pending.RawText += msg.RawText
				if pending.NormalizedText != "" {
					pending.NormalizedText += "\n"
				}
				pending.NormalizedText += msg.NormalizedText
			}
			continue
		}
		flush()
		out = append(out, msg)
	}
	flush()
	return out
}

func ftsQuery(q string) string {
	parts := strings.Fields(q)
	if len(parts) == 0 {
		parts = []string{q}
	}
	quoted := make([]string, 0, len(parts))
	for _, p := range parts {
		p = strings.TrimSpace(p)
		if p == "" {
			continue
		}
		p = strings.ReplaceAll(p, `"`, `""`)
		quoted = append(quoted, `"`+p+`"`)
	}
	if len(quoted) == 0 {
		return `""`
	}
	return strings.Join(quoted, " AND ")
}

func isNoiseOutput(s string) bool {
	t := strings.TrimSpace(s)
	if t == "" {
		return true
	}
	if len([]rune(t)) <= 2 {
		return true
	}
	switch t {
	case "Boot", "Boo", "Bo", "Thinking", "Working":
		return true
	default:
		return false
	}
}

func isDigitsText(s string) bool {
	if s == "" {
		return false
	}
	for _, r := range s {
		if r < '0' || r > '9' {
			return false
		}
	}
	return true
}

func normalizeTags(tags []string) []string {
	seen := map[string]bool{}
	out := make([]string, 0, len(tags))
	for _, tag := range tags {
		tag = trimRunes(strings.TrimSpace(tag), 32)
		if tag == "" || seen[tag] {
			continue
		}
		seen[tag] = true
		out = append(out, tag)
		if len(out) >= 12 {
			break
		}
	}
	return out
}

func decodeTags(raw string) []string {
	raw = strings.TrimSpace(raw)
	if raw == "" {
		return nil
	}
	var tags []string
	if err := json.Unmarshal([]byte(raw), &tags); err != nil {
		return nil
	}
	return normalizeTags(tags)
}

func trimRunes(s string, max int) string {
	if max <= 0 {
		return s
	}
	runes := []rune(s)
	if len(runes) <= max {
		return s
	}
	return string(runes[:max])
}

func boolInt(v bool) int {
	if v {
		return 1
	}
	return 0
}

func stringValue(v any) string {
	switch x := v.(type) {
	case string:
		return x
	case fmt.Stringer:
		return x.String()
	default:
		if x == nil {
			return ""
		}
		return fmt.Sprint(x)
	}
}
