package sessionstore

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	"log/slog"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"many-ai-cli/internal/proto"
	"many-ai-cli/internal/sessionlog"

	_ "modernc.org/sqlite"
)

const defaultTimeout = 3 * time.Second

// resetTimeout は ResetHistory（UI の「履歴を全削除」）専用のタイムアウト。
// 利用者が明示的に押す保守操作なので、肥大化した DB でも完走できるよう長めに取る。
const resetTimeout = 60 * time.Second

// asyncQueueSize は StoreEventAsync のバッファ上限。健全な DB なら書き込みは
// ミリ秒単位で掃けるため溢れない。DB が劣化して書き込みが滞った場合は
// 溢れたイベントを黙って捨てる（.jsonl 側には全量残る）。
const asyncQueueSize = 4096

// プルーニングのバッチサイズと 1 回の実行で使う時間予算。
// 肥大化した DB では 1 文の巨大 DELETE（CASCADE 含む）が defaultTimeout に
// 収まらず永遠に掃除できなくなるため、子テーブルをチャンク削除してから
// セッション行を消す。予算切れなら中断し、次回の定期実行で続きから進む。
var (
	pruneSessionBatch  = 50
	pruneChildRowBatch = 2000
)

const pruneTimeBudget = 60 * time.Second

// errPruneBudgetExhausted は時間予算切れによる正常中断（エラーではない）。
var errPruneBudgetExhausted = errors.New("prune budget exhausted")

type Store struct {
	db         *sql.DB
	path       string
	ftsEnabled bool

	// 非同期イベント書き込み。PTY ホットパス（hub の pty_data 処理）から
	// SQLite の遅延を切り離すため、StoreEventAsync はキューに積むだけで返る。
	asyncCh      chan asyncEvent
	asyncQuit    chan struct{}
	asyncDone    chan struct{}
	asyncDropped atomic.Int64
	closeOnce    sync.Once
	// onWriteError は非同期書き込みの失敗通知（writer goroutine と競合するため atomic）。
	onWriteError atomic.Pointer[func(liveSessionID int, err error)]
}

type asyncEvent struct {
	liveSessionID int
	event         map[string]any
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
	applyPendingFileReset(path)
	db, err := sql.Open("sqlite", path)
	if err != nil {
		return nil, fmt.Errorf("open sqlite session store: %w", err)
	}
	db.SetMaxOpenConns(1)
	db.SetMaxIdleConns(1)
	s := &Store{db: db, path: path}
	if err := s.init(); err != nil {
		_ = db.Close()
		return nil, err
	}
	s.asyncCh = make(chan asyncEvent, asyncQueueSize)
	s.asyncQuit = make(chan struct{})
	s.asyncDone = make(chan struct{})
	go s.asyncWriter()
	return s, nil
}

func (s *Store) Close() error {
	if s == nil || s.db == nil {
		return nil
	}
	// writer goroutine を止めてから DB を閉じる。掃けないほど詰まっている場合は
	// 待ちすぎない（残イベントは捨てる。.jsonl 側には全量残っている）。
	s.closeOnce.Do(func() {
		if s.asyncQuit != nil {
			close(s.asyncQuit)
			select {
			case <-s.asyncDone:
			case <-time.After(2 * defaultTimeout):
			}
		}
	})
	return s.db.Close()
}

// resetPendingSuffix は「次回起動時に DB ファイルを作り直す」予約マーカーの拡張子。
// SQL でのリセットが完了できないほど DB が肥大化・劣化している場合の最終保証。
const resetPendingSuffix = ".reset-pending"

// applyPendingFileReset は予約マーカーがあれば DB を開く前にファイルごと削除する。
// 実行中セッションの行は失われるが、wrapper の reattach 時に StartSession の
// upsert で新 DB へ再登録されるため実害はない。DB ファイルを他プロセスが
// 掴んでいて消せない場合はマーカーを残し、次回起動で再試行する。
func applyPendingFileReset(path string) {
	marker := path + resetPendingSuffix
	if _, err := os.Stat(marker); err != nil {
		return
	}
	if err := os.Remove(path); err != nil && !os.IsNotExist(err) {
		return
	}
	_ = os.Remove(path + "-wal")
	_ = os.Remove(path + "-shm")
	_ = os.Remove(marker)
}

// ScheduleFileReset は次回の OpenForLogDir で DB ファイルを作り直す予約を入れる。
func (s *Store) ScheduleFileReset() error {
	if s == nil || s.path == "" {
		return nil
	}
	return os.WriteFile(s.path+resetPendingSuffix, []byte(time.Now().Format(time.RFC3339)+"\n"), sessionlog.PrivateFileMode)
}

// FileResetPending はファイル再作成の予約マーカーが存在するかを返す。
func (s *Store) FileResetPending() bool {
	if s == nil || s.path == "" {
		return false
	}
	_, err := os.Stat(s.path + resetPendingSuffix)
	return err == nil
}

// SetOnWriteError は非同期書き込み（StoreEventAsync 経由）の失敗時に呼ばれる
// コールバックを設定する。
func (s *Store) SetOnWriteError(fn func(liveSessionID int, err error)) {
	if s == nil {
		return
	}
	s.onWriteError.Store(&fn)
}

// StoreEventAsync はイベントを書き込みキューへ積んで即座に返る。
// PTY 配信のホットパスから呼ばれるため、SQLite の遅延・障害がここへ
// 波及しないことを最優先とし、キューが満杯ならイベントを破棄する。
// 戻り値は累計破棄数（0 = キューイング成功）。
func (s *Store) StoreEventAsync(liveSessionID int, event map[string]any) int64 {
	if s == nil || s.db == nil || s.asyncCh == nil {
		return 0
	}
	select {
	case s.asyncCh <- asyncEvent{liveSessionID: liveSessionID, event: event}:
		return 0
	default:
		return s.asyncDropped.Add(1)
	}
}

func (s *Store) asyncWriter() {
	defer close(s.asyncDone)
	for {
		select {
		case <-s.asyncQuit:
			return
		case ev := <-s.asyncCh:
			if err := s.StoreEvent(ev.liveSessionID, ev.event); err != nil {
				if fn := s.onWriteError.Load(); fn != nil && *fn != nil {
					(*fn)(ev.liveSessionID, err)
				}
			}
		}
	}
}

func (s *Store) init() error {
	ctx, cancel := context.WithTimeout(context.Background(), defaultTimeout)
	defer cancel()
	pragmas := []string{
		// 新規 DB はここで incremental に確定する。既存 DB（auto_vacuum=NONE）には
		// 即時効果は無いが、設定は pending になり次回 VACUUM（ResetHistory 実行時）で
		// 反映される。incremental になると incremental_vacuum で空きページを OS へ返せる。
		`PRAGMA auto_vacuum=INCREMENTAL`,
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
	// 各 DELETE のエラーを拾い、いずれか失敗したら Commit せず return（defer Rollback に委ねる）。
	// 途中失敗を無視して Commit すると messages_fts と messages が部分削除のまま不整合になり、
	// 全文検索が削除済みメッセージにヒットし得る。
	if s.ftsEnabled {
		if _, err := tx.ExecContext(ctx, `DELETE FROM messages_fts WHERE rowid IN (SELECT id FROM messages WHERE session_id=?)`, id); err != nil {
			return
		}
	}
	if _, err := tx.ExecContext(ctx, `DELETE FROM messages WHERE session_id=?`, id); err != nil {
		return
	}
	if _, err := tx.ExecContext(ctx, `DELETE FROM events WHERE session_id=?`, id); err != nil {
		return
	}
	if _, err := tx.ExecContext(ctx, `DELETE FROM approvals WHERE session_id=?`, id); err != nil {
		return
	}
	// attachments も同 Tx 内で削除する。sessions 行は残す（per-session 履歴クリアのため）ので
	// attachments.session_id の ON DELETE CASCADE は発火せず、ここで明示削除しないと
	// 孤児 attachments 行（添付の path/filename/mime/size）が残留する。
	// pruneSessionRow / resetHistorySQL と削除対象を揃える。
	if _, err := tx.ExecContext(ctx, `DELETE FROM attachments WHERE session_id=?`, id); err != nil {
		return
	}
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
	payload, err := json.Marshal(slimEventPayload(event))
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

// MessagesMentionText は指定 live セッションの保存済みメッセージに
// variants のいずれかが部分一致で含まれるかを返す。
// Files プレビューの「チャットで言及されたパスは読み取り専用で開ける」判定に使う。
//
// 照合対象は role='user'（人間の入力）に限定する。pty_output（AI CLI の端末出力）は
// role='ai' で保存されるが、これを照合対象に含めると、AI 出力やプロンプトインジェクション
// 経由で許可ルート外の絶対パス文字列を 1 度出力させるだけで当該ファイルを読めてしまう
// （read-only バイパスの悪用経路）。正規 UX（ユーザーがチャットで言及したファイルを開く）は
// role='user' のみで維持される。
func (s *Store) MessagesMentionText(liveSessionID int, variants []string) (bool, error) {
	sessionID, err := s.sessionIDForLive(liveSessionID)
	if err != nil || sessionID == 0 {
		return false, err
	}
	ctx, cancel := context.WithTimeout(context.Background(), defaultTimeout)
	defer cancel()
	for _, v := range variants {
		if v == "" {
			continue
		}
		var one int
		err := s.db.QueryRowContext(ctx, `SELECT 1 FROM messages
			WHERE session_id=? AND role='user' AND (instr(COALESCE(raw_text, ''), ?) > 0 OR instr(COALESCE(text, ''), ?) > 0)
			LIMIT 1`, sessionID, v, v).Scan(&one)
		if err == sql.ErrNoRows {
			continue
		}
		if err != nil {
			return false, err
		}
		return true, nil
	}
	return false, nil
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
	deadline := time.Now().Add(pruneTimeBudget)
	for {
		ids, err := s.expiredSessionIDs(cutoff, pruneSessionBatch)
		if err != nil {
			return err
		}
		if len(ids) == 0 {
			break
		}
		for _, id := range ids {
			if err := s.pruneSessionRow(id, deadline); err != nil {
				if errors.Is(err, errPruneBudgetExhausted) {
					return nil
				}
				return err
			}
		}
	}
	// 空きページを OS へ返却（auto_vacuum=INCREMENTAL の DB のみ効く。NONE では no-op）。
	// 1 回あたり最大 20000 ページ（4KB ページで約 80MB）に制限して長時間ロックを避ける。
	// 続きは次回の定期実行で進む。その後、溜まった WAL を切り詰める（いずれも失敗無害）。
	ctx, cancel := context.WithTimeout(context.Background(), defaultTimeout)
	defer cancel()
	_, _ = s.db.ExecContext(ctx, `PRAGMA incremental_vacuum(20000)`)
	_, _ = s.db.ExecContext(ctx, `PRAGMA wal_checkpoint(TRUNCATE)`)
	return nil
}

func (s *Store) expiredSessionIDs(cutoff time.Time, limit int) ([]int64, error) {
	ctx, cancel := context.WithTimeout(context.Background(), defaultTimeout)
	defer cancel()
	rows, err := s.db.QueryContext(ctx, `SELECT id FROM sessions
		WHERE ended_at IS NOT NULL AND ended_at < ? ORDER BY id LIMIT ?`,
		cutoff.Format(time.RFC3339), limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var ids []int64
	for rows.Next() {
		var id int64
		if err := rows.Scan(&id); err != nil {
			return nil, err
		}
		ids = append(ids, id)
	}
	return ids, rows.Err()
}

// pruneSessionRow は子テーブルをチャンク削除してからセッション行を消す。
// 子行を先に消しておくことで、最後の sessions DELETE の CASCADE が
// 空振り（インデックス参照のみ）になり、巨大セッションでもタイムアウトしない。
func (s *Store) pruneSessionRow(id int64, deadline time.Time) error {
	for _, table := range []string{"events", "messages", "approvals", "attachments"} {
		if err := s.deleteChildRowsChunked(table, id, deadline); err != nil {
			return err
		}
	}
	if time.Now().After(deadline) {
		return errPruneBudgetExhausted
	}
	ctx, cancel := context.WithTimeout(context.Background(), defaultTimeout)
	defer cancel()
	_, err := s.db.ExecContext(ctx, `DELETE FROM sessions WHERE id=?`, id)
	return err
}

func (s *Store) deleteChildRowsChunked(table string, sessionID int64, deadline time.Time) error {
	for {
		if time.Now().After(deadline) {
			return errPruneBudgetExhausted
		}
		ctx, cancel := context.WithTimeout(context.Background(), defaultTimeout)
		// #nosec G202 -- table は呼び元の固定リストのみ
		rows, err := s.db.QueryContext(ctx, `SELECT id FROM `+table+` WHERE session_id=? LIMIT ?`, sessionID, pruneChildRowBatch)
		if err != nil {
			cancel()
			return err
		}
		var ids []int64
		for rows.Next() {
			var id int64
			if err := rows.Scan(&id); err != nil {
				rows.Close()
				cancel()
				return err
			}
			ids = append(ids, id)
		}
		err = rows.Err()
		rows.Close()
		cancel()
		if err != nil {
			return err
		}
		if len(ids) == 0 {
			return nil
		}
		placeholders := make([]string, len(ids))
		args := make([]any, len(ids))
		for i, id := range ids {
			placeholders[i] = "?"
			args[i] = id
		}
		inClause := strings.Join(placeholders, ",")
		ctx2, cancel2 := context.WithTimeout(context.Background(), defaultTimeout)
		if table == "messages" && s.ftsEnabled {
			// 外部 content の FTS は本体行の削除に追従しないため、同じ rowid を先に消す
			_, _ = s.db.ExecContext(ctx2, `DELETE FROM messages_fts WHERE rowid IN (`+inClause+`)`, args...)
		}
		// #nosec G202 -- table は固定リスト、inClause は "?" の連結のみ
		_, err = s.db.ExecContext(ctx2, `DELETE FROM `+table+` WHERE id IN (`+inClause+`)`, args...)
		cancel2()
		if err != nil {
			return err
		}
		if len(ids) < pruneChildRowBatch {
			return nil
		}
	}
}

// ResetHistory は保存済みセッション履歴を全削除する（実行中セッションの行は保護）。
// SQL での削除または VACUUM が完走できない場合（DB の肥大化・劣化）は、
// 次回 Hub 起動時のファイル再作成を予約する。「UI でリセットしたのに
// ファイルが残り続ける」状態を作らないための最終保証。
func (s *Store) ResetHistory(preserveLiveIDs []int) (ResetResult, error) {
	if s == nil || s.db == nil {
		return ResetResult{}, nil
	}
	out, err := s.resetHistorySQL(preserveLiveIDs)
	if err != nil {
		_ = s.ScheduleFileReset()
		return out, err
	}
	if !s.vacuumAfterReset() {
		// 行削除は成功したが物理縮小に失敗。再作成しても消えるのは
		// リセット後に書かれた直近データのみ（実行中セッションは reattach で再登録）。
		_ = s.ScheduleFileReset()
	}
	return out, nil
}

func (s *Store) resetHistorySQL(preserveLiveIDs []int) (ResetResult, error) {
	var out ResetResult
	if s == nil || s.db == nil {
		return out, nil
	}
	// 利用者が明示的に押す全削除なので、肥大化した DB でも完走できるよう
	// defaultTimeout ではなく resetTimeout を使う。
	ctx, cancel := context.WithTimeout(context.Background(), resetTimeout)
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
		// 子テーブルを先に空にしておくと、sessions の DELETE で CASCADE が
		// 空振りになり、行数が多くても完走しやすい。
		for _, table := range []string{"events", "messages", "approvals", "attachments"} {
			if _, err := tx.ExecContext(ctx, `DELETE FROM `+table); err != nil { // #nosec G202 -- table は固定リスト
				return out, err
			}
		}
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

// vacuumAfterReset は全削除後にファイルを物理的に縮める。
// VACUUM はトランザクション外でしか実行できないため commit 後に呼ぶ。
// 行をほぼ消した直後なので通常は軽い。auto_vacuum=NONE の旧 DB はこの VACUUM で
// init() の pending 設定（INCREMENTAL）が反映される副次効果もある。
// 戻り値 false は VACUUM 未完走（呼び元がファイル再作成の予約で補償する）。
func (s *Store) vacuumAfterReset() bool {
	ctx, cancel := context.WithTimeout(context.Background(), resetTimeout)
	defer cancel()
	if _, err := s.db.ExecContext(ctx, `VACUUM`); err != nil {
		return false
	}
	_, _ = s.db.ExecContext(ctx, `PRAGMA wal_checkpoint(TRUNCATE)`)
	return true
}

// eventPayloadTextLimit は events.payload_json に残す text の上限（rune 数）。
// UI のタイムラインは payload の先頭 180 文字しか表示せず、全文は .jsonl / messages
// 側にあるため、events には参照用の冒頭だけ残せば足りる。
const eventPayloadTextLimit = 2000

// slimEventPayload は events.payload_json 用に PTY 出力イベントを間引く。
// pty_output は出力量が膨大（TUI の再描画を全チャンク含む）な一方、
// data_b64 は .log/.jsonl に全量残る複製のため SQLite には保存しない。
// chat 履歴（messages テーブル）は元の event map から作るので影響しない。
func slimEventPayload(event map[string]any) map[string]any {
	if stringValue(event["type"]) != "pty_output" {
		return event
	}
	slim := make(map[string]any, len(event))
	for k, v := range event {
		if k == "data_b64" {
			continue
		}
		slim[k] = v
	}
	if text, ok := slim["text"].(string); ok {
		slim["text"] = trimRunes(text, eventPayloadTextLimit)
	}
	return slim
}

func (s *Store) storeMessageForEvent(ctx context.Context, tx *sql.Tx, sessionID int64, ts, typ string, event map[string]any, payload []byte) error {
	switch typ {
	case "user_input":
		text := strings.TrimRight(sessionlog.MaskSecrets(stringValue(event["text"])), "\r\n")
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
		// LastInsertId 失敗は ID 取得失敗のみで messages 行は INSERT 済。
		// FTS 行が作れないことを記録して continue（messages の保存は維持）。
		slog.Warn("sessionstore: LastInsertId failed, FTS index entry skipped",
			slog.String("err", err.Error()),
			slog.Int64("session_id", sessionID))
		return nil
	}
	if _, ftsErr := tx.ExecContext(ctx, `INSERT INTO messages_fts(rowid, text, raw_text) VALUES (?, ?, ?)`, id, text, rawText); ftsErr != nil {
		// FTS への INSERT 失敗は messages 本体には影響しないため tx は commit させ
		// 検索インデックス未登録（沈黙的可視性低下）の事実だけ slog に残す。
		// transaction を中断すると以降のメッセージ保存自体が止まり情報損失が拡大する
		// ため、ここでは soft fail に倒す。長期運用で発生頻度が高い場合は
		// onWriteError コールバック経由で UI へ通知する設計に拡張すること。
		slog.Warn("sessionstore: messages_fts insert failed (message saved but not indexed)",
			slog.String("err", ftsErr.Error()),
			slog.Int64("session_id", sessionID),
			slog.Int64("message_rowid", id))
	}
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
	// LIKE のワイルドカード（% と _）と ESCAPE 文字（\）をすべてエスケープし、
	// クエリ語をリテラル部分一致として扱う。\ を先に置換しないと、後段で挿入した
	// エスケープ用 \ が二重エスケープされて意味が崩れるため順序が重要。
	// 例: `go_test` の `_` を未処理にすると `goXtest`（X は任意1文字）にも過剰一致する。
	likeEscaper := strings.NewReplacer(`\`, `\\`, `%`, `\%`, `_`, `\_`)
	like := "%" + likeEscaper.Replace(query) + "%"
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
